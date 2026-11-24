//! This machine's own Matchmaker credential.
//!
//! `HOST_BOOTSTRAP_TOKEN` is the same on every Host Agent, so it cannot tell
//! one host from another. On first run the agent makes a random secret and
//! keeps it beside the sharing config:
//!
//! - Every call to the Matchmaker carries it in `X-DCL-Host-Secret`. The
//!   Matchmaker binds it to this host id on first use and refuses anyone
//!   else who names this host.
//! - The Matchmaker commands this agent (`/v1/provision`, `/v1/teardown`)
//!   with [`command_token`], a hash of the secret. The provision listener
//!   accepts only that, so another host's token cannot start a VM here.
//!
//! The derivation must match `matchmaker/src/matchmaker/host_credentials.py`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

pub const HOST_SECRET_FILE_NAME: &str = "host-secret";
pub const HOST_SECRET_HEADER: &str = "X-DCL-Host-Secret";

const SECRET_BYTES: usize = 32;
const MIN_SECRET_CHARS: usize = 32;
const MAX_SECRET_CHARS: usize = 256;
const COMMAND_DOMAIN: &str = "dcl-agent-command:";

#[derive(Debug, Error)]
pub enum HostSecretError {
    #[error("could not read the host secret: {0}")]
    Read(#[source] io::Error),
    #[error("could not write the host secret: {0}")]
    Write(#[source] io::Error),
    #[error("the host secret file is damaged. Delete it and register this PC again.")]
    Malformed,
    #[error("the operating system could not provide random bytes: {0}")]
    Random(String),
}

pub fn secret_file_path(dir: &Path) -> PathBuf {
    dir.join(HOST_SECRET_FILE_NAME)
}

/// The stored secret, creating it on first use. Never logged.
pub fn load_or_create(dir: &Path) -> Result<String, HostSecretError> {
    let path = secret_file_path(dir);
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let secret = contents.trim().to_string();
            if !is_well_formed(&secret) {
                return Err(HostSecretError::Malformed);
            }
            Ok(secret)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => create(dir, &path),
        Err(error) => Err(HostSecretError::Read(error)),
    }
}

fn create(dir: &Path, path: &Path) -> Result<String, HostSecretError> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::getrandom(&mut bytes).map_err(|error| HostSecretError::Random(error.to_string()))?;
    let secret = hex::encode(bytes);

    fs::create_dir_all(dir).map_err(HostSecretError::Write)?;
    let temp_path = path.with_extension("tmp");
    write_private(&temp_path, &secret).map_err(HostSecretError::Write)?;
    fs::rename(&temp_path, path).map_err(HostSecretError::Write)?;
    Ok(secret)
}

#[cfg(unix)]
fn write_private(path: &Path, secret: &str) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(secret.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &Path, secret: &str) -> io::Result<()> {
    // The app config directory is already per-user on Windows.
    fs::write(path, secret)
}

fn is_well_formed(secret: &str) -> bool {
    (MIN_SECRET_CHARS..=MAX_SECRET_CHARS).contains(&secret.len())
        && secret.chars().all(|c| c.is_ascii_graphic())
}

/// Bearer the Matchmaker presents when it commands this agent.
pub fn command_token(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(COMMAND_DOMAIN.as_bytes());
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_a_secret_once_and_reuses_it() {
        let dir = tempdir().unwrap();

        let first = load_or_create(dir.path()).unwrap();
        let second = load_or_create(dir.path()).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.len(), SECRET_BYTES * 2);
        assert!(is_well_formed(&first));
    }

    #[test]
    fn two_machines_get_different_secrets() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();

        assert_ne!(
            load_or_create(a.path()).unwrap(),
            load_or_create(b.path()).unwrap()
        );
    }

    #[test]
    fn leaves_no_temporary_file_behind() {
        let dir = tempdir().unwrap();
        load_or_create(dir.path()).unwrap();

        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![HOST_SECRET_FILE_NAME.to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn the_secret_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        load_or_create(dir.path()).unwrap();

        let mode = fs::metadata(secret_file_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_damaged_secret_file_is_refused_not_replaced() {
        let dir = tempdir().unwrap();
        fs::write(secret_file_path(dir.path()), "short").unwrap();

        assert!(matches!(
            load_or_create(dir.path()),
            Err(HostSecretError::Malformed)
        ));
        assert_eq!(
            fs::read_to_string(secret_file_path(dir.path())).unwrap(),
            "short"
        );
    }

    #[test]
    fn command_token_matches_the_matchmaker_derivation() {
        // Same vector as matchmaker/tests/test_host_credentials.py.
        let secret = "a".repeat(43);

        assert_eq!(
            command_token(&secret),
            "f0a8b803e11214aa716efab1602e1e23a546e71ccd05e082c11dda747ca70277"
        );
        assert_ne!(command_token(&secret), secret);
    }
}
