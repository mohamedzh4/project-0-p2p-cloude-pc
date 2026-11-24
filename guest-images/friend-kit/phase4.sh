#!/usr/bin/env bash
# Phase 4 on a borrowed Linux PC, in one command:
#
#   sudo bash /path/to/DCL-PHASE4/phase4.sh
#
# 1. Checks the PC (x86_64, KVM, RAM, free space) and notes what is installed.
# 2. Installs the build tools (QEMU, xorriso; Packer as a private binary).
# 3. Builds the Standard guest disk and signs it with the key ON THE USB KIT.
#    The key is read from the kit and never copied to this PC's disk.
# 4. Compresses the disk, checks it, and saves it to DCL-PHASE4/output/.
# 5. Puts the PC back: deletes every file it made and removes exactly the
#    packages it added. Also runs when the build fails or Ctrl-C is pressed.
#
# Ubuntu/Debian (apt) and Arch (pacman). Must run as root (sudo), so nothing
# changes the owner's groups or home folder.
#
#   --keep           keep the build folder and packages (debugging)
#   --cleanup-only   only undo a previous run (after a crash or power cut)
set -Eeuo pipefail
export LC_ALL=C  # same sort order for every package list

KIT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARBALL="$KIT/dcl-guest-images.tgz"
KEY="$KIT/key/dcl-dev-ed25519.pem"
OUT="$KIT/output"
STATE="$KIT/state"
LOG="$KIT/phase4.log"
WORK="${DCL_PHASE4_WORK:-/var/tmp/dcl-phase4}"
MARKER=".dcl-phase4-workdir"
PACKER_VERSION=1.11.2
NEED_WORK_GB=40
NEED_KIT_GB=8
# A "6 GB" PC reports about 5.7-5.9 GB to Linux.
NEED_RAM_MB=5500
# Tests only: skip Packer and sign a tiny fake disk instead.
FAKE_BUILD="${DCL_PHASE4_FAKE_BUILD:-0}"

usage() {
  sed -n '2,21p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

MODE=run
case "${1:-}" in
  "") ;;
  --keep) MODE=keep ;;
  --cleanup-only) MODE=cleanup ;;
  -h | --help) usage; exit 0 ;;
  *) usage >&2; exit 2 ;;
esac

step() { printf '\n==> %s\n' "$*"; }
die() { printf '\nERROR: %s\n' "$*" >&2; exit 1; }

[[ $EUID -eq 0 ]] || die "run it with sudo:  sudo bash $KIT/phase4.sh"

mkdir -p "$STATE"
exec > >(tee -a "$LOG") 2>&1
echo "---- phase4.sh $MODE  $(date -u '+%Y-%m-%dT%H:%M:%SZ')"

# ---------------------------------------------------------------- distro

detect_distro() {
  if [[ -f "$STATE/distro" ]]; then
    cat "$STATE/distro"
  elif command -v pacman >/dev/null 2>&1; then
    echo arch
  elif command -v apt-get >/dev/null 2>&1 && command -v dpkg-query >/dev/null 2>&1; then
    echo debian
  else
    die "only Ubuntu/Debian (apt) and Arch (pacman) are supported"
  fi
}
DISTRO="$(detect_distro)"

installed_packages() {
  if [[ $DISTRO == arch ]]; then
    pacman -Qq | sort
  else
    dpkg-query -W -f='${db:Status-Abbrev} ${binary:Package}\n' |
      awk '$1 ~ /^ii/ {print $2}' | sort
  fi
}

package_cache_dir() {
  if [[ $DISTRO == arch ]]; then echo /var/cache/pacman/pkg; else echo /var/cache/apt/archives; fi
}

cached_files() {
  find "$(package_cache_dir)" -maxdepth 1 -type f 2>/dev/null | sort || true
}

free_gb() { df -P -BG "$1" | awk 'NR==2 {gsub("G","",$4); print $4}'; }

# ---------------------------------------------------------------- cleanup

work_is_ours() {
  [[ -n "$WORK" && "$WORK" != "/" && -f "$WORK/$MARKER" ]]
}

