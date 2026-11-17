//! Windows Standard QEMU+WHPX invoke (phase 28).
//!
//! After the capability probe offers Standard, and the phase-24 Weilnetz QEMU
//! binary plus a verified phase-20/26 Standard disk are present, this module
//! starts `qemu-system-*.exe` with the flags qemu.org documents for WHPX:
//!
//! x86_64: `-accel whpx -M pc -smp cores=N -m SIZE -device ich9-usb-ehci1
//! -device usb-tablet -drive file=DISK,if=virtio,format=raw,snapshot=on`
//!
//! aarch64: `-accel whpx -M virt -cpu host -smp cores=N -m SIZE
//! -bios edk2-aarch64-code.fd -device ramfb -device nec-usb-xhci
//! -device usb-kbd -device usb-tablet -drive file=DISK,if=virtio,format=raw,snapshot=on`
//!
//! Size is the clamped slider mapped to a QEMU `-m` suffix (`NG` when the
//! value is whole GiB, otherwise `NM`). Guest net is SLIRP `-netdev user`
//! (free, no TAP-Windows driver). TCG is not a Standard Windows product
//! fallback. Hyper-V is not a Standard boot path (Hyper-V is the
//! Pro GPU backend). Tests inject a fake process so CI never execs QEMU or
//! needs WHPX. Production spawn is refused under `cfg(test)` and off Windows.

use std::path::Path;
use std::sync::Mutex;

use thiserror::Error;

use crate::host_capabilities::{HostCapabilities, WhpxCapability};
use crate::launch::LaunchConfig;
use crate::provision::{HostOs, LabTier};

/// qemu.org `docs/system/whpx`: `-accel whpx` (not TCG, not HAX, not mshv).
pub const ACCEL_ARG: &str = "-accel";
pub const ACCEL_WHPX: &str = "whpx";
pub const MACHINE_ARG: &str = "-M";
pub const MACHINE_PC: &str = "pc";
pub const MACHINE_VIRT: &str = "virt";
pub const CPU_ARG: &str = "-cpu";
pub const CPU_HOST: &str = "host";
pub const SMP_ARG: &str = "-smp";
pub const MEMORY_ARG: &str = "-m";
pub const DEVICE_ARG: &str = "-device";
pub const USB_EHCI: &str = "ich9-usb-ehci1";
pub const USB_TABLET: &str = "usb-tablet";
/// Guest disk is virtio-blk, same as the Linux hosts. Emulated IDE (`-hda`)
/// measured ~62x slower on small random reads (`docs/issues.md` 9).
pub const DRIVE_ARG: &str = "-drive";
pub const BIOS_ARG: &str = "-bios";
/// Documented aarch64 WHPX firmware name (QEMU datadir).
pub const AARCH64_BIOS: &str = "edk2-aarch64-code.fd";
pub const RAMFB: &str = "ramfb";
pub const USB_XHCI: &str = "nec-usb-xhci";
pub const USB_KBD: &str = "usb-kbd";
pub const NETDEV_ARG: &str = "-netdev";
pub const NETDEV_USER: &str = "user,id=net0";
pub const VIRTIO_NET: &str = "virtio-net-pci,netdev=net0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QemuError {
    #[error("QEMU+WHPX is Windows-only; not starting a guest on {0}")]
    WindowsOnly(HostOs),
    #[error("QEMU+WHPX is the Standard Windows backend; not used for {0} sessions")]
    StandardOnly(&'static str),
    #[error("Standard sessions are not offered; not starting QEMU ({reason})")]
    StandardNotOffered { reason: String },
    #[error(
        "Windows Hypervisor Platform (WHPX) is not available; QEMU software emulation (TCG) is not the Standard Windows product. Enable HypervisorPlatform. Hyper-V is the Pro GPU backend, not a Standard boot fallback."
    )]
    WhpxUnavailable,
    #[error(
        "QEMU is not installed. Locate or install it first (Weilnetz NSIS into the user hypervisor cache)."
    )]
    BinaryMissing,
    #[error(
        "verified Standard guest image is not present; fetch and verify it before boot"
    )]
    DiskMissing,
    #[error("QEMU vCPU count {0} must be at least 1")]
    InvalidVcpu(u32),
    #[error("could not start QEMU ({program}): {detail}")]
    Spawn { program: String, detail: String },
    #[error("QEMU spawn is disabled in tests; inject a fake process")]
    RefusedInTests,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokeResult {
    pub hypervisor_invoked: bool,
    pub argv: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InvokeRequest<'a> {
    pub os: HostOs,
    pub lab_tier: LabTier,
    pub capabilities: &'a HostCapabilities,
    pub binary: &'a Path,
    pub disk: &'a Path,
    pub vcpu_count: u32,
    pub memory_mib: u64,
    pub arch: &'a str,
    pub bootstrap_origin: Option<&'a str>,
    pub bootstrap_token: Option<&'a str>,
}

