#!/bin/bash
set -euo pipefail
umask 077

# Deploy the platform on the VPS. Run from the repository checkout:
#   DCL_DOMAIN=your.domain bash scripts/vps-deploy.sh
# DCL_DOMAIN     public domain; desktop., storage. and mesh. subdomains must
#                resolve to this VPS too (required)
# DCL_ROOT       repository checkout (default: the folder above this script)
# DCL_USER       account that runs the Matchmaker service (default: you)
DOMAIN="${DCL_DOMAIN:?Set DCL_DOMAIN to your public domain, e.g. DCL_DOMAIN=lab.example.com bash scripts/vps-deploy.sh}"
DESKTOP_DOMAIN="desktop.${DOMAIN}"
STORAGE_DOMAIN="storage.${DOMAIN}"
MESH_DOMAIN="mesh.${DOMAIN}"
ROOT="${DCL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
DCL_USER="${DCL_USER:-$(id -un)}"
export DCL_ROOT="$ROOT"

# Heredocs below are quoted (no shell expansion); fill in these placeholders.
fill() {
  sed -e "s|__DCL_DOMAIN__|${DOMAIN}|g" -e "s|__DCL_ROOT__|${ROOT}|g" -e "s|__DCL_USER__|${DCL_USER}|g"
}
HS_PUBLIC="https://${MESH_DOMAIN}"

echo "dns_check $(getent hosts "$DOMAIN" | awk '{print $1}' | head -1)"
for required_domain in "$DOMAIN" "$DESKTOP_DOMAIN" "$STORAGE_DOMAIN" "$MESH_DOMAIN"; do
  if ! getent hosts "$required_domain" >/dev/null; then
    echo "missing_dns=$required_domain" >&2
    exit 1
  fi
done

sudo apt-get update -y
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y python3.12-venv python3-pip jq

if ! grep -q "$DOMAIN" /etc/hosts; then
  echo "127.0.0.1 $DOMAIN" | sudo tee -a /etc/hosts >/dev/null
fi

printf '%s\n' "net.ipv4.ip_forward=1" "net.ipv6.conf.all.forwarding=1" \
  | sudo tee /etc/sysctl.d/99-dcl-forward.conf >/dev/null
sudo sysctl -p /etc/sysctl.d/99-dcl-forward.conf >/dev/null || true

fill <<'PY' | python3 -
import re
from pathlib import Path
p = Path("__DCL_ROOT__/matchmaker/docker-compose.yml")
t = p.read_text()
old = '- "${POSTGRES_PORT:-5432}:5432"'
new = '- "127.0.0.1:${POSTGRES_PORT:-5432}:5432"'
if old in t:
    p.write_text(t.replace(old, new, 1))
elif "127.0.0.1:${POSTGRES_PORT" not in t:
    raise SystemExit("postgres ports line not found")
p = Path("__DCL_ROOT__/headscale/config.yaml")
t = p.read_text()
p.write_text(re.sub(
    r"(?m)^server_url:\s*.*$",
    "server_url: https://mesh.__DCL_DOMAIN__",
    t,
    count=1,
))
print("patched_compose_and_headscale_config")
PY

if [ ! -f "$ROOT/headscale/.env" ]; then
  printf 'HEADSCALE_SERVER_URL=%s\nHEADSCALE_PORT=8080\nHEADSCALE_EXIT_AUTHKEY=unused-until-exit-overlay\n' \
    "$HS_PUBLIC" > "$ROOT/headscale/.env"
  echo "wrote_headscale_env"
else
  echo "keep_headscale_env"
fi
fill <<'PY' | python3 -
from pathlib import Path

p = Path("__DCL_ROOT__/headscale/.env")
values = {}
for line in p.read_text().splitlines():
    if "=" in line and not line.lstrip().startswith("#"):
        key, value = line.split("=", 1)
        values[key] = value
values["HEADSCALE_SERVER_URL"] = "https://mesh.__DCL_DOMAIN__"
values["HEADSCALE_PORT"] = "8080"
p.write_text("".join(f"{key}={value}\n" for key, value in values.items()))
PY
chmod 600 "$ROOT/headscale/.env"