cleanup() {
  step "Putting the PC back the way it was"
  # A Packer VM that is still running holds files in the build folder.
  pkill -f -- "$WORK/" 2>/dev/null || true
  sleep 1

  if work_is_ours; then
    rm -rf -- "$WORK"
    echo "deleted $WORK"
  elif [[ -e "$WORK" ]]; then
    echo "left $WORK alone: it was not made by this script"
  fi

  if [[ -f "$STATE/packages.before" ]]; then
    installed_packages >"$STATE/packages.after"
    comm -13 "$STATE/packages.before" "$STATE/packages.after" >"$STATE/packages.added"
    if [[ -s "$STATE/packages.added" ]]; then
      echo "removing the packages this script added:"
      tr '\n' ' ' <"$STATE/packages.added"; echo
      if [[ $DISTRO == arch ]]; then
        # -Rn without -s: exactly these packages, nothing that was there before.
        xargs -a "$STATE/packages.added" pacman -Rn --noconfirm
      else
        DEBIAN_FRONTEND=noninteractive xargs -a "$STATE/packages.added" \
          apt-get -o DPkg::Lock::Timeout=300 purge -y
      fi
    else
      echo "no packages were added"
    fi
    if [[ -f "$STATE/cache.before" ]]; then
      cached_files >"$STATE/cache.after"
      comm -13 "$STATE/cache.before" "$STATE/cache.after" | while IFS= read -r file; do
        rm -f -- "$file"
      done
    fi
    installed_packages >"$STATE/packages.final"
    if [[ "$(cat "$STATE/packages.before")" != "$(cat "$STATE/packages.final")" ]]; then
      echo "WARNING: the package list differs from before (left: missing now, right: extra now):"
      comm -3 "$STATE/packages.before" "$STATE/packages.final" || true
      echo "state kept in $STATE so this can be checked"
      return 1
    fi
  fi

  if [[ -f "$STATE/disk.before" ]]; then
    echo "free space before: $(cat "$STATE/disk.before") GB   now: $(free_gb /) GB"
  fi
  rm -rf -- "$STATE"
  echo "the PC is back as it was"
}

if [[ $MODE == cleanup ]]; then
  [[ -f "$STATE/packages.before" ]] || echo "no saved state: only the build folder is checked"
  cleanup
  exit 0
fi

# ---------------------------------------------------------------- checks

step "Checking this PC and the USB kit"
[[ "$(uname -m)" == x86_64 ]] || die "this PC is $(uname -m); an x86_64 (Intel/AMD) PC is needed"
[[ -f "$TARBALL" || -f "$KIT/guest-images/scripts/build.sh" ]] ||
  die "put the repo's guest-images folder (or dcl-guest-images.tgz) inside $KIT"
[[ -f "$KEY" ]] || die "missing the signing key $KEY"
grep -q 'BEGIN PRIVATE KEY' "$KEY" || die "$KEY is not a PEM private key"

if [[ $FAKE_BUILD != 1 ]]; then
  [[ -e /dev/kvm ]] || die "/dev/kvm is missing: turn on Intel VT-x / AMD SVM in the BIOS, then reboot"
fi

# RAM for the build VM: what is left must still run the owner's desktop.
build_vm_mb() {
  local ram=$1
  if ((ram >= 15000)); then echo 8192
  elif ((ram >= 7500)); then echo 4096
  elif ((ram >= NEED_RAM_MB)); then echo 3072
  else echo 0
  fi
}

ram_mb=${DCL_PHASE4_RAM_MB:-$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)}
vm_mb=$(build_vm_mb "$ram_mb")
if [[ $vm_mb == 0 && $FAKE_BUILD != 1 ]]; then
  die "this PC has ${ram_mb} MB RAM; 6 GB is the minimum"
fi
[[ $vm_mb == 0 ]] && vm_mb=3072
if ((vm_mb == 3072)); then
  echo "Low RAM (${ram_mb} MB): the build VM gets 3 GB. Close the browser and other apps now."
fi
cores=$(nproc)
vm_cpus=$(( cores >= 6 ? 4 : (cores >= 3 ? cores - 1 : 1) ))

mkdir -p "$(dirname "$WORK")"
work_free=$(free_gb "$(dirname "$WORK")")
[[ $FAKE_BUILD == 1 || $work_free -ge $NEED_WORK_GB ]] ||
  die "only ${work_free} GB free under $(dirname "$WORK"); ${NEED_WORK_GB} GB is needed during the build"
kit_free=$(free_gb "$KIT")
[[ $FAKE_BUILD == 1 || $kit_free -ge $NEED_KIT_GB ]] ||
  die "only ${kit_free} GB free on the USB kit; ${NEED_KIT_GB} GB is needed for the disk"

kit_fs=$(findmnt -no FSTYPE -T "$KIT" 2>/dev/null || echo unknown)
if [[ $kit_fs == vfat || $kit_fs == msdos ]] && [[ ${DCL_PHASE4_ALLOW_FAT:-0} != 1 ]]; then
  die "the USB kit is FAT32, which cannot hold files over 4 GB. Reformat it as exFAT and make the kit again."
fi
echo "distro=$DISTRO ram=${ram_mb}MB cores=$cores build-vm=${vm_mb}MB/${vm_cpus}cpu free=${work_free}GB kit=${kit_free}GB ($kit_fs)"

if work_is_ours; then
  echo "a previous build folder is still here; removing it first"
  rm -rf -- "$WORK"
elif [[ -e "$WORK" ]]; then
  die "$WORK exists and was not made by this script; move it away first"
fi

