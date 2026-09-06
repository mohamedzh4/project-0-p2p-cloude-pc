//! Hypervisor binary locate/fetch (phase 19).
//!
//! After a capability probe, Linux Standard locates or fetches Firecracker into
//! a user-writable cache (documented GitHub URL + SHA-256). Windows Standard
//! records **would fetch QEMU** (silent install is 19b). macOS skips with a
//! logged reason — it does not pretend Firecracker runs here.
//!
//! Nothing in this module invokes a hypervisor. The webview does not fetch
//! (CSP allows no remote origins). Tests inject a fetcher, `file://`, or a
//! loopback httptest; non-loopback HTTP is refused under `cfg(test)` unless
//! `HOST_AGENT_HYPERVISOR_NETWORK=1`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::host_capabilities::HostCapabilities;
use crate::provision::HostOs;

pub const FIRECRACKER_VERSION: &str = "v1.16.1";
pub const FIRECRACKER_RELEASE_BASE: &str =
    "https://github.com/firecracker-microvm/firecracker/releases/download/v1.16.1";
/// Official `firecracker-v1.16.1-x86_64.tgz.sha256.txt`.
pub const FIRECRACKER_SHA256_X86_64: &str =
    "382a02a869e4d6d5cb14c40577f9545e8458021ea8b0b2d3fc10ec14d9c242e6";
/// Official `firecracker-v1.16.1-aarch64.tgz.sha256.txt`.
pub const FIRECRACKER_SHA256_AARCH64: &str =
    "8d0e69f6d6f9a1724551f607f18504052c16c1828ee3d4d7b6e6c73380871e0e";
/// Documented Windows QEMU index. Phase 19 does not download it.
pub const QEMU_WINDOWS_INDEX_URL: &str = "https://qemu.weilnetz.de/w64/";