if [ ! -f "$ROOT/renterd/.env" ]; then
  echo "generating_renterd_seed"
  fill <<'PY' | python3 -
import hashlib, os, urllib.request
from pathlib import Path
words = urllib.request.urlopen(
    "https://raw.githubusercontent.com/bitcoin/bips/master/bip-0039/english.txt",
    timeout=30,
).read().decode().split()
if len(words) != 2048:
    raise SystemExit("bip39 wordlist download failed")
entropy = os.urandom(16)
digest = hashlib.sha256(entropy).digest()
bits = "".join(f"{b:08b}" for b in entropy) + f"{digest[0]:08b}"[:4]
seed = " ".join(words[int(bits[i:i+11], 2)] for i in range(0, 132, 11))
api_pw = os.urandom(24).hex()
Path("__DCL_ROOT__/renterd/.env").write_text(
    f'RENTERD_SEED="{seed}"\n'
    f"RENTERD_API_PASSWORD={api_pw}\n"
    "RENTERD_NETWORK=zen\n"
    "RENTERD_API_PORT=9980\n"
    "RENTERD_S3_PORT=8081\n"
    "RENTERD_GATEWAY_PORT=9981\n"
)
print("wrote_renterd_env")
print("seed_word_count=%d" % len(seed.split()))
PY
else
  echo "keep_renterd_env"
fi
chmod 600 "$ROOT/renterd/.env"

cd "$ROOT/headscale"
sudo docker compose up -d
echo "waiting_headscale"
for i in $(seq 1 60); do
  if curl -sf --max-time 2 "http://127.0.0.1:8080/key" >/dev/null 2>&1 \
     || curl -sf --max-time 2 "http://127.0.0.1:8080/health" >/dev/null 2>&1; then
    echo "headscale_http_up"
    break
  fi
  sleep 2
  if [ "$i" -eq 60 ]; then
    echo "headscale_timeout"
    sudo docker compose logs --tail 80 headscale || true
    exit 1
  fi
done

sudo docker compose exec -T headscale headscale users create dcl >/dev/null 2>&1 || true
HS_USER_JSON="$(sudo docker compose exec -T headscale headscale users list -o json 2>/dev/null \
  || sudo docker compose exec -T headscale headscale users list --output json 2>/dev/null \
  || true)"
echo "headscale_users_json_len=${#HS_USER_JSON}"
HS_USER_ID="$(printf '%s\n' "$HS_USER_JSON" | python3 -c "
import json,sys,re
raw=sys.stdin.read().strip()
uid='1'
try:
    data=json.loads(raw)
    if isinstance(data, dict):
        data=data.get('users') or data.get('Users') or []
    for u in data:
        name=str(u.get('name') or u.get('Name') or '')
        if name.rstrip('@') in {'dcl', 'dcl@'} or name=='dcl':
            uid=str(u.get('id') or u.get('ID') or u.get('Id') or '1')
            break
except Exception:
    m=re.search(r'(?m)^\\s*(\\d+)\\b', raw)
    if m:
        uid=m.group(1)
print(uid)
")"
echo "headscale_user_id=$HS_USER_ID"

parse_last_token() {
  python3 -c "
from pathlib import Path
import sys
raw = Path(sys.argv[1]).read_text()
lines = [ln.strip() for ln in raw.splitlines() if ln.strip()]
if not lines:
    raise SystemExit('empty cli output')
key = lines[-1].split()[-1]
if len(key) < 16:
    raise SystemExit('token too short')
Path(sys.argv[2]).write_text(key)
Path(sys.argv[1]).unlink()
print('parsed_ok')
" "$1" "$2"
}

if [ ! -f "$ROOT/.secret-headscale-api-key" ]; then
  sudo docker compose exec -T headscale headscale apikeys create \
    > "$ROOT/.secret-headscale-api-key.raw" 2>/dev/null
  parse_last_token "$ROOT/.secret-headscale-api-key.raw" "$ROOT/.secret-headscale-api-key"
fi
chmod 600 "$ROOT/.secret-headscale-api-key"