# ---------------------------------------------------------------- snapshot

step "Noting what is on the PC now"
if [[ -f "$STATE/packages.before" ]]; then
  # A previous run never finished cleaning up: its snapshot is the true "before".
  echo "keeping the snapshot from the unfinished run of $(date -r "$STATE/packages.before")"
else
  echo "$DISTRO" >"$STATE/distro.tmp"
  installed_packages >"$STATE/packages.before.tmp"
  cached_files >"$STATE/cache.before"
  free_gb / >"$STATE/disk.before"
  mv "$STATE/distro.tmp" "$STATE/distro"
  mv "$STATE/packages.before.tmp" "$STATE/packages.before"
fi
echo "$(wc -l <"$STATE/packages.before") packages installed, $(cat "$STATE/disk.before") GB free"

finish() {
  local status=$?
  trap - EXIT INT TERM
  if [[ $MODE == keep ]]; then
    echo "--keep: build folder $WORK and packages left in place."
    echo "Undo later with: sudo bash $KIT/phase4.sh --cleanup-only"
  else
    cleanup || status=1
  fi
  if [[ $status -eq 0 ]]; then
    printf '\nPHASE 4 DONE. The disk and env.snippet are in %s\n' "$OUT"
  else
    printf '\nPHASE 4 FAILED (see %s). Send the last 40 lines of it.\n' "$LOG"
  fi
  exit "$status"
}
trap finish EXIT
trap 'exit 130' INT TERM

# ---------------------------------------------------------------- install

step "Installing the build tools"
# Only what is missing. A package already on the PC must never be upgraded:
# an upgrade cannot be undone cleanly, so the dry run stops before anything.
if [[ $DISTRO == arch ]]; then
  wanted=(qemu-base libisoburn openssl python curl openssh)
else
  wanted=(qemu-system-x86 qemu-utils xorriso openssl python3 curl ca-certificates openssh-client)
fi
missing=()
for p in "${wanted[@]}"; do
  grep -qx "$p" "$STATE/packages.before" || grep -qx "$p:amd64" "$STATE/packages.before" ||
    missing+=("$p")
done

refuse_upgrades() {
  local upgrades="$1"
  [[ -z "$upgrades" || ${DCL_PHASE4_ALLOW_UPGRADES:-0} == 1 ]] && return 0
  echo "Installing the tools would UPGRADE packages this PC already has:"
  echo "$upgrades" | sed 's/^/  /'
  echo "An upgrade cannot be undone, so nothing was installed."
  if [[ $DISTRO == arch ]]; then
    echo "The PC's system is out of date. Ask the owner to update it (pacman -Syu), then run this again."
  else
    echo "The PC's system is out of date. Ask the owner to install updates, then run this again."
  fi
  exit 1
}