pub trait QemuChild: Send {
    #[allow(dead_code)]
    fn kill(&mut self);
}

pub trait QemuSpawner {
    fn spawn(&self, binary: &Path, args: &[String]) -> Result<Box<dyn QemuChild>, QemuError>;
    fn detach(&self, child: Box<dyn QemuChild>);
}

pub trait QemuRuntime: Send + Sync {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, QemuError>;
}

/// Production runtime: spawn the cached Weilnetz `qemu-system-*.exe`.
pub struct ProcessQemuRuntime;

impl QemuRuntime for ProcessQemuRuntime {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, QemuError> {
        invoke(request, &ProcessSpawner)
    }
}

struct ProcessSpawner;

impl QemuSpawner for ProcessSpawner {
    fn spawn(&self, binary: &Path, args: &[String]) -> Result<Box<dyn QemuChild>, QemuError> {
        if cfg!(test) {
            return Err(QemuError::RefusedInTests);
        }
        if cfg!(not(target_os = "windows")) {
            return Err(QemuError::WindowsOnly(HostOs::current()));
        }
        let child = std::process::Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| QemuError::Spawn {
                program: binary.display().to_string(),
                detail: error.to_string(),
            })?;
        Ok(Box::new(ProcessChild(Some(child))))
    }

    fn detach(&self, child: Box<dyn QemuChild>) {
        RUNNING.lock().expect("qemu running mutex").push(child);
    }
}

/// Kill QEMU guests detached after a successful spawn.
pub fn kill_running() {
    let mut running = RUNNING.lock().expect("qemu running mutex");
    for child in running.drain(..) {
        let mut child = child;
        child.kill();
    }
}

