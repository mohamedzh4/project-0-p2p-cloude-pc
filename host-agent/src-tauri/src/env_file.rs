//! Operator `.env` for packaged Host Agent installs.
//!
//! `tauri dev` / `cargo run` already find `.env` in the working directory.
//! A copied AppImage, MSI, or NSIS install does not: cwd is whatever the
//! shortcut used. Load the same keys from a stable path without overriding
//! variables the operator already set in the process environment.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Same directory as `sharing-config.json` / `host-identity.json`.
pub const CONFIG_DIR_NAME: &str = "com.distributedcomputelab.hostagent";

/// Optional absolute (or relative) path to a dotenv file. If set, the file
/// must exist (fail fast). Keys already in the process environment still win.
pub const ENV_PATH_VAR: &str = "HOST_AGENT_ENV_FILE";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EnvFileError {
    #[error(
        "{ENV_PATH_VAR} is set but that file does not exist: {0}. Copy host-agent/.env.example, fill MATCHMAKER_URL / HOST_BOOTSTRAP_TOKEN / HOST_AGENT_URL, and point this variable at the file."
    )]
    MissingExplicit(String),
    #[error("could not read operator env file {path}: {detail}")]
    Invalid { path: String, detail: String },
}

/// Platform app-config directory (matches the Host Agent README table).
pub fn app_config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join(CONFIG_DIR_NAME))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(CONFIG_DIR_NAME)
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            Some(PathBuf::from(xdg).join(CONFIG_DIR_NAME))
        } else {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".config").join(CONFIG_DIR_NAME))
        }
    }
}

/// Files to feed to dotenv, first-listed wins for a given key.
///
/// Order: explicit `HOST_AGENT_ENV_FILE`, then cwd `.env`, then parent
/// `.env` (`cargo run` from `src-tauri/`), then next to the executable
/// (portable copy), then the app-config directory (installed app).
pub fn env_files_to_load(
    explicit: Option<&Path>,
    cwd: Option<&Path>,
    exe_dir: Option<&Path>,
    config_dir: Option<&Path>,
) -> Result<Vec<PathBuf>, EnvFileError> {
    let mut out = Vec::new();
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(EnvFileError::MissingExplicit(path.display().to_string()));
        }
        push_unique(&mut out, path.to_path_buf());
    }
    let mut candidates = Vec::new();
    if let Some(cwd) = cwd {
        candidates.push(cwd.join(".env"));
        candidates.push(cwd.join("..").join(".env"));
    }
    if let Some(exe_dir) = exe_dir {
        candidates.push(exe_dir.join(".env"));
    }
    if let Some(config_dir) = config_dir {
        candidates.push(config_dir.join(".env"));
    }
    for path in candidates {
        if path.is_file() {
            push_unique(&mut out, path);
        }
    }
    Ok(out)
}

/// Load dotenv files. Existing process environment is never overwritten.
pub fn load_operator_env() -> Result<Vec<PathBuf>, EnvFileError> {
    let explicit = std::env::var(ENV_PATH_VAR)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let files = env_files_to_load(
        explicit.as_deref(),
        std::env::current_dir().ok().as_deref(),
        std::env::current_exe()
            .ok()
            .as_ref()
            .and_then(|path| path.parent()),
        app_config_dir().as_deref(),
    )?;
    for path in &files {
        dotenvy::from_path(path).map_err(|error| EnvFileError::Invalid {
            path: path.display().to_string(),
            detail: error.to_string(),
        })?;
    }
    Ok(files)
}

fn push_unique(out: &mut Vec<PathBuf>, path: PathBuf) {
    if !out.iter().any(|existing| existing == &path) {
        out.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn missing_explicit_env_file_fails_fast() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nope.env");
        let error = env_files_to_load(Some(&missing), None, None, None).unwrap_err();
        match error {
            EnvFileError::MissingExplicit(path) => {
                assert!(path.contains("nope.env"));
            }
            other => panic!("expected MissingExplicit, got {other:?}"),
        }
    }

    #[test]
    fn explicit_file_is_first_then_cwd_then_exe_then_config() {
        let root = TempDir::new().unwrap();
        let explicit = root.path().join("operator.env");
        let cwd = root.path().join("cwd");
        let exe = root.path().join("bin");
        let config = root.path().join("config");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&exe).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::write(&explicit, "A=1\n").unwrap();
        fs::write(cwd.join(".env"), "B=1\n").unwrap();
        fs::write(exe.join(".env"), "C=1\n").unwrap();
        fs::write(config.join(".env"), "D=1\n").unwrap();

        let files =
            env_files_to_load(Some(&explicit), Some(&cwd), Some(&exe), Some(&config)).unwrap();
        assert_eq!(
            files,
            vec![
                explicit,
                cwd.join(".env"),
                exe.join(".env"),
                config.join(".env"),
            ]
        );
    }

    #[test]
    fn skips_missing_optional_files_and_loads_parent_env() {
        let root = TempDir::new().unwrap();
        let cwd = root.path().join("src-tauri");
        fs::create_dir_all(&cwd).unwrap();
        fs::write(
            root.path().join(".env"),
            "MATCHMAKER_URL=http://example.test\n",
        )
        .unwrap();

        let files = env_files_to_load(None, Some(&cwd), None, None).unwrap();
        assert_eq!(files, vec![cwd.join("..").join(".env")]);
        assert!(files[0].is_file());
    }

    #[test]
    fn packaged_config_dir_is_enough_when_cwd_has_no_env() {
        let root = TempDir::new().unwrap();
        let cwd = root.path().join("somewhere");
        let config = root.path().join(CONFIG_DIR_NAME);
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join(".env"), "HOST_BOOTSTRAP_TOKEN=test\n").unwrap();

        let files = env_files_to_load(None, Some(&cwd), None, Some(&config)).unwrap();
        assert_eq!(files, vec![config.join(".env")]);
    }
}