if [ ! -f "$ROOT/.secret-gateway-preauth" ]; then
  sudo docker compose exec -T headscale headscale preauthkeys create \
    --user "$HS_USER_ID" --reusable --expiration 8760h --tags tag:gateway \
    > "$ROOT/.secret-gateway-preauth.raw" 2>/dev/null \
    || sudo docker compose exec -T headscale headscale preauthkeys create \
         --user "$HS_USER_ID" --reusable --expiration 24h --tags tag:gateway \
         > "$ROOT/.secret-gateway-preauth.raw" 2>/dev/null
  parse_last_token "$ROOT/.secret-gateway-preauth.raw" "$ROOT/.secret-gateway-preauth"
fi
chmod 600 "$ROOT/.secret-gateway-preauth"

cd "$ROOT/renterd"
sudo docker compose up -d
echo "waiting_renterd"
for i in $(seq 1 90); do
  if curl -sf --max-time 2 http://127.0.0.1:9980/ >/dev/null 2>&1; then
    echo "renterd_http_up"
    break
  fi
  sleep 3
  if [ "$i" -eq 90 ]; then
    echo "renterd_timeout"
    sudo docker compose logs --tail 40 renterd || true
    exit 1
  fi
done

if [ ! -f "$ROOT/.secret-s3-access" ]; then
  openssl rand -hex 16 > "$ROOT/.secret-s3-access"
  openssl rand -hex 24 > "$ROOT/.secret-s3-secret"
fi
chmod 600 "$ROOT/.secret-s3-access" "$ROOT/.secret-s3-secret"

fill <<'PY' | python3 -
from pathlib import Path
root = Path("__DCL_ROOT__")
env = {}
for line in (root / "renterd/.env").read_text().splitlines():
    if "=" in line and not line.startswith("#"):
        k, v = line.split("=", 1)
        env[k] = v
access = (root / ".secret-s3-access").read_text().strip()
secret = (root / ".secret-s3-secret").read_text().strip()
(root / ".secret-s3-put.json").write_text(
    '{"v4Keypairs":{"%s":"%s"}}' % (access, secret)
)
print("prepared_s3_put")
PY
RENTERD_API_PASSWORD="$(python3 -c '
from pathlib import Path
import os
for line in Path(os.environ["DCL_ROOT"], "renterd/.env").read_text().splitlines():
    if line.startswith("RENTERD_API_PASSWORD="):
        print(line.split("=",1)[1])
        break
')"
HTTP_CODE="$(curl -sS -o /tmp/dcl-s3-put.out -w "%{http_code}" -u ":${RENTERD_API_PASSWORD}" \
  -X PUT "http://127.0.0.1:9980/api/bus/settings/s3" \
  -H "Content-Type: application/json" \
  --data-binary @"$ROOT/.secret-s3-put.json" || true)"
rm -f "$ROOT/.secret-s3-put.json" /tmp/dcl-s3-put.out
echo "renterd_s3_put_http=$HTTP_CODE"

if [ ! -f "$ROOT/matchmaker/.env" ]; then
  PG_PASS="$(openssl rand -hex 16)"
  export PG_PASS HS_USER_ID
  fill <<'PY' | python3 -
