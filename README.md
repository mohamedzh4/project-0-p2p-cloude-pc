# Distributed Compute Lab

People with idle PCs ("hosts") lend a slice of CPU/RAM, and optionally one
whole GPU, while they keep using the machine. Renters get a Linux desktop in
their browser, running in a VM on a host.

Where the project stands and what to do next: [`PROJECT_STATUS.md`](PROJECT_STATUS.md).
Open bugs and risks: [`docs/issues.md`](docs/issues.md).

## How a session works

```
renter's browser
   │  1. sign up, describe the task        web/
   ▼
Matchmaker ── 2. pick a free host ──────►  matchmaker/   (FastAPI + Postgres)
   │  3. POST /v1/provision
   ▼
Host Agent on the volunteer's PC           host-agent/   (Rust + Tauri + React)
   │  4. verify signed disk, start QEMU
   ▼
Ubuntu guest VM                            guest-images/ (Packer build + join script)
   │  5. join the private mesh, mount the renter's files
   ▼
Headscale mesh  +  renterd storage         headscale/  renterd/   (docker compose)
   │  6. desktop reachable only inside the mesh
   ▼
Public gateway ── 7. https link ────────►  gateway/      (FastAPI proxy + Caddy)
```

Two tiers. **Standard**: CPU/RAM only, desktop streamed by KasmVNC. **Pro**:
one GPU handed to the guest exclusively, streamed by Sunshine/Moonlight.

## Layout

| Folder | What it is | Language | Tests |
| --- | --- | --- | --- |
| `matchmaker/` | Control plane: accounts, host inventory, booking, credits ledger, mesh keys, storage buckets | Python (FastAPI, SQLAlchemy, Alembic) | `pytest`, needs Postgres |
| `host-agent/` | Desktop app on the host PC: detects hardware, registers, boots guests | Rust + Tauri, React UI | `cargo test --lib` |
| `host-agent/scripts/` | Host setup kits and the LAN hardware test kit | Bash, PowerShell | `pytest` |
| `web/` | Renter's browser console | React + TypeScript | `vitest` |
| `gateway/` | Public HTTPS entry to desktops that live inside the mesh | Python (FastAPI) | `pytest` |
| `guest-images/` | Builds and signs the guest disk; the script the guest runs at boot | Packer, Bash, Python | `pytest` |
| `headscale/`, `renterd/` | Compose files for the mesh and the storage node | YAML | — |
| `scripts/` | `check.sh` (all tests), `vps-deploy.sh` (server deploy) | Bash | — |
| `docs/` | Specs, runbooks, issues, hardware test logs, history | Markdown | — |

Each component has its own README with setup details.

## Run the tests

```bash
bash scripts/check.sh
```

Runs lint and every suite and prints one summary. Name suites to run fewer:
`bash scripts/check.sh agent web`. The Matchmaker suite needs Postgres
(`cd matchmaker && docker compose up -d`); without it that suite is reported
as skipped. The same suites run in GitHub Actions (`.github/workflows/ci.yml`).

## Docs

- `docs/distributed-compute-lab-architecture.md`: target architecture
- `docs/cahier-des-charges-distributed-compute-lab.md`: requirements (FR1–FR16)
- `docs/runbooks/`: step-by-step operator guides. Start with `lan-test.md`
  (one PC, no server), then `demo.md` (full path through a VPS)
- `docs/test-logs/`: results from real hardware
- `docs/issues.md`: known bugs, ordered by severity
- `docs/phase-logs/`, `docs/archive/`: history of how the code was built.
  Not maintained; some of it describes designs that have since changed

## Rules that are easy to break

- Never commit `.env`, signing keys, renterd seeds, or Headscale keys.
- Nothing is "confirmed working" until a person has seen it on real
  hardware. Record that in `docs/test-logs/`.
- Tests never start a real VM, touch `/sys`, run PowerShell, or call a live
  Headscale or renterd. Each of those is injected; keep it that way.
