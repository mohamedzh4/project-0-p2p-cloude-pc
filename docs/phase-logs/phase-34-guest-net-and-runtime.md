# Handoff — phase 34 guest net, runtime attach, guest bootstrap (2026-09-11)

- Phase goal: Finish coding-completable leftover so a guest can get a NIC,
  a hypervisor attach, Matchmaker secrets without volunteer-disk keys, and
  teardown/FR6 software paths. Stay on zen renterd (free). No Stripe.
- Verified git state at start: local `main` after phase 35; did not push.
  Did not invent a domain. Did not switch `RENTERD_NETWORK` off zen. Did not
  claim metal or a cross-city confirmation.
- Files created/changed:
  - Host Agent: `guest_net.rs`, `firecracker.rs` TAP + `network-interfaces/eth0`,
    `qemu.rs` SLIRP + fw_cfg, `qemu_linux.rs` KVM+VFIO attach, `gpu_bind.rs`
    Start-VM/Stop-VM, `provision_http.rs` `/v1/teardown`, occupancy clear
  - Matchmaker: `guest_bootstrap.py`, `routers/guest.py`, alembic `0008`,
    provision teardown client, session/silence expire + teardown
  - Guest image: `dcl-guest-join.py` + systemd unit in Packer
  - Web harness: `storage.savedAsOf` copy
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, runbooks, this file
- Key decisions made and why:
  - **Firecracker TAP + static `ip=`**, not a DHCP daemon (no extra package
    on the volunteer).
  - **QEMU SLIRP user-net** instead of TAP-Windows / dnsmasq (free, Ubuntu
    cloud DHCP works).
  - **Guest bootstrap token** on provision/cmdline/fw_cfg. Headscale preauth
    and S3 keys only on `POST /v1/guest/bootstrap` (guest Bearer).
  - **MATCHMAKER_PUBLIC_ORIGIN optional.** Unset → no `bootstrapOrigin`;
    guest cannot join (honest). http allowed for lab compose.
  - **Linux Pro QEMU** uses distro `qemu-system-*` / `HOST_AGENT_QEMU_BIN`,
    not Weilnetz. Missing binary keeps VFIO bind-only.
  - **Windows Pro Start-VM** on existing `dcl-pro-guest`. Window stays
    `apply: false`. Hyper-V has no fw_cfg; baked join unit no-ops there.
  - Storage stays **renterd zen**, not Storj.
- Test results: Host Agent `cargo test --lib` **344 passed**; Matchmaker
  pytest **185 passed**; web vitest **54 passed**; guest-images unittest
  **14 passed**; gateway pytest **10 passed**. Tests still inject fakes —
  no `/dev/kvm`, GitHub, Weilnetz, live `/sys`, or `powershell.exe`.
- Orchestrator flags:
  - Hardware lines stay open. TAP/Kasm/Sunshine on metal is still a human
    report.
  - Human-only: gateway domain + DNS + VPS + ACME email;
    `HEADSCALE_GATEWAY_AUTHKEY`; `GATEWAY_RESOLVE_TOKEN`;
    `MATCHMAKER_PUBLIC_ORIGIN` reachable from the guest; funded Siacoin
    wallet only if they want mainnet (stay zen unless they say so).
  - Pairwise `tag:session-<uuid>` still impossible on Headscale 0.26.
  - Do not add Stripe/PayPal. Do not start Storj.
