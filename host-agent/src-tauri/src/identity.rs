//! Last Matchmaker registration, stored beside the sharing config.
//!
//! Sharing config stays v2 (`sharing-config.json`). The host id is not a
//! sharing cap, so it lives in its own file and can be reused on `PUT
//! /v1/hosts/{id}` without a schema bump.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const IDENTITY_FILE_NAME: &str = "host-identity.json";

/// Same bound as the sharing config: a planted file must not decide how much
/// memory the agent allocates.
const MAX_IDENTITY_BYTES: u64 = 64 * 1024;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("could not read the host identity: {0}")]
    Read(#[source] io::Error),
    #[error("could not write the host identity: {0}")]
    Write(#[source] io::Error),
    #[error("the host identity file is not valid JSON: {0}")]
    Parse(#[source] serde_json::Error),
    #[error("the host identity file is too large to be valid ({size} bytes)")]
    TooLarge { size: u64 },
    #[error("could not serialize the host identity: {0}")]
    Serialize(#[source] serde_json::Error),
}

/// What the Matchmaker last returned for this machine. Not live state: status
/// stays pending until an operator approves with a different token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRegistration {
    pub host_id: String,
    pub display_name: String,
    pub lab_tier: String,
    pub status: String,
    pub shared_logical_cores: u32,
    pub shared_memory_bytes: u64,
    pub share_gpu: bool,
    #[serde(default)]
    pub shared_gpu_id: Option<String>,
}

pub fn identity_file_path(dir: &Path) -> PathBuf {
    dir.join(IDENTITY_FILE_NAME)
}

pub fn load(dir: &Path) -> Result<Option<HostRegistration>, IdentityError> {
    let path = identity_file_path(dir);

    match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_IDENTITY_BYTES => {
            return Err(IdentityError::TooLarge {
                size: metadata.len(),
            })
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(IdentityError::Read(error)),
    }

    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(IdentityError::Read(error)),
    };

    serde_json::from_str(&contents)
        .map(Some)
        .map_err(IdentityError::Parse)
}

pub fn save(dir: &Path, registration: &HostRegistration) -> Result<(), IdentityError> {
    fs::create_dir_all(dir).map_err(IdentityError::Write)?;

    let serialized = serde_json::to_string_pretty(registration).map_err(IdentityError::Serialize)?;
    let path = identity_file_path(dir);
    let temp_path = path.with_extension("json.tmp");

    fs::write(&temp_path, serialized).map_err(IdentityError::Write)?;
    fs::rename(&temp_path, &path).map_err(IdentityError::Write)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn registration() -> HostRegistration {
        HostRegistration {
            host_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
            display_name: "lab-pc".to_string(),
            lab_tier: "standard".to_string(),
            status: "pending".to_string(),
            shared_logical_cores: 4,
            shared_memory_bytes: 8 * 1024 * 1024 * 1024,
            share_gpu: true,
            shared_gpu_id: Some("GPU-TEST-4070".to_string()),
        }
    }

    #[test]
    fn returns_none_when_nothing_has_been_saved() {
        let dir = tempdir().unwrap();

        assert_eq!(load(dir.path()).unwrap(), None);
    }

    #[test]
    fn saves_and_reloads_the_host_id() {
        let dir = tempdir().unwrap();
        let saved = registration();

        save(dir.path(), &saved).unwrap();

        assert_eq!(load(dir.path()).unwrap(), Some(saved));
    }

    #[test]
    fn refuses_an_implausibly_large_identity_file() {
        let dir = tempdir().unwrap();
        let padding = vec![b' '; (MAX_IDENTITY_BYTES + 1) as usize];
        fs::write(identity_file_path(dir.path()), padding).unwrap();

        assert!(matches!(
            load(dir.path()),
            Err(IdentityError::TooLarge { .. })
        ));
    }
}