const CACHE_ENV: &str = "HOST_AGENT_HYPERVISOR_CACHE";
const NETWORK_ENV: &str = "HOST_AGENT_HYPERVISOR_NETWORK";
const FC_URL_ENV: &str = "HOST_AGENT_FIRECRACKER_URL";
const FC_SHA_ENV: &str = "HOST_AGENT_FIRECRACKER_SHA256";
const FC_VERSION_ENV: &str = "HOST_AGENT_FIRECRACKER_VERSION";
const FC_KIND_ENV: &str = "HOST_AGENT_FIRECRACKER_KIND";
const QEMU_URL_ENV: &str = "HOST_AGENT_QEMU_URL";
const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HypervisorError {
    #[error("Standard sessions are not offered; not fetching a hypervisor ({reason})")]
    StandardNotOffered { reason: String },
    #[error("checksum mismatch for {name}: expected {expected}, got {actual}; not installed")]
    ChecksumMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    #[error("{FC_SHA_ENV} / advertised checksum must be a 64-character SHA-256 hex digest")]
    InvalidChecksum,
    #[error("no documented Firecracker artifact for architecture {arch}")]
    UnsupportedArch { arch: String },
    #[error("Firecracker version {0:?} is not a safe cache directory name")]
    InvalidVersion(String),
    #[error("could not fetch {url}: {detail}")]
    FetchFailed { url: String, detail: String },
    #[error(
        "non-loopback hypervisor fetch is disabled in tests; set {NETWORK_ENV}=1 to allow {url}"
    )]
    NetworkDisabled { url: String },
    #[error("could not install the hypervisor binary: {0}")]
    Install(String),
    #[error("Firecracker tarball did not contain a firecracker binary")]
    MissingFirecrackerMember,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnsureAction {
    SkippedPresent,
    Fetched,
    SkippedOs,
    WouldFetch,
    Refused,
    ChecksumMismatch,
    FetchFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HypervisorName {
    Firecracker,
    Qemu,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    RawBinary,
    GzipTarFirecracker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSpec {
    pub name: String,
    pub version: String,
    pub url: String,
    pub sha256_hex: String,
    pub kind: ArtifactKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallManifest {
    version: String,
    url: String,
    artifact_sha256: String,
    binary_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HypervisorStatus {
    pub action: EnsureAction,
    pub hypervisor: HypervisorName,
    pub version: Option<String>,
    pub path: Option<String>,
    pub log: String,
    pub hypervisor_invoked: bool,
}

impl HypervisorStatus {
    fn success(
        action: EnsureAction,
        hypervisor: HypervisorName,
        version: Option<String>,
        path: Option<PathBuf>,
        log: String,
    ) -> Self {
        Self {
            action,
            hypervisor,
            version,
            path: path.map(|p| p.to_string_lossy().into_owned()),
            log,
            hypervisor_invoked: false,
        }
    }
}

pub trait BinaryFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, HypervisorError>;
}

/// Production/HTTP fetcher. `file://` is always allowed. Loopback HTTP is
/// allowed in tests (httptest). Other HTTP is refused under `cfg(test)` unless
/// [`NETWORK_ENV`] is `1`.
pub struct HttpBinaryFetcher;

impl BinaryFetcher for HttpBinaryFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, HypervisorError> {
        if let Some(path) = file_url_path(url) {
            return read_limited_file(&path, url);
        }
        if url.starts_with("http://") || url.starts_with("https://") {
            if !http_fetch_permitted(url) {
                return Err(HypervisorError::NetworkDisabled {
                    url: url.to_string(),
                });
            }
            return http_get(url);
        }
        Err(HypervisorError::FetchFailed {
            url: url.to_string(),
            detail: "URL must be http(s) or file://".to_string(),
        })
    }
}

pub fn network_env_enabled() -> bool {
    matches!(
        std::env::var(NETWORK_ENV).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    )
}

fn http_fetch_permitted(url: &str) -> bool {
    if is_loopback_http(url) {
        return true;
    }
    if cfg!(test) {
        network_env_enabled()
    } else {
        true
    }
}

fn is_loopback_http(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    let Some(rest) = rest else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .unwrap_or(host)
            .trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or(host)
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    if cfg!(windows) && rest.starts_with('/') && rest.len() >= 3 {
        let bytes: Vec<char> = rest.chars().collect();
        if bytes.get(2) == Some(&':') {
            return Some(PathBuf::from(&rest[1..]));
        }
    }
    Some(PathBuf::from(rest))
}

fn read_limited_file(path: &Path, url: &str) -> Result<Vec<u8>, HypervisorError> {
    let file = fs::File::open(path).map_err(|error| HypervisorError::FetchFailed {
        url: url.to_string(),
        detail: error.to_string(),
    })?;
    read_limited(file, url)
}

fn http_get(url: &str) -> Result<Vec<u8>, HypervisorError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    match agent.get(url).call() {
        Ok(response) => read_limited(response.into_reader(), url),
        Err(ureq::Error::Status(status, response)) => {
            let detail = response
                .into_string()
                .unwrap_or_else(|_| format!("HTTP {status}"));
            Err(HypervisorError::FetchFailed {
                url: url.to_string(),
                detail: format!("HTTP {status}: {detail}"),
            })
        }
        Err(error) => Err(HypervisorError::FetchFailed {
            url: url.to_string(),
            detail: error.to_string(),
        }),
    }
}

fn read_limited(mut reader: impl Read, url: &str) -> Result<Vec<u8>, HypervisorError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|error| HypervisorError::FetchFailed {
                url: url.to_string(),
                detail: error.to_string(),
            })?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > MAX_ARTIFACT_BYTES {
            return Err(HypervisorError::FetchFailed {
                url: url.to_string(),
                detail: format!("artifact larger than {MAX_ARTIFACT_BYTES} bytes"),
            });
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

pub fn resolve_cache_dir(getenv: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(dir) = getenv(CACHE_ENV) {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_cache_dir()
}

fn default_cache_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg)
            .join("dcl-host-agent")
            .join("hypervisors");
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| PathBuf::from(h).join("AppData").join("Local")))
            .unwrap_or_else(|| PathBuf::from("."))
    } else if cfg!(target_os = "macos") {
        home.map(|h| PathBuf::from(h).join("Library").join("Caches"))
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        home.map(|h| PathBuf::from(h).join(".cache"))
            .unwrap_or_else(|| PathBuf::from("."))
    };
    base.join("dcl-host-agent").join("hypervisors")
}

