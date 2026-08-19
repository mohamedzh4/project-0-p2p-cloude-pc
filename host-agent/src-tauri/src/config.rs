//! Local persistence for the sharing configuration.
//!
//! Every function takes the directory explicitly rather than resolving it from
//! the Tauri app handle, so the store can be tested without a running app and
//! reused later by a background service that has no window at all.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::models::{HardwareSnapshot, SharingConfig};

pub const CONFIG_FILE_NAME: &str = "sharing-config.json";

/// The real file is a few hundred bytes. Anything larger is a corrupted or
/// planted file, and reading it whole before parsing would let it decide how
/// much memory the agent allocates.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read the sharing configuration: {0}")]
    Read(#[source] io::Error),
    #[error("could not write the sharing configuration: {0}")]
    Write(#[source] io::Error),
    #[error("the sharing configuration file is not valid JSON: {0}")]
    Parse(#[source] serde_json::Error),
    #[error("the sharing configuration file is too large to be valid ({size} bytes)")]
    TooLarge { size: u64 },
    #[error("could not serialize the sharing configuration: {0}")]
    Serialize(#[source] serde_json::Error),
}

pub fn config_file_path(dir: &Path) -> PathBuf {
    dir.join(CONFIG_FILE_NAME)
}

/// Returns `None` when the host has not configured sharing yet.
pub fn load(dir: &Path) -> Result<Option<SharingConfig>, ConfigError> {
    let path = config_file_path(dir);

    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_CONFIG_BYTES => {
            return Err(ConfigError::TooLarge {
                size: metadata.len(),
            })
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConfigError::Read(error)),
    }

    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConfigError::Read(error)),
    };

    serde_json::from_str(&contents)
        .map(Some)
        .map_err(ConfigError::Parse)
}

/// Writes through a temporary file so an interrupted write cannot leave a
/// half-written config behind.
pub fn save(dir: &Path, config: &SharingConfig) -> Result<(), ConfigError> {
    fs::create_dir_all(dir).map_err(ConfigError::Write)?;

    let serialized = serde_json::to_string_pretty(config).map_err(ConfigError::Serialize)?;
    let path = config_file_path(dir);
    let temp_path = path.with_extension("json.tmp");

    fs::write(&temp_path, serialized).map_err(ConfigError::Write)?;
    fs::rename(&temp_path, &path).map_err(ConfigError::Write)
}

/// Loads the stored config clamped to the current machine, or the recommended
/// defaults when nothing has been saved yet.
pub fn load_for_hardware(
    dir: &Path,
    hardware: &HardwareSnapshot,
) -> Result<SharingConfig, ConfigError> {
    Ok(match load(dir)? {
        Some(stored) => stored.clamped_to(hardware),
        None => SharingConfig::recommended_for(hardware),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BYTES_PER_GIB, CONFIG_VERSION};
    use crate::test_support::{nvidia_gpu, snapshot};
    use tempfile::tempdir;

    fn config() -> SharingConfig {
        SharingConfig {
            version: CONFIG_VERSION,
            sharing_enabled: true,
            shared_logical_cores: 4,
            shared_memory_bytes: 8 * BYTES_PER_GIB,
            share_gpu: true,
        }
    }

    #[test]
    fn returns_none_when_nothing_has_been_saved() {
        let dir = tempdir().unwrap();

        assert_eq!(load(dir.path()).unwrap(), None);
    }

    #[test]
    fn saves_and_reloads_the_same_configuration() {
        let dir = tempdir().unwrap();
        let saved = config();

        save(dir.path(), &saved).unwrap();

        assert_eq!(load(dir.path()).unwrap(), Some(saved));
    }

    #[test]
    fn creates_the_config_directory_when_missing() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("host-agent").join("nested");

        save(&nested, &config()).unwrap();

        assert!(config_file_path(&nested).exists());
    }

    #[test]
    fn leaves_no_temporary_file_behind() {
        let dir = tempdir().unwrap();

        save(dir.path(), &config()).unwrap();

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "unexpected temp files: {leftovers:?}");
    }

    #[test]
    fn overwrites_a_previous_configuration() {
        let dir = tempdir().unwrap();
        save(dir.path(), &config()).unwrap();

        let updated = SharingConfig {
            shared_logical_cores: 2,
            ..config()
        };
        save(dir.path(), &updated).unwrap();

        assert_eq!(load(dir.path()).unwrap(), Some(updated));
    }

    #[test]
    fn reports_a_corrupted_config_file() {
        let dir = tempdir().unwrap();
        fs::write(config_file_path(dir.path()), "{ not json").unwrap();

        assert!(matches!(load(dir.path()), Err(ConfigError::Parse(_))));
    }

    #[test]
    fn refuses_an_implausibly_large_config_file() {
        let dir = tempdir().unwrap();
        let padding = vec![b' '; (MAX_CONFIG_BYTES + 1) as usize];
        fs::write(config_file_path(dir.path()), padding).unwrap();

        assert!(matches!(
            load(dir.path()),
            Err(ConfigError::TooLarge { .. })
        ));
    }

    #[test]
    fn falls_back_to_recommended_defaults_on_first_run() {
        let dir = tempdir().unwrap();
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);

        let loaded = load_for_hardware(dir.path(), &hardware).unwrap();

        assert_eq!(loaded, SharingConfig::recommended_for(&hardware));
    }

    #[test]
    fn clamps_a_stored_config_to_a_smaller_machine() {
        let dir = tempdir().unwrap();
        save(dir.path(), &config()).unwrap();
        let hardware = snapshot(2, 4 * BYTES_PER_GIB, Vec::new());

        let loaded = load_for_hardware(dir.path(), &hardware).unwrap();

        assert_eq!(loaded.shared_logical_cores, 2);
        assert_eq!(loaded.shared_memory_bytes, 4 * BYTES_PER_GIB);
        assert!(!loaded.share_gpu);
    }
}