import os
from pathlib import Path
root = Path("__DCL_ROOT__")
pg = os.environ["PG_PASS"]
hs_user = os.environ["HS_USER_ID"]
api = (root / ".secret-headscale-api-key").read_text().strip()
access = (root / ".secret-s3-access").read_text().strip()
secret = (root / ".secret-s3-secret").read_text().strip()
resolve = os.urandom(24).hex()
bootstrap = os.urandom(24).hex()
approve = os.urandom(24).hex()
jwt = os.urandom(32).hex()
(root / ".secret-gateway-resolve").write_text(resolve)
(root / "matchmaker/.env").write_text(f"""POSTGRES_USER=matchmaker
POSTGRES_PASSWORD={pg}
POSTGRES_DB=matchmaker
POSTGRES_PORT=5432
DATABASE_URL=postgresql+psycopg://matchmaker:{pg}@127.0.0.1:5432/matchmaker
JWT_SECRET={jwt}
JWT_EXPIRE_MINUTES=1440
HOST_BOOTSTRAP_TOKEN={bootstrap}
HOST_APPROVE_TOKEN={approve}
HOST_AGENT_PROVISION_URL=
CREDITS_PER_MINUTE=1
STALE_SESSION_SWEEP_INTERVAL_SECONDS=15
SEED_HOST_DISPLAY_NAME=phase1-lab-1
SEED_HOST_KASM_URL=https://example.invalid/kasm
SEED_HOST_KASM_TOKEN=
SEED_HOST_AGENT_URL=
SEED_HOST_SHARED_LOGICAL_CORES=4
SEED_HOST_SHARED_MEMORY_BYTES=8589934592
SEED_HOST_SHARE_GPU=false
SEED_HOST_SHARED_GPU_ID=
IMAGE_STANDARD_DIGEST=6ed707af13eb04a928001996ae5fc7a49ab405e73534caacf86ad65c51bab8fe
IMAGE_STANDARD_SIGNATURE=WQjS2zyW+BjgfoZDK3IFx6OIwXPmRxO22PN4FpfEJNpjY/AgEsLmqLVak99QZyY14YKNI5hiARiPkulHNpsrCQ==
IMAGE_PRO_DIGEST=b5dc04d095774efc8fee56dfcbabbe7346788f29a7576aaa27248d9b8249b118
IMAGE_PRO_SIGNATURE=T1FrV8mlhTvhk+0laH74qnb6s6CTM2M+waEhIAtjszKlN3pZyQ+w18xDfp0xvbA3MO+FXvt8Z+B+hmXDrvBaDA==
HEADSCALE_URL=http://127.0.0.1:8080
HEADSCALE_LOGIN_SERVER=https://mesh.__DCL_DOMAIN__
HEADSCALE_API_KEY={api}
HEADSCALE_USER={hs_user}
HEADSCALE_EXIT_NODE=off
GATEWAY_PUBLIC_ORIGIN=https://__DCL_DOMAIN__
GATEWAY_RESOLVE_TOKEN={resolve}
GATEWAY_TOKEN_TTL_SECONDS=43200
MATCHMAKER_PUBLIC_ORIGIN=http://__DCL_DOMAIN__:8000
S3_ENDPOINT=http://127.0.0.1:8081
S3_GUEST_ENDPOINT=https://storage.__DCL_DOMAIN__
S3_REGION=us-east-1
S3_ACCESS_KEY={access}
S3_SECRET_KEY={secret}
""")
print("wrote_matchmaker_env")
PY
  chmod 600 "$ROOT/matchmaker/.env" "$ROOT/.secret-gateway-resolve"
else
  echo "keep_matchmaker_env"
fi
fill <<'PY' | python3 -
from pathlib import Path

p = Path("__DCL_ROOT__/matchmaker/.env")
values = {}
for line in p.read_text().splitlines():
    if "=" in line and not line.lstrip().startswith("#"):
        key, value = line.split("=", 1)
        values[key] = value
values.update({
    "HEADSCALE_LOGIN_SERVER": "https://mesh.__DCL_DOMAIN__",
    "GATEWAY_PUBLIC_ORIGIN": "https://desktop.__DCL_DOMAIN__",
    "MATCHMAKER_PUBLIC_ORIGIN": "https://__DCL_DOMAIN__/api",
    "S3_GUEST_ENDPOINT": "https://storage.__DCL_DOMAIN__",
})
p.write_text("".join(f"{key}={value}\n" for key, value in values.items()))
PY
chmod 600 "$ROOT/matchmaker/.env"

cd "$ROOT/matchmaker"
sudo docker compose up -d
echo "waiting_postgres"
for i in $(seq 1 60); do
  if sudo docker compose exec -T postgres pg_isready -U matchmaker >/dev/null 2>&1; then
    echo "postgres_ready"
    break
  fi
  sleep 2
  if [ "$i" -eq 60 ]; then
    echo "postgres_timeout"
    sudo docker compose logs --tail 40 postgres || true
    exit 1
  fi
done

cd "$ROOT/matchmaker"
if [ ! -x .venv/bin/uvicorn ]; then
  python3 -m venv .venv
  .venv/bin/pip install -U pip
  .venv/bin/pip install -e .
  echo "matchmaker_venv_installed"
else
  echo "keep_matchmaker_venv"
