//! Host-to-guest networking for Firecracker TAP (phase 34).
//!
//! Firecracker has no user-mode NIC. The documented path is a TAP on the
//! volunteer plus a static guest address in `boot_args`. Tests inject
//! [`TapOps`] and never run `ip` or touch `/sys`. Production spawn is
//! refused under `cfg(test)`.
//!
//! QEMU Standard/Pro uses SLIRP user networking instead (free, no TAP
//! device, guest DHCP works on Ubuntu cloud images). Hyper-V uses the
//! operator's existing vSwitch on `dcl-pro-guest`.

use std::process::Command;

use thiserror::Error;

/// Firecracker TAP name. Short enough for `ip tuntap`.
pub const TAP_DEV: &str = "dcl-tap0";
/// Firecracker guest MAC (AA:FC prefix is the Firecracker getting-started convention).
pub const GUEST_MAC: &str = "AA:FC:00:00:00:01";
/// Host side of the TAP.
pub const HOST_IP: &str = "172.16.0.1";
/// Guest static address (kernel `ip=` cmdline). No DHCP daemon required.
pub const GUEST_IP: &str = "172.16.0.2";
pub const NETMASK: &str = "255.255.255.0";
pub const TAP_PREFIX: u8 = 24;

/// QEMU fw_cfg names the guest join script reads (not kernel cmdline).
pub const FW_CFG_MATCHMAKER: &str = "opt/org.dcl/matchmaker";
pub const FW_CFG_BOOTSTRAP: &str = "opt/org.dcl/bootstrap";