pub fn qemu_index_url(getenv: impl Fn(&str) -> Option<String>) -> String {
    getenv(QEMU_URL_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| QEMU_WINDOWS_INDEX_URL.to_string())
}

pub fn firecracker_spec(
    arch: &str,
    getenv: impl Fn(&str) -> Option<String>,
) -> Result<ArtifactSpec, HypervisorError> {
    let version = getenv(FC_VERSION_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| FIRECRACKER_VERSION.to_string());
    let (default_url, default_sha) = documented_firecracker(arch)?;
    let url = getenv(FC_URL_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(default_url);
    let sha = getenv(FC_SHA_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(default_sha);
    let kind = match getenv(FC_KIND_ENV)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "raw" | "binary" => ArtifactKind::RawBinary,
        _ => ArtifactKind::GzipTarFirecracker,
    };
    Ok(ArtifactSpec {
        name: "firecracker".to_string(),
        version,
        url,
        sha256_hex: normalize_sha256(&sha)?,
        kind,
    })
}

fn documented_firecracker(arch: &str) -> Result<(String, String), HypervisorError> {
    let (suffix, sha) = match arch {
        "x86_64" | "amd64" => ("x86_64", FIRECRACKER_SHA256_X86_64),
        "aarch64" | "arm64" => ("aarch64", FIRECRACKER_SHA256_AARCH64),
        other => {
            return Err(HypervisorError::UnsupportedArch {
                arch: other.to_string(),
            })
        }
    };
    Ok((
        format!("{FIRECRACKER_RELEASE_BASE}/firecracker-{FIRECRACKER_VERSION}-{suffix}.tgz"),
        sha.to_string(),
    ))
}

/// Locate or fetch the OS-appropriate hypervisor binary. Never runs it.
pub fn ensure(
    os: HostOs,
    caps: &HostCapabilities,
    cache_dir: &Path,
    firecracker: &ArtifactSpec,
    qemu_url: &str,
    fetcher: &dyn BinaryFetcher,
) -> Result<HypervisorStatus, HypervisorError> {
    match os {
        HostOs::MacOs | HostOs::Other => Ok(skip_unsupported(os, caps)),
        HostOs::Linux => ensure_linux(caps, cache_dir, firecracker, fetcher),
        HostOs::Windows => ensure_windows(caps, qemu_url),
    }
}

pub fn ensure_from_env(
    os: HostOs,
    caps: &HostCapabilities,
    arch: &str,
    getenv: impl Fn(&str) -> Option<String>,
    fetcher: &dyn BinaryFetcher,
) -> Result<HypervisorStatus, HypervisorError> {
    let cache = resolve_cache_dir(&getenv);
    let qemu = qemu_index_url(&getenv);
    let firecracker = match os {
        HostOs::Linux => firecracker_spec(arch, &getenv)?,
        _ => ArtifactSpec {
            name: "firecracker".to_string(),
            version: FIRECRACKER_VERSION.to_string(),
            url: "file:///unused".to_string(),
            sha256_hex: "00".repeat(32),
            kind: ArtifactKind::RawBinary,
        },
    };
    ensure(os, caps, &cache, &firecracker, &qemu, fetcher)
}

pub fn status_from_result(result: Result<HypervisorStatus, HypervisorError>) -> HypervisorStatus {
    match result {
        Ok(status) => {
            debug_assert!(!status.hypervisor_invoked);
            status
        }
        Err(HypervisorError::StandardNotOffered { reason }) => HypervisorStatus::success(
            EnsureAction::Refused,
            HypervisorName::None,
            None,
            None,
            format!("refused: Standard is not offered; not fetching a hypervisor. {reason}"),
        ),
        Err(error @ HypervisorError::ChecksumMismatch { .. }) => HypervisorStatus::success(
            EnsureAction::ChecksumMismatch,
            HypervisorName::Firecracker,
            None,
            None,
            error.to_string(),
        ),
        Err(error) => HypervisorStatus::success(
            EnsureAction::FetchFailed,
            HypervisorName::None,
            None,
            None,
            error.to_string(),
        ),
    }
}

fn skip_unsupported(os: HostOs, caps: &HostCapabilities) -> HypervisorStatus {
    let log = match os {
        HostOs::MacOs => format!(
            "skipped: this Mac cannot host; Firecracker is Linux-only and is not fetched. {}",
            caps.standard.reason
        ),
        _ => format!(
            "skipped: this OS cannot host; no hypervisor binary fetched. {}",
            caps.standard.reason
        ),
    };
    HypervisorStatus::success(
        EnsureAction::SkippedOs,
        HypervisorName::None,
        None,
        None,
        log,
    )
}

fn ensure_linux(
    caps: &HostCapabilities,
    cache_dir: &Path,
    spec: &ArtifactSpec,
    fetcher: &dyn BinaryFetcher,
) -> Result<HypervisorStatus, HypervisorError> {
    if !caps.standard.offered {
        return Err(HypervisorError::StandardNotOffered {
            reason: caps.standard.reason.clone(),
        });
    }
    locate_or_fetch_firecracker(cache_dir, spec, fetcher)
}

fn ensure_windows(
    caps: &HostCapabilities,
    qemu_url: &str,
) -> Result<HypervisorStatus, HypervisorError> {
    if !caps.standard.offered {
        return Err(HypervisorError::StandardNotOffered {
            reason: caps.standard.reason.clone(),
        });
    }
    Ok(HypervisorStatus::success(
        EnsureAction::WouldFetch,
        HypervisorName::Qemu,
        None,
        None,
        format!(
            "would fetch QEMU from {qemu_url} for Windows Standard (phase 19b silent install); not fetched"
        ),
    ))
}

fn locate_or_fetch_firecracker(
    cache_dir: &Path,
    spec: &ArtifactSpec,
    fetcher: &dyn BinaryFetcher,
) -> Result<HypervisorStatus, HypervisorError> {
    let expected = normalize_sha256(&spec.sha256_hex)?;
    let dest = firecracker_install_path(cache_dir, &spec.version)?;
    let manifest_path = dest.with_extension("manifest.json");

    if let Some(status) = skip_if_present(&dest, &manifest_path, spec, &expected) {
        return Ok(status);
    }

    let artifact = fetcher.fetch(&spec.url)?;
    let actual = sha256_hex(&artifact);
    if actual != expected {
        remove_install(&dest, &manifest_path);
        return Err(HypervisorError::ChecksumMismatch {
            name: spec.name.clone(),
            expected,
            actual,
        });
    }

    let binary = match spec.kind {
        ArtifactKind::RawBinary => artifact,
        ArtifactKind::GzipTarFirecracker => extract_firecracker_binary(&artifact)?,
    };
    install_binary(&dest, &binary)?;
    write_manifest(
        &manifest_path,
        &InstallManifest {
            version: spec.version.clone(),
            url: spec.url.clone(),
            artifact_sha256: expected,
            binary_sha256: sha256_hex(&binary),
        },
    )?;

    Ok(HypervisorStatus::success(
        EnsureAction::Fetched,
        HypervisorName::Firecracker,
        Some(spec.version.clone()),
        Some(dest.clone()),
        format!(
            "fetched firecracker {} ({} bytes) to {}; checksum ok",
            spec.version,
            binary.len(),
            dest.display()
        ),
    ))
}

fn skip_if_present(
    dest: &Path,
    manifest_path: &Path,
    spec: &ArtifactSpec,
    expected_artifact: &str,
) -> Option<HypervisorStatus> {
    let Ok(bytes) = fs::read(dest) else {
        return None;
    };
    let Ok(raw) = fs::read_to_string(manifest_path) else {
        return None;
    };
    let Ok(manifest) = serde_json::from_str::<InstallManifest>(&raw) else {
        return None;
    };
    if manifest.artifact_sha256 != expected_artifact {
        return None;
    }
    if sha256_hex(&bytes) != manifest.binary_sha256 {
        return None;
    }
    Some(HypervisorStatus::success(
        EnsureAction::SkippedPresent,
        HypervisorName::Firecracker,
        Some(spec.version.clone()),
        Some(dest.to_path_buf()),
        format!(
            "skipped because present; firecracker {} at {} (sha256 {})",
            spec.version,
            dest.display(),
            manifest.binary_sha256
        ),
    ))
}

fn firecracker_install_path(cache_dir: &Path, version: &str) -> Result<PathBuf, HypervisorError> {
    let version = version.trim();
    if version.is_empty()
        || version.contains(['/', '\\'])
        || version.split(['/', '\\']).any(|part| part == "..")
        || version == "."
        || version == ".."
    {
        return Err(HypervisorError::InvalidVersion(version.to_string()));
    }
    Ok(cache_dir
        .join("firecracker")
        .join(version)
        .join("firecracker"))
}

fn install_binary(path: &Path, bytes: &[u8]) -> Result<(), HypervisorError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| HypervisorError::Install(error.to_string()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|error| HypervisorError::Install(error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
            .map_err(|error| HypervisorError::Install(error.to_string()))?;
    }
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        HypervisorError::Install(error.to_string())
    })?;
    Ok(())
}

