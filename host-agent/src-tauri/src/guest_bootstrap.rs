//! How the Host Agent hands a guest its Matchmaker origin and bootstrap token.
//!
//! Every backend is QEMU, and every guest uses QEMU SLIRP user networking
//! (no TAP device), so the only host → guest channel at boot is the QEMU
//! command line. The guest join script (`guest-images/scripts/dcl-guest-join.py`)
//! reads these values; it never receives a Headscale preauth key this way.

/// QEMU fw_cfg names the guest join script reads (not kernel cmdline).
pub const FW_CFG_MATCHMAKER: &str = "opt/org.dcl/matchmaker";
pub const FW_CFG_BOOTSTRAP: &str = "opt/org.dcl/bootstrap";
/// SMBIOS OEM-string (type 11) keys the guest join script reads.
pub const SMBIOS_MATCHMAKER_KEY: &str = "dcl.matchmaker";
pub const SMBIOS_BOOTSTRAP_KEY: &str = "dcl.bootstrap";

/// QEMU argv that hands the guest its Matchmaker origin and bootstrap token.
///
/// SMBIOS OEM strings (`-smbios type=11`) come first: the stock Ubuntu cloud
/// kernel the guest image is built from ships no `qemu_fw_cfg` module, so
/// fw_cfg alone never reaches the guest. fw_cfg is still passed for images
/// that do carry the module. Origin/token with commas are omitted — QEMU
/// treats comma as a field separator.
pub fn qemu_bootstrap_argv(origin: Option<&str>, token: Option<&str>) -> Vec<String> {
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
        "-smbios".to_string(),
        format!(
            "type=11,value={SMBIOS_MATCHMAKER_KEY}={origin},value={SMBIOS_BOOTSTRAP_KEY}={token}"
        ),
        "-fw_cfg".to_string(),
        format!("name={FW_CFG_MATCHMAKER},string={origin}"),
        "-fw_cfg".to_string(),
        format!("name={FW_CFG_BOOTSTRAP},string={token}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qemu_bootstrap_requires_origin_and_token_without_commas() {
        assert!(qemu_bootstrap_argv(None, Some("tok")).is_empty());
        assert!(qemu_bootstrap_argv(Some("http://mm.test"), None).is_empty());
        assert!(qemu_bootstrap_argv(Some("http://mm,evil"), Some("tok")).is_empty());
        let args = qemu_bootstrap_argv(Some("http://mm.test"), Some("guest-token"));
        assert_eq!(args[0], "-smbios");
        assert_eq!(
            args[1],
            "type=11,value=dcl.matchmaker=http://mm.test,value=dcl.bootstrap=guest-token"
        );
        assert_eq!(args[2], "-fw_cfg");
        assert!(args[3].contains(FW_CFG_MATCHMAKER));
        assert!(args[3].contains("http://mm.test"));
        assert!(args[5].contains(FW_CFG_BOOTSTRAP));
        assert!(args[5].contains("guest-token"));
        assert!(!args.iter().any(|arg| arg.contains("tskey-auth")));
    }
}