struct ProcessChild(#[allow(dead_code)] Option<std::process::Child>);

impl QemuChild for ProcessChild {
    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

static RUNNING: Mutex<Vec<Box<dyn QemuChild>>> = Mutex::new(Vec::new());

pub fn is_aarch64(arch: &str) -> bool {
    matches!(arch, "aarch64" | "arm64")
}

/// Clamped MiB → qemu.org `-m` size (`2G` in the WHPX quick start).
pub fn memory_flag(memory_mib: u64) -> String {
    if memory_mib > 0 && memory_mib % 1024 == 0 {
        format!("{}G", memory_mib / 1024)
    } else {
        format!("{memory_mib}M")
    }
}

/// SLIRP user net so the Ubuntu guest gets DHCP without a host TAP device.
pub fn net_argv() -> Vec<String> {
    vec![
        NETDEV_ARG.to_string(),
        NETDEV_USER.to_string(),
        DEVICE_ARG.to_string(),
        VIRTIO_NET.to_string(),
    ]
}

/// Documented WHPX argv (no TCG, no `-enable-kvm`, no Hyper-V) plus user net.
/// `-drive` value for the verified raw guest disk on virtio-blk.
///
/// `snapshot=on`: the guest writes go to a temporary file QEMU deletes when
/// it exits. The signed disk is never changed, so every session (and every
/// renter) boots the same clean image and nothing of one renter is left for
/// the next. The renter's own files live in object storage, not here.
pub fn virtio_drive(disk: &str) -> String {
    format!("file={disk},if=virtio,format=raw,snapshot=on")
}

pub fn argv(arch: &str, vcpu_count: u32, memory_mib: u64, disk: &Path) -> Vec<String> {
    let smp = format!("cores={vcpu_count}");
    let memory = memory_flag(memory_mib);
    let disk = disk.to_string_lossy().into_owned();
    let mut args = if is_aarch64(arch) {
        vec![
            ACCEL_ARG.to_string(),
            ACCEL_WHPX.to_string(),
            MACHINE_ARG.to_string(),
            MACHINE_VIRT.to_string(),
            CPU_ARG.to_string(),
            CPU_HOST.to_string(),
            SMP_ARG.to_string(),
            smp,
            MEMORY_ARG.to_string(),
            memory,
            BIOS_ARG.to_string(),
            AARCH64_BIOS.to_string(),
            DEVICE_ARG.to_string(),
            RAMFB.to_string(),
            DEVICE_ARG.to_string(),
            USB_XHCI.to_string(),
            DEVICE_ARG.to_string(),
            USB_KBD.to_string(),
            DEVICE_ARG.to_string(),
            USB_TABLET.to_string(),
            DRIVE_ARG.to_string(),
            virtio_drive(&disk),
        ]
    } else {
        vec![
            ACCEL_ARG.to_string(),
            ACCEL_WHPX.to_string(),
            MACHINE_ARG.to_string(),
            MACHINE_PC.to_string(),
            SMP_ARG.to_string(),
            smp,
            MEMORY_ARG.to_string(),
            memory,
            DEVICE_ARG.to_string(),
            USB_EHCI.to_string(),
            DEVICE_ARG.to_string(),
            USB_TABLET.to_string(),
            DRIVE_ARG.to_string(),
            virtio_drive(&disk),
        ]
    };
    args.extend(net_argv());
    args
}

/// Start QEMU from a phase-21 launch document. Never used for Pro or Linux.
pub fn boot_from_launch(
    os: HostOs,
    lab_tier: LabTier,
    capabilities: &HostCapabilities,
    launch: &LaunchConfig,
    binary: &Path,
    arch: &str,
    runtime: &dyn QemuRuntime,
) -> Result<InvokeResult, QemuError> {
    if os != HostOs::Windows {
        return Err(QemuError::WindowsOnly(os));
    }
    if lab_tier != LabTier::Standard {
        return Err(QemuError::StandardOnly(lab_tier.as_str()));
    }
    let disk = launch
        .rootfs_path
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .ok_or(QemuError::DiskMissing)?;
    runtime.invoke(&InvokeRequest {
        os,
        lab_tier,
        capabilities,
        binary,
        disk,
        vcpu_count: launch.vcpu_count,
        memory_mib: launch.memory_mib,
        arch,
        bootstrap_origin: None,
        bootstrap_token: None,
    })
}

/// Same as [`boot_from_launch`], with Matchmaker origin+token on QEMU fw_cfg.
pub fn boot_from_launch_with_bootstrap(
    os: HostOs,
    lab_tier: LabTier,
    capabilities: &HostCapabilities,
    launch: &LaunchConfig,
    binary: &Path,
    arch: &str,
    bootstrap_origin: Option<&str>,
    bootstrap_token: Option<&str>,
    runtime: &dyn QemuRuntime,
) -> Result<InvokeResult, QemuError> {
    if os != HostOs::Windows {
        return Err(QemuError::WindowsOnly(os));
    }
    if lab_tier != LabTier::Standard {
        return Err(QemuError::StandardOnly(lab_tier.as_str()));
    }
    let disk = launch
        .rootfs_path
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .ok_or(QemuError::DiskMissing)?;
    runtime.invoke(&InvokeRequest {
        os,
        lab_tier,
        capabilities,
        binary,
        disk,
        vcpu_count: launch.vcpu_count,
        memory_mib: launch.memory_mib,
        arch,
        bootstrap_origin,
        bootstrap_token,
    })
}

pub fn invoke(
    request: &InvokeRequest<'_>,
    spawner: &dyn QemuSpawner,
) -> Result<InvokeResult, QemuError> {
    if request.os != HostOs::Windows {
        return Err(QemuError::WindowsOnly(request.os));
    }
    if request.lab_tier != LabTier::Standard {
        return Err(QemuError::StandardOnly(request.lab_tier.as_str()));
    }
    if !request.capabilities.standard.offered {
        return Err(QemuError::StandardNotOffered {
            reason: request.capabilities.standard.reason.clone(),
        });
    }
    if request.capabilities.whpx != WhpxCapability::Available {
        return Err(QemuError::WhpxUnavailable);
    }
    require_file(request.binary, QemuError::BinaryMissing)?;
    require_file(request.disk, QemuError::DiskMissing)?;
    if request.vcpu_count == 0 {
        return Err(QemuError::InvalidVcpu(request.vcpu_count));
    }

    let mut args = argv(
        request.arch,
        request.vcpu_count,
        request.memory_mib,
        request.disk,
    );
    args.extend(crate::guest_bootstrap::qemu_bootstrap_argv(
        request.bootstrap_origin,
        request.bootstrap_token,
    ));
    debug_assert!(
        !args.iter().any(|arg| arg.eq_ignore_ascii_case("tcg")),
        "Standard Windows must not pass TCG"
    );
    let child = spawner.spawn(request.binary, &args)?;
    spawner.detach(child);
    Ok(InvokeResult {
        hypervisor_invoked: true,
        argv: args,
    })
}

fn require_file(path: &Path, missing: QemuError) -> Result<(), QemuError> {
    if path.as_os_str().is_empty() || !path.is_file() {
        Err(missing)
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) struct RecordingQemu {
    pub spawns: Mutex<Vec<(std::path::PathBuf, Vec<String>)>>,
}

#[cfg(test)]
impl RecordingQemu {
    pub(crate) fn succeeding() -> Self {
        Self {
            spawns: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn spawn_calls(&self) -> Vec<(std::path::PathBuf, Vec<String>)> {
        self.spawns.lock().expect("spawn mutex").clone()
    }
}

#[cfg(test)]
struct RecordingChild;

#[cfg(test)]
impl QemuChild for RecordingChild {
    fn kill(&mut self) {}
}

#[cfg(test)]
impl QemuSpawner for RecordingQemu {
    fn spawn(&self, binary: &Path, args: &[String]) -> Result<Box<dyn QemuChild>, QemuError> {
        self.spawns
            .lock()
            .expect("spawn mutex")
            .push((binary.to_path_buf(), args.to_vec()));
        Ok(Box::new(RecordingChild))
    }

    fn detach(&self, _child: Box<dyn QemuChild>) {}
}

#[cfg(test)]
impl QemuRuntime for RecordingQemu {
    fn invoke(&self, request: &InvokeRequest<'_>) -> Result<InvokeResult, QemuError> {
        invoke(request, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_capabilities::{
        evaluate, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
    };
    use crate::provision::ComputeBackend;
    use std::fs;
    use std::path::PathBuf;

    fn windows_standard_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Available,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        })
    }

    fn windows_no_whpx_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Unavailable,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        })
    }

    fn fixture_tree() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("qemu").join("qemu-system-x86_64.exe");
        let disk = dir.path().join("standard").join("rootfs.img");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::create_dir_all(disk.parent().unwrap()).unwrap();
        fs::write(&binary, b"fake-qemu").unwrap();
        fs::write(&disk, b"fake-rootfs").unwrap();
        (dir, binary, disk)
    }

    fn request<'a>(
        caps: &'a HostCapabilities,
        binary: &'a Path,
        disk: &'a Path,
    ) -> InvokeRequest<'a> {
        InvokeRequest {
            os: HostOs::Windows,
            lab_tier: LabTier::Standard,
            capabilities: caps,
            binary,
            disk,
            vcpu_count: 4,
            memory_mib: 8192,
            arch: "x86_64",
            bootstrap_origin: None,
            bootstrap_token: None,
        }
    }

    #[test]
    fn windows_standard_invoke_locks_documented_whpx_argv() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let result = invoke(&request(&caps, &binary, &disk), &recorder).unwrap();

        assert!(result.hypervisor_invoked);
        let spawns = recorder.spawn_calls();
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].0, binary);
        assert_eq!(spawns[0].1, argv("x86_64", 4, 8192, &disk));
        assert_eq!(result.argv, spawns[0].1);
        assert!(spawns[0].1.windows(2).any(|w| {
            w[0] == NETDEV_ARG && w[1] == NETDEV_USER
        }));
        assert!(spawns[0].1.iter().any(|arg| arg == VIRTIO_NET));
        assert!(!spawns[0].1.iter().any(|arg| arg.eq_ignore_ascii_case("tcg")));
        assert!(!spawns[0].1.iter().any(|arg| arg.contains("Start-VM")));
        assert_eq!(spawns[0].1[0], ACCEL_ARG);
        assert_eq!(spawns[0].1[1], ACCEL_WHPX);
    }

    #[test]
    fn windows_standard_fw_cfg_carries_bootstrap_not_preauth() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let mut req = request(&caps, &binary, &disk);
        req.bootstrap_origin = Some("http://mm.test");
        req.bootstrap_token = Some("guest-token");
        let result = invoke(&req, &recorder).unwrap();
        assert!(result.argv.windows(2).any(|w| {
            w[0] == "-fw_cfg" && w[1].contains("opt/org.dcl/matchmaker") && w[1].contains("http://mm.test")
        }));
        assert!(result.argv.windows(2).any(|w| {
            w[0] == "-fw_cfg" && w[1].contains("opt/org.dcl/bootstrap") && w[1].contains("guest-token")
        }));
        assert!(!result.argv.iter().any(|arg| arg.contains("tskey-auth")));
        assert!(!result.argv.iter().any(|arg| arg.contains("AWS_SECRET")));
    }

    #[test]
    fn aarch64_locks_documented_whpx_argv() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let mut req = request(&caps, &binary, &disk);
        req.arch = "aarch64";
        let result = invoke(&req, &recorder).unwrap();
        assert_eq!(result.argv, argv("aarch64", 4, 8192, &disk));
        assert!(result.argv.windows(2).any(|w| {
            w[0] == NETDEV_ARG && w[1] == NETDEV_USER
        }));
        assert_eq!(result.argv[1], ACCEL_WHPX);
        assert!(!result.argv.iter().any(|arg| arg.eq_ignore_ascii_case("tcg")));
    }

    #[test]
    fn memory_flag_uses_g_when_whole_gib_else_m() {
        assert_eq!(memory_flag(2048), "2G");
        assert_eq!(memory_flag(4096), "4G");
        assert_eq!(memory_flag(8192), "8G");
        assert_eq!(memory_flag(1536), "1536M");
    }

    #[test]
    fn missing_binary_fails_before_spawn() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        fs::remove_file(&binary).unwrap();
        let recorder = RecordingQemu::succeeding();
        let error = invoke(&request(&caps, &binary, &disk), &recorder).unwrap_err();
        assert_eq!(error, QemuError::BinaryMissing);
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn every_boot_leaves_the_signed_disk_untouched() {
        // Without snapshot=on a session writes into rootfs.img: the next
        // renter would boot the previous renter's browser history, files
        // and mesh identity.
        let drive = virtio_drive("C:/dcl/rootfs.img");
        assert_eq!(drive, "file=C:/dcl/rootfs.img,if=virtio,format=raw,snapshot=on");
        let kvm = crate::qemu_kvm::argv(4, 4096, std::path::Path::new("/c/rootfs.img"));
        assert!(kvm.iter().any(|a| a.ends_with(",snapshot=on")), "{kvm:?}");
    }

    #[test]
    fn missing_disk_fails_before_spawn() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        fs::remove_file(&disk).unwrap();
        let recorder = RecordingQemu::succeeding();
        let error = invoke(&request(&caps, &binary, &disk), &recorder).unwrap_err();
        assert_eq!(error, QemuError::DiskMissing);
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn linux_does_not_spawn_qemu() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let mut req = request(&caps, &binary, &disk);
        req.os = HostOs::Linux;
        assert_eq!(
            invoke(&req, &recorder).unwrap_err(),
            QemuError::WindowsOnly(HostOs::Linux)
        );
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn macos_does_not_spawn_qemu() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let mut req = request(&caps, &binary, &disk);
        req.os = HostOs::MacOs;
        assert_eq!(
            invoke(&req, &recorder).unwrap_err(),
            QemuError::WindowsOnly(HostOs::MacOs)
        );
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn pro_does_not_use_qemu_whpx() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let mut req = request(&caps, &binary, &disk);
        req.lab_tier = LabTier::Pro;
        assert_eq!(
            invoke(&req, &recorder).unwrap_err(),
            QemuError::StandardOnly("pro")
        );
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn probe_without_whpx_does_not_spawn_and_does_not_use_tcg() {
        let caps = windows_no_whpx_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let error = invoke(&request(&caps, &binary, &disk), &recorder).unwrap_err();
        assert!(
            matches!(error, QemuError::StandardNotOffered { .. })
                || error == QemuError::WhpxUnavailable
        );
        assert!(error.to_string().contains("TCG") || error.to_string().contains("hypervisor"));
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn unknown_whpx_does_not_spawn() {
        let caps = evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Unknown,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        });
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let error = invoke(&request(&caps, &binary, &disk), &recorder).unwrap_err();
        assert!(matches!(
            error,
            QemuError::StandardNotOffered { .. } | QemuError::WhpxUnavailable
        ));
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn production_spawner_is_refused_under_cfg_test() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("qemu-system-x86_64.exe");
        fs::write(&binary, b"fake").unwrap();
        let disk = dir.path().join("disk.img");
        fs::write(&disk, b"disk").unwrap();
        let error = match ProcessSpawner.spawn(&binary, &argv("x86_64", 2, 4096, &disk)) {
            Err(error) => error,
            Ok(_) => panic!("production spawner must refuse under cfg(test)"),
        };
        assert_eq!(error, QemuError::RefusedInTests);
    }

    #[test]
    fn boot_from_launch_uses_slider_vcpu_ram_and_disk() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let launch = LaunchConfig {
            backend: ComputeBackend::QemuWhpx,
            lab_tier: LabTier::Standard,
            vcpu_count: 2,
            memory_mib: 4096,
            rootfs_path: Some(disk.to_string_lossy().into_owned()),
            gpu: None,
            hypervisor_invoked: false,
        };
        let result = boot_from_launch(
            HostOs::Windows,
            LabTier::Standard,
            &caps,
            &launch,
            &binary,
            "x86_64",
            &recorder,
        )
        .unwrap();
        assert!(result.hypervisor_invoked);
        let args = &recorder.spawn_calls()[0].1;
        assert_eq!(args[1], ACCEL_WHPX);
        assert_eq!(args[5], "cores=2");
        assert_eq!(args[7], "4G");
        assert_eq!(args[12], DRIVE_ARG);
        assert_eq!(args[13], virtio_drive(&disk.to_string_lossy()));
    }

    #[test]
    fn boot_from_launch_linux_does_not_spawn() {
        let caps = windows_standard_caps();
        let (_dir, binary, disk) = fixture_tree();
        let recorder = RecordingQemu::succeeding();
        let launch = LaunchConfig {
            backend: ComputeBackend::QemuKvm,
            lab_tier: LabTier::Standard,
            vcpu_count: 2,
            memory_mib: 4096,
            rootfs_path: Some(disk.to_string_lossy().into_owned()),
            gpu: None,
            hypervisor_invoked: false,
        };
        let error = boot_from_launch(
            HostOs::Linux,
            LabTier::Standard,
            &caps,
            &launch,
            &binary,
            "x86_64",
            &recorder,
        )
        .unwrap_err();
        assert_eq!(error, QemuError::WindowsOnly(HostOs::Linux));
        assert!(recorder.spawn_calls().is_empty());
    }

    #[test]
    fn production_source_does_not_start_hyperv_or_tcg() {
        let source = include_str!("qemu_whpx.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("qemu_whpx.rs should have a test module");
        assert!(!production.contains("Start-VM"));
        assert!(!production.contains("powershell"));
        assert!(!production.contains("-accel tcg"));
        assert!(!production.contains("ACCEL_TCG"));
        assert!(production.contains("ACCEL_WHPX"));
    }
}