fn write_manifest(path: &Path, manifest: &InstallManifest) -> Result<(), HypervisorError> {
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|error| HypervisorError::Install(error.to_string()))?;
    fs::write(path, json).map_err(|error| HypervisorError::Install(error.to_string()))
}

fn remove_install(dest: &Path, manifest: &Path) {
    let _ = fs::remove_file(dest);
    let _ = fs::remove_file(manifest);
    let _ = fs::remove_file(dest.with_extension("tmp"));
}

fn extract_firecracker_binary(tarball: &[u8]) -> Result<Vec<u8>, HypervisorError> {
    let decoder = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| HypervisorError::Install(error.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| HypervisorError::Install(error.to_string()))?;
        let path = entry
            .path()
            .map_err(|error| HypervisorError::Install(error.to_string()))?
            .into_owned();
        if path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(HypervisorError::Install(
                "tarball member path is not safe".to_string(),
            ));
        }
        if !is_firecracker_member(&path) {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| HypervisorError::Install(error.to_string()))?;
        return Ok(bytes);
    }
    Err(HypervisorError::MissingFirecrackerMember)
}

fn is_firecracker_member(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("firecracker") && !name.contains("jailer"))
}

fn normalize_sha256(value: &str) -> Result<String, HypervisorError> {
    let token = value.split_whitespace().next().unwrap_or("");
    let stripped = token
        .strip_prefix("sha256:")
        .or_else(|| token.strip_prefix("SHA256:"))
        .unwrap_or(token);
    if stripped.len() != 64 || !stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(HypervisorError::InvalidChecksum);
    }
    Ok(stripped.to_ascii_lowercase())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_capabilities::{
        evaluate, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
    };
    use crate::test_support::{nvidia_gpu, nvidia_gpu_with};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;
    use tiny_http::{Response, Server};

    struct ScriptedFetcher {
        payload: Vec<u8>,
        hits: AtomicU32,
    }

    impl ScriptedFetcher {
        fn new(payload: impl Into<Vec<u8>>) -> Self {
            Self {
                payload: payload.into(),
                hits: AtomicU32::new(0),
            }
        }

        fn hit_count(&self) -> u32 {
            self.hits.load(Ordering::SeqCst)
        }
    }

    impl BinaryFetcher for ScriptedFetcher {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, HypervisorError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Ok(self.payload.clone())
        }
    }

    fn linux_standard() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &[],
        })
    }

    fn linux_no_kvm() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::Missing,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Absent,
            gpus: &[],
        })
    }

    fn macos_caps() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::MacOs,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        })
    }

    fn windows_standard() -> HostCapabilities {
        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Available,
            iommu: IommuStatus::Unknown,
            gpus: &gpus,
        })
    }

    fn windows_no_whpx() -> HostCapabilities {
        evaluate(EvaluateInput {
            os: HostOs::Windows,
            kvm: KvmAccess::NotApplicable,
            whpx: WhpxCapability::Unavailable,
            iommu: IommuStatus::Unknown,
            gpus: &[],
        })
    }

    fn raw_spec(bytes: &[u8], url: &str) -> ArtifactSpec {
        ArtifactSpec {
            name: "firecracker".to_string(),
            version: "v-test".to_string(),
            url: url.to_string(),
            sha256_hex: sha256_hex(bytes),
            kind: ArtifactKind::RawBinary,
        }
    }

    fn gzip_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut encoded = Vec::new();
        {
            let encoder =
                flate2::write::GzEncoder::new(&mut encoded, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            for (name, data) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, *name, *data).unwrap();
            }
            let encoder = builder.into_inner().unwrap();
            encoder.finish().unwrap();
        }
        encoded
    }

    #[test]
    fn macos_skips_without_fetching_firecracker() {
        let cache = tempfile::tempdir().unwrap();
        let payload = b"not-firecracker";
        let fetcher = ScriptedFetcher::new(payload.as_slice());
        let spec = raw_spec(payload, "https://example.invalid/firecracker");
        let status = ensure(
            HostOs::MacOs,
            &macos_caps(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &fetcher,
        )
        .unwrap();

        assert_eq!(status.action, EnsureAction::SkippedOs);
        assert_eq!(status.hypervisor, HypervisorName::None);
        assert!(!status.hypervisor_invoked);
        assert_eq!(fetcher.hit_count(), 0);
        assert!(status.log.contains("Mac"));
        assert!(
            status.log.contains("not fetched")
                || status.log.contains("Not fetched")
                || status.log.contains("is not fetched")
        );
        assert!(!cache.path().join("firecracker").exists());
    }

    #[test]
    fn linux_without_standard_offered_fails_fast_without_fetch() {
        let cache = tempfile::tempdir().unwrap();
        let payload = b"fc";
        let fetcher = ScriptedFetcher::new(payload.as_slice());
        let spec = raw_spec(payload, "https://example.invalid/firecracker");
        let error = ensure(
            HostOs::Linux,
            &linux_no_kvm(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &fetcher,
        )
        .unwrap_err();

        assert!(matches!(error, HypervisorError::StandardNotOffered { .. }));
        assert_eq!(fetcher.hit_count(), 0);
        let status = status_from_result(Err(error));
        assert_eq!(status.action, EnsureAction::Refused);
        assert!(!status.hypervisor_invoked);
    }

    #[test]
    fn linux_fetches_firecracker_when_missing() {
        let cache = tempfile::tempdir().unwrap();
        let payload = b"fake-firecracker-binary";
        let fetcher = ScriptedFetcher::new(payload.as_slice());
        let spec = raw_spec(payload, "https://example.invalid/firecracker");
        let status = ensure(
            HostOs::Linux,
            &linux_standard(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &fetcher,
        )
        .unwrap();

        assert_eq!(status.action, EnsureAction::Fetched);
        assert_eq!(status.hypervisor, HypervisorName::Firecracker);
        assert_eq!(status.version.as_deref(), Some("v-test"));
        assert!(!status.hypervisor_invoked);
        assert_eq!(fetcher.hit_count(), 1);
        let path = PathBuf::from(status.path.as_deref().unwrap());
        assert_eq!(fs::read(&path).unwrap(), payload);
        assert!(status.log.contains("fetched"));
        assert!(status.log.contains("checksum ok"));
    }

    #[test]
    fn linux_skips_when_present_and_checksum_matches() {
        let cache = tempfile::tempdir().unwrap();
        let payload = b"already-installed";
        let spec = raw_spec(payload, "https://example.invalid/firecracker");
        let first = ScriptedFetcher::new(payload.as_slice());
        ensure(
            HostOs::Linux,
            &linux_standard(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &first,
        )
        .unwrap();

        let second = ScriptedFetcher::new(payload.as_slice());
        let status = ensure(
            HostOs::Linux,
            &linux_standard(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &second,
        )
        .unwrap();

        assert_eq!(status.action, EnsureAction::SkippedPresent);
        assert_eq!(second.hit_count(), 0);
        assert!(status.log.contains("skipped because present"));
        assert!(!status.hypervisor_invoked);
    }

    #[test]
    fn linux_checksum_mismatch_fails_fast_and_does_not_install() {
        let cache = tempfile::tempdir().unwrap();
        let payload = b"evil-bytes";
        let fetcher = ScriptedFetcher::new(payload.as_slice());
        let mut spec = raw_spec(b"expected-bytes", "https://example.invalid/firecracker");
        spec.sha256_hex = sha256_hex(b"expected-bytes");
        let error = ensure(
            HostOs::Linux,
            &linux_standard(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &fetcher,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            HypervisorError::ChecksumMismatch { ref name, .. } if name == "firecracker"
        ));
        assert_eq!(fetcher.hit_count(), 1);
        assert!(!cache.path().join("firecracker/v-test/firecracker").exists());
        let status = status_from_result(Err(error));
        assert_eq!(status.action, EnsureAction::ChecksumMismatch);
        assert!(status.log.contains("expected"));
        assert!(!status.hypervisor_invoked);
    }

    #[test]
    fn windows_would_fetch_qemu_does_not_download() {
        let cache = tempfile::tempdir().unwrap();
        let payload = b"qemu-setup";
        let fetcher = ScriptedFetcher::new(payload.as_slice());
        let spec = raw_spec(payload, "https://example.invalid/qemu");
        let status = ensure(
            HostOs::Windows,
            &windows_standard(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &fetcher,
        )
        .unwrap();

        assert_eq!(status.action, EnsureAction::WouldFetch);
        assert_eq!(status.hypervisor, HypervisorName::Qemu);
        assert_eq!(fetcher.hit_count(), 0);
        assert!(status.log.contains("would fetch QEMU"));
        assert!(status.log.contains("19b"));
        assert!(!status.hypervisor_invoked);
        assert!(!cache.path().join("qemu").exists());
        assert!(!cache.path().join("firecracker").exists());
    }

    #[test]
    fn windows_without_standard_offered_fails_fast() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedFetcher::new(b"qemu".as_slice());
        let spec = raw_spec(b"qemu", "https://example.invalid/qemu");
        let error = ensure(
            HostOs::Windows,
            &windows_no_whpx(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &fetcher,
        )
        .unwrap_err();
        assert!(matches!(error, HypervisorError::StandardNotOffered { .. }));
        assert_eq!(fetcher.hit_count(), 0);
    }

    #[test]
    fn linux_and_windows_are_peer_paths_not_a_windows_special_case() {
        let cache = tempfile::tempdir().unwrap();
        let payload = b"fc";
        let linux = ensure(
            HostOs::Linux,
            &linux_standard(),
            cache.path(),
            &raw_spec(payload, "https://example.invalid/fc"),
            QEMU_WINDOWS_INDEX_URL,
            &ScriptedFetcher::new(payload.as_slice()),
        )
        .unwrap();
        let windows = ensure(
            HostOs::Windows,
            &windows_standard(),
            cache.path(),
            &raw_spec(payload, "https://example.invalid/fc"),
            QEMU_WINDOWS_INDEX_URL,
            &ScriptedFetcher::new(payload.as_slice()),
        )
        .unwrap();

        assert_eq!(linux.hypervisor, HypervisorName::Firecracker);
        assert_eq!(windows.hypervisor, HypervisorName::Qemu);
        assert_ne!(linux.action, windows.action);
        assert!(!linux.hypervisor_invoked);
        assert!(!windows.hypervisor_invoked);
    }

    #[test]
    fn gzip_tar_extracts_firecracker_not_jailer() {
        let cache = tempfile::tempdir().unwrap();
        let tarball = gzip_tar(&[
            (
                "release-v-test-x86_64/jailer-v-test-x86_64",
                b"jailer-bytes",
            ),
            (
                "release-v-test-x86_64/firecracker-v-test-x86_64",
                b"real-firecracker",
            ),
        ]);
        let spec = ArtifactSpec {
            name: "firecracker".to_string(),
            version: "v-test".to_string(),
            url: "https://example.invalid/firecracker.tgz".to_string(),
            sha256_hex: sha256_hex(&tarball),
            kind: ArtifactKind::GzipTarFirecracker,
        };
        let status = ensure(
            HostOs::Linux,
            &linux_standard(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &ScriptedFetcher::new(tarball),
        )
        .unwrap();

        let path = PathBuf::from(status.path.unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"real-firecracker");
        assert_eq!(status.action, EnsureAction::Fetched);
        assert!(!status.hypervisor_invoked);
    }

    #[test]
    fn file_url_fetcher_reads_local_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("artifact.bin");
        fs::write(&file, b"from-file").unwrap();
        let url = format!("file://{}", file.display());
        let bytes = HttpBinaryFetcher.fetch(&url).unwrap();
        assert_eq!(bytes, b"from-file");
    }

    #[test]
    fn httptest_loopback_fetch_does_not_use_the_internet() {
        let payload = b"loopback-firecracker";
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().expect("tcp");
        thread::spawn(move || {
            let request = server.recv().unwrap();
            request
                .respond(Response::from_data(payload.as_slice()))
                .unwrap();
        });

        let cache = tempfile::tempdir().unwrap();
        let url = format!("http://{addr}/firecracker");
        let spec = raw_spec(payload, &url);
        let status = ensure(
            HostOs::Linux,
            &linux_standard(),
            cache.path(),
            &spec,
            QEMU_WINDOWS_INDEX_URL,
            &HttpBinaryFetcher,
        )
        .unwrap();
        assert_eq!(status.action, EnsureAction::Fetched);
        assert_eq!(fs::read(status.path.as_deref().unwrap()).unwrap(), payload);
    }

    #[test]
    fn http_fetcher_refuses_non_loopback_in_tests_by_default() {
        if network_env_enabled() {
            return;
        }
        let error = HttpBinaryFetcher
            .fetch("https://example.invalid/firecracker")
            .unwrap_err();
        assert!(matches!(error, HypervisorError::NetworkDisabled { .. }));
    }

    #[test]
    fn documented_firecracker_url_is_github_release_not_a_secret() {
        let spec = firecracker_spec("x86_64", |_| None).unwrap();
        assert_eq!(
            spec.url,
            format!("{FIRECRACKER_RELEASE_BASE}/firecracker-{FIRECRACKER_VERSION}-x86_64.tgz")
        );
        assert_eq!(spec.sha256_hex, FIRECRACKER_SHA256_X86_64);
        assert_eq!(spec.kind, ArtifactKind::GzipTarFirecracker);
        let arm = firecracker_spec("aarch64", |_| None).unwrap();
        assert!(arm.url.ends_with("aarch64.tgz"));
        assert_eq!(arm.sha256_hex, FIRECRACKER_SHA256_AARCH64);
        assert!(matches!(
            firecracker_spec("riscv64", |_| None),
            Err(HypervisorError::UnsupportedArch { .. })
        ));
    }

    #[test]
    fn env_overrides_firecracker_url_checksum_and_kind() {
        let bytes = b"override";
        let spec = firecracker_spec("x86_64", |key| match key {
            FC_URL_ENV => Some("file:///tmp/fc".to_string()),
            FC_SHA_ENV => Some(sha256_hex(bytes)),
            FC_KIND_ENV => Some("raw".to_string()),
            FC_VERSION_ENV => Some("v-override".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(spec.url, "file:///tmp/fc");
        assert_eq!(spec.kind, ArtifactKind::RawBinary);
        assert_eq!(spec.version, "v-override");
        assert_eq!(spec.sha256_hex, sha256_hex(bytes));
    }

    #[test]
    fn cache_env_overrides_default_dir() {
        let dir =
            resolve_cache_dir(|key| (key == CACHE_ENV).then(|| "/tmp/dcl-hv-cache".to_string()));
        assert_eq!(dir, PathBuf::from("/tmp/dcl-hv-cache"));
    }

    #[test]
    fn ensure_never_sets_hypervisor_invoked() {
        let cache = tempfile::tempdir().unwrap();
        let payload = b"fc";
        let cases = [
            ensure(
                HostOs::MacOs,
                &macos_caps(),
                cache.path(),
                &raw_spec(payload, "https://example.invalid/fc"),
                QEMU_WINDOWS_INDEX_URL,
                &ScriptedFetcher::new(payload.as_slice()),
            )
            .unwrap(),
            ensure(
                HostOs::Linux,
                &linux_standard(),
                cache.path(),
                &raw_spec(payload, "https://example.invalid/fc"),
                QEMU_WINDOWS_INDEX_URL,
                &ScriptedFetcher::new(payload.as_slice()),
            )
            .unwrap(),
            ensure(
                HostOs::Windows,
                &windows_standard(),
                cache.path(),
                &raw_spec(payload, "https://example.invalid/fc"),
                QEMU_WINDOWS_INDEX_URL,
                &ScriptedFetcher::new(payload.as_slice()),
            )
            .unwrap(),
        ];
        for status in cases {
            assert!(!status.hypervisor_invoked);
        }
    }
}
