//! Spare disk on a given path (phase 23).
//!
//! Hardware detect still does not enumerate disks. This helper only asks how
//! much free space sits on the volume that would hold a guest image. Tests
//! inject a byte count into [`crate::eligibility`] and do not need this probe.

use std::path::Path;

use sysinfo::Disks;

/// Available bytes on the volume that contains `path`. Walks up to an existing
/// ancestor when the cache directory has not been created yet. `None` if this
/// OS does not report a matching mount.
pub fn available_bytes(path: &Path) -> Option<u64> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return available_on_existing(&current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn available_on_existing(path: &Path) -> Option<u64> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let disks = Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|disk| canon.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn reports_some_space_on_a_real_temp_dir() {
        let dir = TempDir::new().unwrap();
        let bytes = available_bytes(dir.path());
        if let Some(n) = bytes {
            assert!(n > 0, "reported spare disk should be positive: {n}");
        }

        let missing = dir.path().join("images").join("standard");
        let nested = available_bytes(&missing);
        if bytes.is_some() {
            assert!(nested.is_some(), "parent volume should still report space");
        }
    }
}