fi
set -a
# shellcheck disable=SC1091
source "$ROOT/matchmaker/.env"
set +a
./.venv/bin/alembic upgrade head
echo "alembic_ok"

fill <<'UNIT' | sudo tee /etc/systemd/system/dcl-matchmaker.service >/dev/null
[Unit]
Description=DCL Matchmaker
After=network.target docker.service
Wants=docker.service

[Service]
Type=simple
User=__DCL_USER__
Group=__DCL_USER__
WorkingDirectory=__DCL_ROOT__/matchmaker
ExecStart=__DCL_ROOT__/matchmaker/.venv/bin/uvicorn matchmaker.main:app --app-dir src --host 127.0.0.1 --port 8000
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl daemon-reload
sudo systemctl enable --now dcl-matchmaker.service
sleep 3
sudo systemctl --no-pager --full status dcl-matchmaker.service | sed -n '1,25p' || true

if [ ! -f "$ROOT/gateway/.env" ]; then
  fill <<'PY' | python3 -
from pathlib import Path
root = Path("__DCL_ROOT__")
resolve = (root / ".secret-gateway-resolve").read_text().strip()
preauth = (root / ".secret-gateway-preauth").read_text().strip()
(root / "gateway/.env").write_text(f"""GATEWAY_DOMAIN=__DCL_DOMAIN__
GATEWAY_DESKTOP_DOMAIN=desktop.__DCL_DOMAIN__
GATEWAY_STORAGE_DOMAIN=storage.__DCL_DOMAIN__
GATEWAY_MESH_DOMAIN=mesh.__DCL_DOMAIN__
GATEWAY_ACME_EMAIL=mohamedm8931@gmail.com
GATEWAY_PUBLIC_ORIGIN=https://desktop.__DCL_DOMAIN__
GATEWAY_RESOLVE_TOKEN={resolve}
MATCHMAKER_URL=http://127.0.0.1:8000
HEADSCALE_LOGIN_SERVER=https://mesh.__DCL_DOMAIN__
HEADSCALE_GATEWAY_AUTHKEY={preauth}
GATEWAY_FAIL_LIMIT=20
GATEWAY_FAIL_WINDOW_SECONDS=60
""")
print("wrote_gateway_env")
PY
  chmod 600 "$ROOT/gateway/.env"
else
  echo "keep_gateway_env"
fi
fill <<'PY' | python3 -
from pathlib import Path

p = Path("__DCL_ROOT__/gateway/.env")
values = {}
for line in p.read_text().splitlines():
    if "=" in line and not line.lstrip().startswith("#"):
        key, value = line.split("=", 1)
        values[key] = value
values.update({
    "GATEWAY_DOMAIN": "__DCL_DOMAIN__",
    "GATEWAY_DESKTOP_DOMAIN": "desktop.__DCL_DOMAIN__",
    "GATEWAY_STORAGE_DOMAIN": "storage.__DCL_DOMAIN__",
    "GATEWAY_MESH_DOMAIN": "mesh.__DCL_DOMAIN__",
    "GATEWAY_PUBLIC_ORIGIN": "https://desktop.__DCL_DOMAIN__",
    "HEADSCALE_LOGIN_SERVER": "https://mesh.__DCL_DOMAIN__",
})
p.write_text("".join(f"{key}={value}\n" for key, value in values.items()))
PY
chmod 600 "$ROOT/gateway/.env"

cd "$ROOT/gateway"
sudo docker compose up -d --build
echo "gateway_compose_up"
sleep 8

echo "==== STATUS ===="
cd "$ROOT/matchmaker" && sudo docker compose ps || true
cd "$ROOT/headscale" && sudo docker compose ps || true
cd "$ROOT/renterd" && sudo docker compose ps || true
cd "$ROOT/gateway" && sudo docker compose ps || true
sudo systemctl is-active dcl-matchmaker || true
curl -sS -o /dev/null -w "matchmaker_local_http=%{http_code}\n" http://127.0.0.1:8000/docs || true
curl -sS -o /dev/null -w "headscale_local_http=%{http_code}\n" http://127.0.0.1:8080/ || true
curl -sS -o /dev/null -w "gateway_loopback=%{http_code}\n" http://127.0.0.1:7843/ || true
echo DONE