/// `-fw_cfg` argv so Ubuntu guests can redeem Matchmaker without TAP DHCP.
/// Origin/token with commas are omitted — QEMU treats comma as a field separator.
pub fn qemu_fw_cfg_argv(origin: Option<&str>, token: Option<&str>) -> Vec<String> {
    let Some(origin) = origin.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    if origin.contains(',') || token.contains(',') || origin.contains('\n') || token.contains('\n')
    {
        return Vec::new();
    }
    vec![
        "-fw_cfg".to_string(),
        format!("name={FW_CFG_MATCHMAKER},string={origin}"),
        "-fw_cfg".to_string(),
        format!("name={FW_CFG_BOOTSTRAP},string={token}"),
    ]
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TapError {
    #[error("TAP setup is Linux-only")]
    LinuxOnly,
    #[error("could not configure TAP {dev}: {detail}")]
    Setup { dev: String, detail: String },
    #[error("TAP setup is disabled in tests; inject TapOps")]
    RefusedInTests,
}

pub trait TapOps: Send + Sync {
    fn ensure(&self, dev: &str, host_ip: &str, prefix: u8) -> Result<(), TapError>;
    fn teardown(&self, dev: &str) -> Result<(), TapError> {
        let _ = (dev,);
        Ok(())
    }
}

/// Production: `ip tuntap` + address + up. Never used under `cfg(test)`.
pub struct ProcessTapOps;

impl TapOps for ProcessTapOps {
    fn ensure(&self, dev: &str, host_ip: &str, prefix: u8) -> Result<(), TapError> {
        if cfg!(test) {
            return Err(TapError::RefusedInTests);
        }
        if cfg!(not(target_os = "linux")) {
            return Err(TapError::LinuxOnly);
        }
        run_ip(&["tuntap", "add", "dev", dev, "mode", "tap"]).or_else(|error| {
            // Already exists from a previous session: still try to address/up.
            if error.contains("File exists") || error.contains("exists") {
                Ok(())
            } else {
                Err(TapError::Setup {
                    dev: dev.to_string(),
                    detail: error,
                })
            }
        })?;
        let cidr = format!("{host_ip}/{prefix}");
        run_ip(&["addr", "replace", &cidr, "dev", dev]).map_err(|detail| TapError::Setup {
            dev: dev.to_string(),
            detail,
        })?;
        run_ip(&["link", "set", "dev", dev, "up"]).map_err(|detail| TapError::Setup {
            dev: dev.to_string(),
            detail,
        })?;
        Ok(())
    }

    fn teardown(&self, dev: &str) -> Result<(), TapError> {
        if cfg!(test) {
            return Err(TapError::RefusedInTests);
        }
        if cfg!(not(target_os = "linux")) {
            return Ok(());
        }
        let _ = run_ip(&["link", "set", "dev", dev, "down"]);
        let _ = run_ip(&["tuntap", "del", "dev", dev, "mode", "tap"]);
        Ok(())
    }
}

/// Tests and Firecracker unit invoke: pretends the TAP exists.
pub struct NoopTap;

impl TapOps for NoopTap {
    fn ensure(&self, _dev: &str, _host_ip: &str, _prefix: u8) -> Result<(), TapError> {
        Ok(())
    }
}

/// Records TAP ensure/teardown for tests.
#[derive(Default)]
pub struct RecordingTap {
    pub ensures: std::sync::Mutex<Vec<(String, String, u8)>>,
    pub teardowns: std::sync::Mutex<Vec<String>>,
}

impl TapOps for RecordingTap {
    fn ensure(&self, dev: &str, host_ip: &str, prefix: u8) -> Result<(), TapError> {
        self.ensures
            .lock()
            .expect("tap ensure mutex")
            .push((dev.to_string(), host_ip.to_string(), prefix));
        Ok(())
    }

    fn teardown(&self, dev: &str) -> Result<(), TapError> {
        self.teardowns
            .lock()
            .expect("tap teardown mutex")
            .push(dev.to_string());
        Ok(())
    }
}

impl RecordingTap {
    pub fn ensure_calls(&self) -> Vec<(String, String, u8)> {
        self.ensures.lock().expect("tap ensure mutex").clone()
    }
}

/// Firecracker getting-started static `ip=` cmdline (no DHCP daemon).
pub fn firecracker_ip_boot_arg() -> String {
    format!("ip={GUEST_IP}::{HOST_IP}:{NETMASK}::eth0:off")
}

fn run_ip(args: &[&str]) -> Result<(), String> {
    let output = Command::new("ip")
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!("{}{}", stderr.trim(), stdout.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firecracker_ip_boot_arg_is_static_not_dhcp() {
        let arg = firecracker_ip_boot_arg();
        assert!(arg.contains(GUEST_IP));
        assert!(arg.contains(HOST_IP));
        assert!(arg.contains("eth0"));
        assert!(!arg.contains("dhcp"));
    }

    #[test]
    fn process_tap_ops_refuses_in_tests() {
        let error = ProcessTapOps
            .ensure(TAP_DEV, HOST_IP, TAP_PREFIX)
            .unwrap_err();
        assert_eq!(error, TapError::RefusedInTests);
    }

    #[test]
    fn recording_tap_stores_ensure() {
        let tap = RecordingTap::default();
        tap.ensure(TAP_DEV, HOST_IP, TAP_PREFIX).unwrap();
        assert_eq!(
            tap.ensure_calls(),
            vec![(TAP_DEV.to_string(), HOST_IP.to_string(), TAP_PREFIX)]
        );
    }

    #[test]
    fn qemu_fw_cfg_requires_origin_and_token_without_commas() {
        assert!(qemu_fw_cfg_argv(None, Some("tok")).is_empty());
        assert!(qemu_fw_cfg_argv(Some("http://mm.test"), None).is_empty());
        assert!(qemu_fw_cfg_argv(Some("http://mm,evil"), Some("tok")).is_empty());
        let args = qemu_fw_cfg_argv(Some("http://mm.test"), Some("guest-token"));
        assert_eq!(args[0], "-fw_cfg");
        assert!(args[1].contains(FW_CFG_MATCHMAKER));
        assert!(args[1].contains("http://mm.test"));
        assert!(args[3].contains(FW_CFG_BOOTSTRAP));
        assert!(args[3].contains("guest-token"));
        assert!(!args.iter().any(|arg| arg.contains("tskey-auth")));
    }
}