if ((${#missing[@]} == 0)); then
  echo "all build tools are already installed"
elif [[ $DISTRO == arch ]]; then
  echo "missing: ${missing[*]}"
  targets=$(pacman -S --needed --print --print-format '%n' "${missing[@]}") ||
    die "pacman could not find the tools. If it says 'not found', the PC needs 'pacman -Syu' first (ask the owner)."
  refuse_upgrades "$(grep -xF -f <(printf '%s\n' "$targets") "$STATE/packages.before" || true)"
  pacman -S --needed --noconfirm "${missing[@]}"
else
  echo "missing: ${missing[*]}"
  apt-get -o DPkg::Lock::Timeout=300 update
  refuse_upgrades "$(apt-get -s install --no-upgrade "${missing[@]}" | awk '/^Inst [^ ]+ \[/ {print $2}')"
  DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=300 install -y --no-upgrade \
    "${missing[@]}"
fi

mkdir -p "$WORK"
touch "$WORK/$MARKER"
mkdir -p "$WORK/bin" "$WORK/tmp" "$WORK/home" "$WORK/src"

echo "downloading Packer $PACKER_VERSION (kept inside $WORK only)"
base="https://releases.hashicorp.com/packer/$PACKER_VERSION"
zip="packer_${PACKER_VERSION}_linux_amd64.zip"
curl -fsSL --retry 3 -o "$WORK/tmp/$zip" "$base/$zip"
curl -fsSL --retry 3 -o "$WORK/tmp/SHA256SUMS" "$base/packer_${PACKER_VERSION}_SHA256SUMS"
(cd "$WORK/tmp" && grep " $zip\$" SHA256SUMS | sha256sum -c -) || die "the Packer download is corrupt"
python3 -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extract("packer", sys.argv[2])' \
  "$WORK/tmp/$zip" "$WORK/bin"
chmod 755 "$WORK/bin/packer"
"$WORK/bin/packer" version

# ---------------------------------------------------------------- build

step "Building and signing the guest disk (30-60 minutes)"
if [[ -f "$KIT/guest-images/scripts/build.sh" ]]; then
  # A folder copied by hand: take it without old build output or macOS ._ files.
  tar -C "$KIT" --exclude 'guest-images/output' --exclude 'guest-images/.packer-ssh' \
    --exclude 'guest-images/cache' --exclude '__pycache__' --exclude '._*' \
    -cf - guest-images | tar -C "$WORK/src" -xf -
else
  tar xzf "$TARBALL" -C "$WORK/src"
fi
SRC="$WORK/src/guest-images"
[[ -x "$SRC/scripts/build.sh" || -f "$SRC/scripts/build.sh" ]] || die "the kit's archive has no guest-images/scripts/build.sh"
D="$SRC/output/standard"

# Everything Packer, Python and openssl write goes under $WORK.
build_env=(
  "PATH=$WORK/bin:$PATH"
  "HOME=$WORK/home"
  "TMPDIR=$WORK/tmp"
  "PACKER_CONFIG_DIR=$WORK/home/.packer.d"
  "PACKER_PLUGIN_PATH=$WORK/home/.packer.d/plugins"
  "PACKER_CACHE_DIR=$WORK/packer-cache"
  "CHECKPOINT_DISABLE=1"
  "PYTHONDONTWRITEBYTECODE=1"
  "PACKER_ACCELERATOR=kvm"
  "PACKER_MEMORY_MB=$vm_mb"
  "PACKER_CPUS=$vm_cpus"
  "IMAGE_SIGNING_PRIVATE_KEY=$KEY"
)

if [[ $FAKE_BUILD == 1 ]]; then
  echo "TEST MODE: signing a small fake disk instead of running Packer"
  mkdir -p "$D"
  qemu-img create -q -f qcow2 "$D/rootfs.qcow2" 64M
  qemu-img convert -O raw "$D/rootfs.qcow2" "$D/rootfs.img"
  (cd "$SRC" && env "${build_env[@]}" python3 scripts/sign_image.py sign \
    --image "$D/rootfs.img" --key "$KEY" --lab-tier standard)
else
  (cd "$SRC" && env "${build_env[@]}" bash scripts/build.sh standard)
fi

for f in rootfs.qcow2 rootfs.img rootfs.img.sha256 rootfs.img.sig rootfs.img.manifest.json env.snippet; do
  [[ -s "$D/$f" ]] || die "the build did not produce $f"
done
digest=$(awk '{print $1; exit}' "$D/rootfs.img.sha256")

# ---------------------------------------------------------------- save

step "Compressing and checking the disk"
qemu-img convert -p -c -O qcow2 "$D/rootfs.qcow2" "$WORK/rootfs.qcow2"
rm -f "$D/rootfs.img" "$D/rootfs.qcow2"
qemu-img convert -O raw "$WORK/rootfs.qcow2" "$WORK/check.img"
check=$(sha256sum "$WORK/check.img" | awk '{print $1}')
rm -f "$WORK/check.img"
[[ $check == "$digest" ]] || die "the compressed disk does not match the signed digest"
echo "compressed disk matches the signed digest $digest"

step "Saving to the USB kit"
size_gb=$(( ($(stat -c %s "$WORK/rootfs.qcow2") + 1073741823) / 1073741824 ))
kit_free=$(free_gb "$KIT")
[[ $kit_free -gt $size_gb ]] || die "the disk is ${size_gb} GB but the kit has ${kit_free} GB free"

rm -rf "$OUT.new"
mkdir -p "$OUT.new"
cp "$WORK/rootfs.qcow2" "$OUT.new/rootfs.qcow2"
for f in rootfs.img.sha256 rootfs.img.sig rootfs.img.manifest.json env.snippet; do
  cp "$D/$f" "$OUT.new/$f"
done
sync
copy=$(sha256sum "$OUT.new/rootfs.qcow2" | awk '{print $1}')
orig=$(sha256sum "$WORK/rootfs.qcow2" | awk '{print $1}')
[[ $copy == "$orig" ]] || die "the copy on the USB kit is damaged; try another USB drive"
cat >"$OUT.new/README.txt" <<EOF
Built $(date -u '+%Y-%m-%d %H:%M UTC') on $DISTRO.

rootfs.qcow2              the guest disk (compressed). Hosts convert it to rootfs.img.
env.snippet               digest + signature + public key. Safe to share: no secret in it.
rootfs.img.sha256         SHA-256 of the raw disk (rootfs.img), which is what is signed.
rootfs.img.sig            Ed25519 signature over that digest.
rootfs.img.manifest.json  the same, as the Matchmaker serves it.
EOF
rm -rf "$OUT.old"
[[ -d "$OUT" ]] && mv "$OUT" "$OUT.old"
mv "$OUT.new" "$OUT"
rm -rf "$OUT.old"
sync
echo "saved to $OUT:"
ls -lh "$OUT"
echo
cat "$OUT/env.snippet"
