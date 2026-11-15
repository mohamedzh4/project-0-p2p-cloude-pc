//! Guest base-image digest + Ed25519 signature checks, plus locate/fetch.
//!
//! The platform signs the raw 32-byte SHA-256 of the image with an Ed25519
//! key. The agent pins the **public** key (`IMAGE_SIGNING_PUBLIC_KEY`) and
//! compares the advertised digest to the standby image it already has
//! (`IMAGE_LOCAL_DIGEST`, or a SHA-256 of provided bytes). Mismatch refuses
//! the stub boot path.
//!
//! Phase 20 locates or fetches the signed base image for the probed backend
//! into a user cache, then runs the same SHA-256 + Ed25519 checks **before**
//! treating the file as ready. Linux Standard fetches the QEMU+KVM guest
//! image. Linux Pro (when offered) fetches the Pro image. Windows Standard
//! fetches the QEMU disk the same way (SHA-256 + Ed25519) so `-accel whpx`
//! has a verified disk. Windows Pro is not fetched here (Hyper-V GPU). macOS
//! skips with a reason — it does not pretend KVM/WHPX images run. The
//! webview does not fetch. This module never holds a private key and never
//! boots a guest. Tests inject a fetcher, `file://`, or loopback httptest.

use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::host_capabilities::HostCapabilities;
use crate::provision::{HostOs, LabTier};

const PUBLIC_KEY_ENV: &str = "IMAGE_SIGNING_PUBLIC_KEY";
const LOCAL_DIGEST_ENV: &str = "IMAGE_LOCAL_DIGEST";
const STANDARD_DIGEST_ENV: &str = "IMAGE_STANDARD_DIGEST";
const STANDARD_SIG_ENV: &str = "IMAGE_STANDARD_SIGNATURE";
const PRO_DIGEST_ENV: &str = "IMAGE_PRO_DIGEST";
const PRO_SIG_ENV: &str = "IMAGE_PRO_SIGNATURE";
const CACHE_ENV: &str = "HOST_AGENT_IMAGE_CACHE";
const STANDARD_URL_ENV: &str = "HOST_AGENT_STANDARD_IMAGE_URL";
const PRO_URL_ENV: &str = "HOST_AGENT_PRO_IMAGE_URL";
const NETWORK_ENV: &str = "HOST_AGENT_IMAGE_NETWORK";
const MAX_BYTES_ENV: &str = "HOST_AGENT_IMAGE_MAX_BYTES";
const DEFAULT_MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn parse_hex(value: &str) -> Result<Self, ImageError> {
        let stripped = strip_sha256_prefix(value.trim());
        let decoded = hex::decode(stripped).map_err(|_| ImageError::InvalidDigest)?;
        let bytes: [u8; 32] = decoded.try_into().map_err(|_| ImageError::InvalidDigest)?;
        Ok(Self(bytes))
    }

    pub fn of(image_bytes: &[u8]) -> Self {
        let digest = Sha256::digest(image_bytes);
        Self(digest.into())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Sha256Digest").field(&self.to_hex()).finish()
    }
}

/// Advertised by the Matchmaker for a resource class. Digest is 64 hex chars;
/// signature is standard base64 of the 64-byte Ed25519 signature over the
/// raw 32-byte digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageManifest {
    pub digest: Sha256Digest,
    pub signature: Signature,
}

impl ImageManifest {
    pub fn parse(digest_hex: &str, signature_b64: &str) -> Result<Self, ImageError> {
        let digest = Sha256Digest::parse_hex(digest_hex)?;
        let signature = parse_signature(signature_b64)?;
        Ok(Self { digest, signature })
    }

    pub fn digest_hex(&self) -> String {
        self.digest.to_hex()
    }

    pub fn signature_base64(&self) -> String {
        BASE64.encode(self.signature.to_bytes())
    }
}

/// Pinned platform public key plus the digest of the image this host has on
/// standby. Built from env when the provision listener is enabled.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageTrust {
    public_key: VerifyingKey,
    local_digest: Sha256Digest,
}

impl ImageTrust {
    pub fn new(public_key: VerifyingKey, local_digest: Sha256Digest) -> Self {
        Self {
            public_key,
            local_digest,
        }
    }

    pub fn local_digest(&self) -> Sha256Digest {
        self.local_digest
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key.as_bytes())
    }

    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.public_key
    }
}

impl fmt::Debug for ImageTrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageTrust")
            .field("public_key", &self.public_key_hex())
            .field("local_digest", &self.local_digest.to_hex())
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageError {
    #[error("{PUBLIC_KEY_ENV} is required")]
    MissingPublicKey,
    #[error("{PUBLIC_KEY_ENV} must be a 32-byte Ed25519 public key (64 hex chars)")]
    InvalidPublicKey,
    #[error("{LOCAL_DIGEST_ENV} is required")]
    MissingLocalDigest,
    #[error("{LOCAL_DIGEST_ENV} must be a 64-character SHA-256 hex digest")]
    InvalidLocalDigest,
    #[error("image digest must be a 64-character SHA-256 hex digest")]
    InvalidDigest,
    #[error("image signature must be standard base64 of a 64-byte Ed25519 signature")]
    InvalidSignature,
    #[error("image digest does not match the signed standby image")]
    DigestMismatch,
    #[error("image signature does not verify with the platform public key")]
    SignatureMismatch,
    #[error("Standard sessions are not offered; not fetching a guest image ({reason})")]
    StandardNotOffered { reason: String },
    #[error("{STANDARD_SIG_ENV} is required to fetch a signed guest image")]
    MissingStandardSignature,
    #[error("{PRO_DIGEST_ENV} and {PRO_SIG_ENV} are required to fetch the Pro guest image")]
    MissingProMetadata,
    #[error("{url_env} is required to fetch the {lab_tier} guest image")]
    MissingImageUrl {
        url_env: &'static str,
        lab_tier: String,
    },
    #[error("could not fetch guest image from {url}: {detail}")]
    FetchFailed { url: String, detail: String },
    #[error(
        "non-loopback guest-image fetch is disabled in tests; set {NETWORK_ENV}=1 to allow {url}"
    )]
    NetworkDisabled { url: String },
    #[error("could not store the guest image: {0}")]
    Cache(String),
    #[error("{MAX_BYTES_ENV} must be a positive byte count")]
    InvalidMaxBytes,
}

/// Load the pinned public key and local standby digest. Fail fast: no in-code
/// fallback and no hardcoded key.
pub fn trust_from_env<F>(mut lookup: F) -> Result<ImageTrust, ImageError>
where
    F: FnMut(&str) -> Option<String>,
{
    let public_raw = nonempty(lookup(PUBLIC_KEY_ENV)).ok_or(ImageError::MissingPublicKey)?;
    let public_key = parse_public_key(&public_raw)?;
    let digest_raw = nonempty(lookup(LOCAL_DIGEST_ENV)).ok_or(ImageError::MissingLocalDigest)?;
    let local_digest =
        Sha256Digest::parse_hex(&digest_raw).map_err(|_| ImageError::InvalidLocalDigest)?;
    Ok(ImageTrust::new(public_key, local_digest))
}

/// Verify advertised digest+signature, then that local standby (or provided
/// bytes) hashes to that digest. Call this **before** [`crate::provision::plan`].
pub fn verify_image(
    trust: &ImageTrust,
    advertised: &ImageManifest,
    image_bytes: Option<&[u8]>,
) -> Result<(), ImageError> {
    if let Some(bytes) = image_bytes {
        let computed = Sha256Digest::of(bytes);
        if !ct_eq(computed.as_bytes(), advertised.digest.as_bytes()) {
            return Err(ImageError::DigestMismatch);
        }
    }
    if !ct_eq(trust.local_digest.as_bytes(), advertised.digest.as_bytes()) {
        return Err(ImageError::DigestMismatch);
    }
    trust
        .public_key
        .verify(advertised.digest.as_bytes(), &advertised.signature)
        .map_err(|_| ImageError::SignatureMismatch)
}

/// SHA-256 of `image_bytes` plus Ed25519 over the raw digest. Used after a
/// fetch **before** the file is treated as ready. Unlike [`verify_image`],
/// this does not require [`ImageTrust::local_digest`] (Pro images differ from
/// the Standard standby pin).
pub fn verify_fetched_bytes(
    public_key: &VerifyingKey,
    advertised: &ImageManifest,
    image_bytes: &[u8],
) -> Result<(), ImageError> {
    verify_fetched_digest(public_key, advertised, Sha256Digest::of(image_bytes), None)
}

fn verify_fetched_digest(
    public_key: &VerifyingKey,
    advertised: &ImageManifest,
    computed: Sha256Digest,
    local_pin: Option<Sha256Digest>,
) -> Result<(), ImageError> {
    if !ct_eq(computed.as_bytes(), advertised.digest.as_bytes()) {
        return Err(ImageError::DigestMismatch);
    }
    if let Some(pin) = local_pin {
        if !ct_eq(pin.as_bytes(), advertised.digest.as_bytes()) {
            return Err(ImageError::DigestMismatch);
        }
    }
    public_key
        .verify(advertised.digest.as_bytes(), &advertised.signature)
        .map_err(|_| ImageError::SignatureMismatch)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageEnsureAction {
    SkippedPresent,
    Fetched,
    SkippedOs,
    WouldFetch,
    Refused,
    Rejected,
    FetchFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageBackend {
    Qemu,
    None,
}

/// Advertised Matchmaker `IMAGE_*` checksums plus the bytes URL for one lab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestImageSpec {
    pub lab_tier: LabTier,
    pub url: String,
    pub manifest: ImageManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedImageManifest {
    lab_tier: String,
    digest_sha256: String,
    signature: String,
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestImageStatus {
    pub action: ImageEnsureAction,
    pub lab_tier: Option<String>,
    pub backend: ImageBackend,
    pub path: Option<String>,
    pub digest: Option<String>,
    pub log: String,
    pub image_booted: bool,
}

impl GuestImageStatus {
    fn success(
        action: ImageEnsureAction,
        lab_tier: Option<String>,
        backend: ImageBackend,
        path: Option<PathBuf>,
        digest: Option<String>,
        log: String,
    ) -> Self {
        Self {
            action,
            lab_tier,
            backend,
            path: path.map(|p| p.to_string_lossy().into_owned()),
            digest,
            log,
            image_booted: false,
        }
    }
}

pub trait ImageFetcher {
    fn fetch_into(&self, url: &str, dest: &Path) -> Result<(), ImageError>;
}

/// Production/HTTP fetcher. `file://` is always allowed. Loopback HTTP is
/// allowed in tests (httptest). Other HTTP is refused under `cfg(test)` unless
/// [`NETWORK_ENV`] is `1`.
pub struct HttpImageFetcher {
    pub max_bytes: u64,
}

impl Default for HttpImageFetcher {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_IMAGE_BYTES,
        }
    }
}

impl ImageFetcher for HttpImageFetcher {
    fn fetch_into(&self, url: &str, dest: &Path) -> Result<(), ImageError> {
        if let Some(path) = file_url_path(url) {
            return copy_limited_file(&path, dest, url, self.max_bytes);
        }
        if url.starts_with("http://") || url.starts_with("https://") {
            if !http_fetch_permitted(url) {
                return Err(ImageError::NetworkDisabled {
                    url: url.to_string(),
                });
            }
            return http_get_into(url, dest, self.max_bytes);
        }
        Err(ImageError::FetchFailed {
            url: url.to_string(),
            detail: "URL must be http(s) or file://".to_string(),
        })
    }
}

pub fn image_network_env_enabled() -> bool {
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
        image_network_env_enabled()
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

fn copy_limited_file(src: &Path, dest: &Path, url: &str, max_bytes: u64) -> Result<(), ImageError> {
    let file = fs::File::open(src).map_err(|error| ImageError::FetchFailed {
        url: url.to_string(),
        detail: error.to_string(),
    })?;
    write_limited(file, dest, url, max_bytes)
}

fn http_get_into(url: &str, dest: &Path, max_bytes: u64) -> Result<(), ImageError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    match agent.get(url).call() {
        Ok(response) => write_limited(response.into_reader(), dest, url, max_bytes),
        Err(ureq::Error::Status(status, response)) => {
            let detail = response
                .into_string()
                .unwrap_or_else(|_| format!("HTTP {status}"));
            Err(ImageError::FetchFailed {
                url: url.to_string(),
                detail: format!("HTTP {status}: {detail}"),
            })
        }
        Err(error) => Err(ImageError::FetchFailed {
            url: url.to_string(),
            detail: error.to_string(),
        }),
    }
}

fn write_limited(
    mut reader: impl Read,
    dest: &Path,
    url: &str,
    max_bytes: u64,
) -> Result<(), ImageError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|error| ImageError::Cache(error.to_string()))?;
    }
    let mut file = fs::File::create(dest).map_err(|error| ImageError::Cache(error.to_string()))?;
    let mut written: u64 = 0;
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|error| ImageError::FetchFailed {
                url: url.to_string(),
                detail: error.to_string(),
            })?;
        if n == 0 {
            break;
        }
        written = written.saturating_add(n as u64);
        if written > max_bytes {
            drop(file);
            let _ = fs::remove_file(dest);
            return Err(ImageError::FetchFailed {
                url: url.to_string(),
                detail: format!("artifact larger than {max_bytes} bytes"),
            });
        }
        file.write_all(&chunk[..n])
            .map_err(|error| ImageError::Cache(error.to_string()))?;
    }
    Ok(())
}

pub fn resolve_image_cache_dir(getenv: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(dir) = getenv(CACHE_ENV) {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_image_cache_dir()
}

fn default_image_cache_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("dcl-host-agent").join("images");
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
    base.join("dcl-host-agent").join("images")
}

pub fn max_image_bytes(getenv: impl Fn(&str) -> Option<String>) -> Result<u64, ImageError> {
    let Some(raw) = getenv(MAX_BYTES_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(DEFAULT_MAX_IMAGE_BYTES);
    };
    let parsed = raw
        .parse::<u64>()
        .map_err(|_| ImageError::InvalidMaxBytes)?;
    if parsed == 0 {
        return Err(ImageError::InvalidMaxBytes);
    }
    Ok(parsed)
}

/// Locate or fetch the OS-appropriate signed guest image. Never boots it.
pub fn ensure(
    os: HostOs,
    caps: &HostCapabilities,
    trust: Option<&ImageTrust>,
    cache_dir: &Path,
    standard: Option<&GuestImageSpec>,
    pro: Option<&GuestImageSpec>,
    fetcher: &dyn ImageFetcher,
) -> Result<GuestImageStatus, ImageError> {
    match os {
        HostOs::MacOs | HostOs::Other => Ok(skip_unsupported(os, caps)),
        HostOs::Linux => ensure_linux(caps, trust, cache_dir, standard, pro, fetcher),
        HostOs::Windows => ensure_windows(caps, trust, cache_dir, standard, pro, fetcher),
    }
}

pub fn ensure_from_env(
    os: HostOs,
    caps: &HostCapabilities,
    getenv: impl Fn(&str) -> Option<String>,
    fetcher: &dyn ImageFetcher,
) -> Result<GuestImageStatus, ImageError> {
    let cache = resolve_image_cache_dir(&getenv);
    match os {
        HostOs::MacOs | HostOs::Other => Ok(skip_unsupported(os, caps)),
        HostOs::Windows => {
            if !caps.standard.offered {
                return Err(ImageError::StandardNotOffered {
                    reason: caps.standard.reason.clone(),
                });
            }
            let trust = trust_from_env(&getenv)?;
            let standard = required_spec(
                LabTier::Standard,
                STANDARD_URL_ENV,
                STANDARD_DIGEST_ENV,
                STANDARD_SIG_ENV,
                Some(trust.local_digest()),
                &getenv,
            )?;
            ensure(
                os,
                caps,
                Some(&trust),
                &cache,
                Some(&standard),
                None,
                fetcher,
            )
        }
        HostOs::Linux => {
            if !caps.standard.offered {
                return Err(ImageError::StandardNotOffered {
                    reason: caps.standard.reason.clone(),
                });
            }
            let trust = trust_from_env(&getenv)?;
            let standard = required_spec(
                LabTier::Standard,
                STANDARD_URL_ENV,
                STANDARD_DIGEST_ENV,
                STANDARD_SIG_ENV,
                Some(trust.local_digest()),
                &getenv,
            )?;
            let pro = if caps.pro.offered {
                Some(required_spec(
                    LabTier::Pro,
                    PRO_URL_ENV,
                    PRO_DIGEST_ENV,
                    PRO_SIG_ENV,
                    None,
                    &getenv,
                )?)
            } else {
                None
            };
            ensure(
                os,
                caps,
                Some(&trust),
                &cache,
                Some(&standard),
                pro.as_ref(),
                fetcher,
            )
        }
    }
}

pub fn status_from_result(result: Result<GuestImageStatus, ImageError>) -> GuestImageStatus {
    match result {
        Ok(status) => {
            debug_assert!(!status.image_booted);
            status
        }
        Err(ImageError::StandardNotOffered { reason }) => GuestImageStatus::success(
            ImageEnsureAction::Refused,
            None,
            ImageBackend::None,
            None,
            None,
            format!("refused: Standard is not offered; not fetching a guest image. {reason}"),
        ),
        Err(error @ ImageError::DigestMismatch) | Err(error @ ImageError::SignatureMismatch) => {
            GuestImageStatus::success(
                ImageEnsureAction::Rejected,
                None,
                ImageBackend::None,
                None,
                None,
                format!("rejected: {error}"),
            )
        }
        Err(error) => GuestImageStatus::success(
            ImageEnsureAction::FetchFailed,
            None,
            ImageBackend::None,
            None,
            None,
            error.to_string(),
        ),
    }
}

fn skip_unsupported(os: HostOs, caps: &HostCapabilities) -> GuestImageStatus {
    let log = match os {
        HostOs::MacOs => format!(
            "skipped: this Mac cannot host; KVM/WHPX guest images are not fetched. {}",
            caps.standard.reason
        ),
        _ => format!(
            "skipped: this OS cannot host; no guest image fetched. {}",
            caps.standard.reason
        ),
    };
    GuestImageStatus::success(
        ImageEnsureAction::SkippedOs,
        None,
        ImageBackend::None,
        None,
        None,
        log,
    )
}

fn ensure_linux(
    caps: &HostCapabilities,
    trust: Option<&ImageTrust>,
    cache_dir: &Path,
    standard: Option<&GuestImageSpec>,
    pro: Option<&GuestImageSpec>,
    fetcher: &dyn ImageFetcher,
) -> Result<GuestImageStatus, ImageError> {
    if !caps.standard.offered {
        return Err(ImageError::StandardNotOffered {
            reason: caps.standard.reason.clone(),
        });
    }
    let trust = trust.ok_or(ImageError::MissingPublicKey)?;
    let standard = standard.ok_or(ImageError::MissingImageUrl {
        url_env: STANDARD_URL_ENV,
        lab_tier: LabTier::Standard.as_str().to_string(),
    })?;
    let mut status = locate_or_fetch_image(
        cache_dir,
        standard,
        trust.verifying_key(),
        Some(trust.local_digest()),
        ImageBackend::Qemu,
        fetcher,
    )?;

    if caps.pro.offered {
        let pro = pro.ok_or(ImageError::MissingImageUrl {
            url_env: PRO_URL_ENV,
            lab_tier: LabTier::Pro.as_str().to_string(),
        })?;
        let pro_status = locate_or_fetch_image(
            cache_dir,
            pro,
            trust.verifying_key(),
            None,
            ImageBackend::Qemu,
            fetcher,
        )?;
        status.log = format!("{}; {}", status.log.trim_end_matches('.'), pro_status.log);
    }
    Ok(status)
}

fn ensure_windows(
    caps: &HostCapabilities,
    trust: Option<&ImageTrust>,
    cache_dir: &Path,
    standard: Option<&GuestImageSpec>,
    pro: Option<&GuestImageSpec>,
    fetcher: &dyn ImageFetcher,
) -> Result<GuestImageStatus, ImageError> {
    if !caps.standard.offered {
        return Err(ImageError::StandardNotOffered {
            reason: caps.standard.reason.clone(),
        });
    }
    let trust = trust.ok_or(ImageError::MissingPublicKey)?;
    let standard = standard.ok_or(ImageError::MissingImageUrl {
        url_env: STANDARD_URL_ENV,
        lab_tier: LabTier::Standard.as_str().to_string(),
    })?;
    let mut status = locate_or_fetch_image(
        cache_dir,
        standard,
        trust.verifying_key(),
        Some(trust.local_digest()),
        ImageBackend::Qemu,
        fetcher,
    )?;

    if caps.pro.offered {
        let listed = pro
            .map(|spec| spec.url.as_str())
            .filter(|url| !url.is_empty())
            .unwrap_or("Matchmaker IMAGE_PRO_* metadata");
        status.log = format!(
            "{}; would fetch Pro guest image from {listed}; not fetched (Windows Pro is Hyper-V GPU, not QEMU+WHPX)",
            status.log.trim_end_matches('.')
        );
    }
    Ok(status)
}

fn locate_or_fetch_image(
    cache_dir: &Path,
    spec: &GuestImageSpec,
    public_key: &VerifyingKey,
    local_pin: Option<Sha256Digest>,
    backend: ImageBackend,
    fetcher: &dyn ImageFetcher,
) -> Result<GuestImageStatus, ImageError> {
    let dest = cached_rootfs_path(cache_dir, spec.lab_tier);
    let manifest_path = dest.with_extension("manifest.json");

    if let Some(status) =
        skip_if_present(&dest, &manifest_path, spec, public_key, local_pin, backend)
    {
        return Ok(status);
    }

    let tmp = dest.with_extension("img.tmp");
    if let Err(error) = fetcher.fetch_into(&spec.url, &tmp) {
        remove_image(&dest, &manifest_path, &tmp);
        return Err(error);
    }

    let computed = match sha256_file(&tmp) {
        Ok(digest) => digest,
        Err(error) => {
            remove_image(&dest, &manifest_path, &tmp);
            return Err(error);
        }
    };
    if let Err(error) = verify_fetched_digest(public_key, &spec.manifest, computed, local_pin) {
        remove_image(&dest, &manifest_path, &tmp);
        return Err(error);
    }

    if let Some(parent) = dest.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            remove_image(&dest, &manifest_path, &tmp);
            return Err(ImageError::Cache(error.to_string()));
        }
    }
    if let Err(error) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Err(ImageError::Cache(error.to_string()));
    }
    if let Err(error) = write_cached_manifest(
        &manifest_path,
        &CachedImageManifest {
            lab_tier: spec.lab_tier.as_str().to_string(),
            digest_sha256: spec.manifest.digest_hex(),
            signature: spec.manifest.signature_base64(),
            url: spec.url.clone(),
        },
    ) {
        remove_image(&dest, &manifest_path, &tmp);
        return Err(error);
    }

    let bytes = fs::metadata(&dest).map(|meta| meta.len()).unwrap_or(0);
    Ok(GuestImageStatus::success(
        ImageEnsureAction::Fetched,
        Some(spec.lab_tier.as_str().to_string()),
        backend,
        Some(dest.clone()),
        Some(spec.manifest.digest_hex()),
        format!(
            "fetched {} {} guest image ({} bytes) to {}; digest and Ed25519 ok",
            spec.lab_tier.as_str(),
            backend_label(backend),
            bytes,
            dest.display()
        ),
    ))
}

fn skip_if_present(
    dest: &Path,
    manifest_path: &Path,
    spec: &GuestImageSpec,
    public_key: &VerifyingKey,
    local_pin: Option<Sha256Digest>,
    backend: ImageBackend,
) -> Option<GuestImageStatus> {
    let computed = sha256_file(dest).ok()?;
    let raw = fs::read_to_string(manifest_path).ok()?;
    let cached = serde_json::from_str::<CachedImageManifest>(&raw).ok()?;
    if cached.digest_sha256 != spec.manifest.digest_hex() {
        return None;
    }
    if verify_fetched_digest(public_key, &spec.manifest, computed, local_pin).is_err() {
        return None;
    }
    Some(GuestImageStatus::success(
        ImageEnsureAction::SkippedPresent,
        Some(spec.lab_tier.as_str().to_string()),
        backend,
        Some(dest.to_path_buf()),
        Some(spec.manifest.digest_hex()),
        format!(
            "skipped because present; {} {} guest image at {} (sha256 {})",
            spec.lab_tier.as_str(),
            backend_label(backend),
            dest.display(),
            spec.manifest.digest_hex()
        ),
    ))
}

/// Phase-20 cache layout: `{cache}/{labTier}/rootfs.img`.
pub fn cached_rootfs_path(cache_dir: &Path, lab_tier: LabTier) -> PathBuf {
    cache_dir.join(lab_tier.as_str()).join("rootfs.img")
}

fn write_cached_manifest(path: &Path, manifest: &CachedImageManifest) -> Result<(), ImageError> {
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|error| ImageError::Cache(error.to_string()))?;
    fs::write(path, json).map_err(|error| ImageError::Cache(error.to_string()))
}

fn remove_image(dest: &Path, manifest: &Path, tmp: &Path) {
    let _ = fs::remove_file(dest);
    let _ = fs::remove_file(manifest);
    let _ = fs::remove_file(tmp);
}

fn sha256_file(path: &Path) -> Result<Sha256Digest, ImageError> {
    let mut file = fs::File::open(path).map_err(|error| ImageError::Cache(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = file
            .read(&mut chunk)
            .map_err(|error| ImageError::Cache(error.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    Ok(Sha256Digest(hasher.finalize().into()))
}

fn backend_label(backend: ImageBackend) -> &'static str {
    match backend {
        ImageBackend::Qemu => "QEMU",
        ImageBackend::None => "guest",
    }
}

fn required_spec(
    lab_tier: LabTier,
    url_env: &'static str,
    digest_env: &'static str,
    sig_env: &'static str,
    digest_fallback: Option<Sha256Digest>,
    getenv: impl Fn(&str) -> Option<String>,
) -> Result<GuestImageSpec, ImageError> {
    let url = nonempty(getenv(url_env)).ok_or(ImageError::MissingImageUrl {
        url_env,
        lab_tier: lab_tier.as_str().to_string(),
    })?;
    let digest_raw = nonempty(getenv(digest_env)).or_else(|| digest_fallback.map(|d| d.to_hex()));
    let Some(digest_raw) = digest_raw else {
        return Err(if lab_tier == LabTier::Pro {
            ImageError::MissingProMetadata
        } else {
            ImageError::MissingLocalDigest
        });
    };
    let sig_raw = nonempty(getenv(sig_env)).ok_or(if lab_tier == LabTier::Pro {
        ImageError::MissingProMetadata
    } else {
        ImageError::MissingStandardSignature
    })?;
    let manifest = ImageManifest::parse(&digest_raw, &sig_raw)?;
    if let Some(pin) = digest_fallback {
        if !ct_eq(pin.as_bytes(), manifest.digest.as_bytes()) {
            return Err(ImageError::DigestMismatch);
        }
    }
    Ok(GuestImageSpec {
        lab_tier,
        url,
        manifest,
    })
}

fn parse_public_key(value: &str) -> Result<VerifyingKey, ImageError> {
    let stripped = value.trim();
    let bytes = if let Ok(decoded) = hex::decode(stripped) {
        decoded
    } else {
        BASE64
            .decode(stripped.as_bytes())
            .map_err(|_| ImageError::InvalidPublicKey)?
    };
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|_| ImageError::InvalidPublicKey)?;
    VerifyingKey::from_bytes(&key_bytes).map_err(|_| ImageError::InvalidPublicKey)
}

fn parse_signature(value: &str) -> Result<Signature, ImageError> {
    let stripped = value.trim();
    let decoded = BASE64
        .decode(stripped.as_bytes())
        .map_err(|_| ImageError::InvalidSignature)?;
    let bytes: [u8; 64] = decoded
        .try_into()
        .map_err(|_| ImageError::InvalidSignature)?;
    Ok(Signature::from_bytes(&bytes))
}

fn strip_sha256_prefix(value: &str) -> &str {
    value
        .strip_prefix("sha256:")
        .or_else(|| value.strip_prefix("SHA256:"))
        .unwrap_or(value)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn ct_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::host_capabilities::{
        evaluate, EvaluateInput, IommuStatus, KvmAccess, WhpxCapability,
    };
    use crate::test_support::{nvidia_gpu, nvidia_gpu_with};
    use ed25519_dalek::{Signer, SigningKey};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;
    use tiny_http::{Response, Server};

    /// SHA-256 of these bytes is the Standard Lab `.env.example` digest.
    /// Not a real guest disk — CI must not fetch multi-GB images.
    pub(crate) const FIXTURE_STANDARD_BYTES: &[u8] = b"dcl-fixture-standard-v1\n";
    /// Pro Lab fixture bytes. Same software-contract stand-in.
    pub(crate) const FIXTURE_PRO_BYTES: &[u8] = b"dcl-fixture-pro-v1\n";

    /// 32-byte test seed. Not a production platform key. Do not ship as a secret.
    pub(crate) const TEST_SIGNING_SEED: [u8; 32] = *b"test-ed25519-image-key-seed-v001";

    pub(crate) fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&TEST_SIGNING_SEED)
    }

    pub(crate) fn signed_fixture(bytes: &[u8]) -> (ImageTrust, ImageManifest) {
        let signing_key = test_signing_key();
        let digest = Sha256Digest::of(bytes);
        let signature = signing_key.sign(digest.as_bytes());
        let trust = ImageTrust::new(signing_key.verifying_key(), digest);
        (trust, ImageManifest { digest, signature })
    }

    #[test]
    fn valid_signature_accepts_matching_bytes() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        assert_eq!(
            verify_image(&trust, &manifest, Some(FIXTURE_STANDARD_BYTES)),
            Ok(())
        );
        assert_eq!(verify_image(&trust, &manifest, None), Ok(()));
    }

    #[test]
    fn tampered_bytes_are_rejected() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let mut tampered = FIXTURE_STANDARD_BYTES.to_vec();
        tampered[0] ^= 0x01;
        assert_eq!(
            verify_image(&trust, &manifest, Some(&tampered)),
            Err(ImageError::DigestMismatch)
        );
    }

    #[test]
    fn advertised_digest_must_match_local_standby() {
        let (trust, mut manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        manifest.digest = Sha256Digest::of(FIXTURE_PRO_BYTES);
        assert_eq!(
            verify_image(&trust, &manifest, None),
            Err(ImageError::DigestMismatch)
        );
    }

    #[test]
    fn signature_over_tampered_digest_is_rejected() {
        let (trust, mut manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let other = signed_fixture(FIXTURE_PRO_BYTES);
        // Same local digest, but a signature that belongs to a different digest.
        manifest.signature = other.1.signature;
        assert_eq!(
            verify_image(&trust, &manifest, Some(FIXTURE_STANDARD_BYTES)),
            Err(ImageError::SignatureMismatch)
        );
    }

    #[test]
    fn missing_public_key_fails_fast() {
        let error = trust_from_env(|key| match key {
            LOCAL_DIGEST_ENV => Some(Sha256Digest::of(FIXTURE_STANDARD_BYTES).to_hex()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, ImageError::MissingPublicKey);
        assert!(error.to_string().contains(PUBLIC_KEY_ENV));
    }

    #[test]
    fn missing_local_digest_fails_fast() {
        let (trust, _) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let error = trust_from_env(|key| match key {
            PUBLIC_KEY_ENV => Some(trust.public_key_hex()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, ImageError::MissingLocalDigest);
        assert!(error.to_string().contains(LOCAL_DIGEST_ENV));
    }

    #[test]
    fn invalid_public_key_fails_fast() {
        let error = trust_from_env(|key| match key {
            PUBLIC_KEY_ENV => Some("not-a-key".into()),
            LOCAL_DIGEST_ENV => Some(Sha256Digest::of(FIXTURE_STANDARD_BYTES).to_hex()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, ImageError::InvalidPublicKey);
    }

    #[test]
    fn trust_from_env_accepts_hex_public_key() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let loaded = trust_from_env(|key| match key {
            PUBLIC_KEY_ENV => Some(trust.public_key_hex()),
            LOCAL_DIGEST_ENV => Some(trust.local_digest().to_hex()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            verify_image(&loaded, &manifest, Some(FIXTURE_STANDARD_BYTES)),
            Ok(())
        );
    }

    #[test]
    fn sha256_prefix_is_accepted() {
        let digest = Sha256Digest::of(FIXTURE_STANDARD_BYTES);
        let parsed = Sha256Digest::parse_hex(&format!("sha256:{}", digest.to_hex())).unwrap();
        assert_eq!(parsed, digest);
    }

    #[test]
    fn fixture_digests_are_stable() {
        assert_eq!(
            Sha256Digest::of(FIXTURE_STANDARD_BYTES).to_hex(),
            "6ed707af13eb04a928001996ae5fc7a49ab405e73534caacf86ad65c51bab8fe"
        );
        assert_eq!(
            Sha256Digest::of(FIXTURE_PRO_BYTES).to_hex(),
            "b5dc04d095774efc8fee56dfcbabbe7346788f29a7576aaa27248d9b8249b118"
        );
    }

    /// Local-dev / pytest fixtures. Seed is test-only; never a production key.
    #[test]
    fn local_dev_image_vectors_are_stable() {
        let signing_key = test_signing_key();
        let public_hex = hex::encode(signing_key.verifying_key().as_bytes());
        let standard = Sha256Digest::of(FIXTURE_STANDARD_BYTES);
        let pro = Sha256Digest::of(FIXTURE_PRO_BYTES);
        let standard_sig = BASE64.encode(signing_key.sign(standard.as_bytes()).to_bytes());
        let pro_sig = BASE64.encode(signing_key.sign(pro.as_bytes()).to_bytes());
        eprintln!("IMAGE_SIGNING_PUBLIC_KEY={public_hex}");
        eprintln!("IMAGE_STANDARD_DIGEST={}", standard.to_hex());
        eprintln!("IMAGE_STANDARD_SIGNATURE={standard_sig}");
        eprintln!("IMAGE_PRO_DIGEST={}", pro.to_hex());
        eprintln!("IMAGE_PRO_SIGNATURE={pro_sig}");
        assert_eq!(public_hex.len(), 64);
        assert_eq!(standard.to_hex().len(), 64);
        assert_eq!(pro.to_hex().len(), 64);
        assert_eq!(standard_sig.len(), 88);
        assert_eq!(pro_sig.len(), 88);
        assert_eq!(
            standard.to_hex(),
            "6ed707af13eb04a928001996ae5fc7a49ab405e73534caacf86ad65c51bab8fe"
        );
        assert_eq!(
            pro.to_hex(),
            "b5dc04d095774efc8fee56dfcbabbe7346788f29a7576aaa27248d9b8249b118"
        );
        assert_eq!(
            public_hex,
            "7dadc44f5a3f883a46746bfdade30924db20fe6b43c8469733cfb007279d0da3"
        );
        assert_eq!(
            standard_sig,
            "WQjS2zyW+BjgfoZDK3IFx6OIwXPmRxO22PN4FpfEJNpjY/AgEsLmqLVak99QZyY14YKNI5hiARiPkulHNpsrCQ=="
        );
        assert_eq!(
            pro_sig,
            "T1FrV8mlhTvhk+0laH74qnb6s6CTM2M+waEhIAtjszKlN3pZyQ+w18xDfp0xvbA3MO+FXvt8Z+B+hmXDrvBaDA=="
        );
    }

    struct ScriptedImageFetcher {
        payload: Vec<u8>,
        hits: AtomicU32,
    }

    impl ScriptedImageFetcher {
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

    impl ImageFetcher for ScriptedImageFetcher {
        fn fetch_into(&self, _url: &str, dest: &Path) -> Result<(), ImageError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|error| ImageError::Cache(error.to_string()))?;
            }
            fs::write(dest, &self.payload).map_err(|error| ImageError::Cache(error.to_string()))
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

    fn linux_pro() -> HostCapabilities {
        let gpus = [
            nvidia_gpu(),
            nvidia_gpu_with("NVIDIA GeForce RTX 3060", "GPU-TEST-3060", 12),
        ];
        evaluate(EvaluateInput {
            os: HostOs::Linux,
            kvm: KvmAccess::PresentReadWrite,
            whpx: WhpxCapability::NotApplicable,
            iommu: IommuStatus::Present,
            gpus: &gpus,
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

    fn spec_for(bytes: &[u8], url: &str, lab_tier: LabTier) -> (ImageTrust, GuestImageSpec) {
        let (trust, manifest) = signed_fixture(bytes);
        (
            trust,
            GuestImageSpec {
                lab_tier,
                url: url.to_string(),
                manifest,
            },
        )
    }

    #[test]
    fn macos_skips_without_fetching_a_guest_image() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES);
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let status = ensure(
            HostOs::MacOs,
            &macos_caps(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &fetcher,
        )
        .unwrap();

        assert_eq!(status.action, ImageEnsureAction::SkippedOs);
        assert_eq!(status.backend, ImageBackend::None);
        assert!(!status.image_booted);
        assert_eq!(fetcher.hit_count(), 0);
        assert!(status.log.contains("Mac"));
        assert!(status.log.contains("not fetched") || status.log.contains("are not fetched"));
        assert!(!cache.path().join("standard").exists());
    }

    #[test]
    fn linux_without_standard_offered_fails_fast_without_fetch() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES);
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let error = ensure(
            HostOs::Linux,
            &linux_no_kvm(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &fetcher,
        )
        .unwrap_err();

        assert!(matches!(error, ImageError::StandardNotOffered { .. }));
        assert_eq!(fetcher.hit_count(), 0);
        let status = status_from_result(Err(error));
        assert_eq!(status.action, ImageEnsureAction::Refused);
        assert!(!status.image_booted);
    }

    #[test]
    fn linux_fetches_standard_image_when_missing() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES);
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let status = ensure(
            HostOs::Linux,
            &linux_standard(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &fetcher,
        )
        .unwrap();

        assert_eq!(status.action, ImageEnsureAction::Fetched);
        assert_eq!(status.backend, ImageBackend::Qemu);
        assert_eq!(status.lab_tier.as_deref(), Some("standard"));
        assert!(!status.image_booted);
        assert_eq!(fetcher.hit_count(), 1);
        let path = PathBuf::from(status.path.as_deref().unwrap());
        assert_eq!(fs::read(&path).unwrap(), FIXTURE_STANDARD_BYTES);
        assert!(status.log.contains("fetched"));
        assert!(status.log.contains("Ed25519"));
        assert_eq!(
            verify_image(&trust, &spec.manifest, Some(&fs::read(&path).unwrap())),
            Ok(())
        );
    }

    #[test]
    fn linux_skips_when_present_and_signature_matches() {
        let cache = tempfile::tempdir().unwrap();
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let first = ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES);
        ensure(
            HostOs::Linux,
            &linux_standard(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &first,
        )
        .unwrap();

        let second = ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES);
        let status = ensure(
            HostOs::Linux,
            &linux_standard(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &second,
        )
        .unwrap();

        assert_eq!(status.action, ImageEnsureAction::SkippedPresent);
        assert_eq!(second.hit_count(), 0);
        assert!(status.log.contains("skipped because present"));
        assert!(!status.image_booted);
    }

    #[test]
    fn linux_digest_mismatch_fails_fast_and_does_not_mark_ready() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedImageFetcher::new(b"evil-bytes".as_slice());
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let error = ensure(
            HostOs::Linux,
            &linux_standard(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &fetcher,
        )
        .unwrap_err();

        assert_eq!(error, ImageError::DigestMismatch);
        assert_eq!(fetcher.hit_count(), 1);
        assert!(!cache.path().join("standard/rootfs.img").exists());
        let status = status_from_result(Err(error));
        assert_eq!(status.action, ImageEnsureAction::Rejected);
        assert!(status.log.contains("rejected"));
        assert!(!status.image_booted);
    }

    #[test]
    fn linux_signature_mismatch_fails_fast() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES);
        let (trust, mut spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        spec.manifest.signature = signed_fixture(FIXTURE_PRO_BYTES).1.signature;
        let error = ensure(
            HostOs::Linux,
            &linux_standard(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &fetcher,
        )
        .unwrap_err();
        assert_eq!(error, ImageError::SignatureMismatch);
        assert!(!cache.path().join("standard/rootfs.img").exists());
        let status = status_from_result(Err(error));
        assert_eq!(status.action, ImageEnsureAction::Rejected);
    }

    #[test]
    fn linux_pro_fetches_pro_image_in_addition_to_standard() {
        let cache = tempfile::tempdir().unwrap();
        let (trust, standard) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let pro = GuestImageSpec {
            lab_tier: LabTier::Pro,
            url: "https://example.invalid/pro.img".to_string(),
            manifest: signed_fixture(FIXTURE_PRO_BYTES).1,
        };
        let fetcher = DualImageFetcher {
            standard: FIXTURE_STANDARD_BYTES.to_vec(),
            pro: FIXTURE_PRO_BYTES.to_vec(),
            hits: AtomicU32::new(0),
        };
        let status = ensure(
            HostOs::Linux,
            &linux_pro(),
            Some(&trust),
            cache.path(),
            Some(&standard),
            Some(&pro),
            &fetcher,
        )
        .unwrap();

        assert_eq!(status.action, ImageEnsureAction::Fetched);
        assert_eq!(status.backend, ImageBackend::Qemu);
        assert_eq!(fetcher.hits.load(Ordering::SeqCst), 2);
        assert_eq!(
            fs::read(cache.path().join("standard/rootfs.img")).unwrap(),
            FIXTURE_STANDARD_BYTES
        );
        assert_eq!(
            fs::read(cache.path().join("pro/rootfs.img")).unwrap(),
            FIXTURE_PRO_BYTES
        );
        assert!(status.log.contains("standard"));
        assert!(status.log.contains("pro"));
        assert!(!status.image_booted);
        assert_eq!(
            verify_fetched_bytes(trust.verifying_key(), &pro.manifest, FIXTURE_PRO_BYTES),
            Ok(())
        );
    }

    struct DualImageFetcher {
        standard: Vec<u8>,
        pro: Vec<u8>,
        hits: AtomicU32,
    }

    impl ImageFetcher for DualImageFetcher {
        fn fetch_into(&self, url: &str, dest: &Path) -> Result<(), ImageError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            let payload = if url.contains("pro") {
                &self.pro
            } else {
                &self.standard
            };
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|error| ImageError::Cache(error.to_string()))?;
            }
            fs::write(dest, payload).map_err(|error| ImageError::Cache(error.to_string()))
        }
    }

    #[test]
    fn windows_fetches_standard_qemu_image_when_missing() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES);
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/qemu.img",
            LabTier::Standard,
        );
        let status = ensure(
            HostOs::Windows,
            &windows_standard(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &fetcher,
        )
        .unwrap();

        assert_eq!(status.action, ImageEnsureAction::Fetched);
        assert_eq!(status.backend, ImageBackend::Qemu);
        assert_eq!(status.lab_tier.as_deref(), Some("standard"));
        assert_eq!(fetcher.hit_count(), 1);
        let path = PathBuf::from(status.path.as_deref().unwrap());
        assert_eq!(fs::read(&path).unwrap(), FIXTURE_STANDARD_BYTES);
        assert!(status.log.contains("fetched"));
        assert!(status.log.contains("Ed25519"));
        assert!(status.log.contains("would fetch Pro"));
        assert!(!status.image_booted);
        assert_eq!(
            verify_image(&trust, &spec.manifest, Some(&fs::read(&path).unwrap())),
            Ok(())
        );
    }

    #[test]
    fn windows_does_not_fetch_pro_image() {
        let cache = tempfile::tempdir().unwrap();
        let (trust, standard) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let (_trust_pro, pro) = spec_for(
            FIXTURE_PRO_BYTES,
            "https://example.invalid/pro.img",
            LabTier::Pro,
        );
        let fetcher = DualImageFetcher {
            standard: FIXTURE_STANDARD_BYTES.to_vec(),
            pro: FIXTURE_PRO_BYTES.to_vec(),
            hits: AtomicU32::new(0),
        };
        let status = ensure(
            HostOs::Windows,
            &windows_standard(),
            Some(&trust),
            cache.path(),
            Some(&standard),
            Some(&pro),
            &fetcher,
        )
        .unwrap();

        assert_eq!(status.backend, ImageBackend::Qemu);
        assert_eq!(fetcher.hits.load(Ordering::SeqCst), 1);
        assert!(!cache.path().join("pro/rootfs.img").exists());
        assert!(status.log.contains("would fetch Pro"));
        assert!(!status.image_booted);
    }

    #[test]
    fn windows_digest_mismatch_fails_fast_and_does_not_mark_ready() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedImageFetcher::new(b"evil-bytes".as_slice());
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/qemu.img",
            LabTier::Standard,
        );
        let error = ensure(
            HostOs::Windows,
            &windows_standard(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &fetcher,
        )
        .unwrap_err();

        assert_eq!(error, ImageError::DigestMismatch);
        assert_eq!(fetcher.hit_count(), 1);
        assert!(!cache.path().join("standard/rootfs.img").exists());
        let status = status_from_result(Err(error));
        assert_eq!(status.action, ImageEnsureAction::Rejected);
        assert!(!status.image_booted);
    }

    #[test]
    fn windows_without_standard_offered_fails_fast() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES);
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/qemu.img",
            LabTier::Standard,
        );
        let error = ensure(
            HostOs::Windows,
            &windows_no_whpx(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &fetcher,
        )
        .unwrap_err();
        assert!(matches!(error, ImageError::StandardNotOffered { .. }));
        assert_eq!(fetcher.hit_count(), 0);
    }

    #[test]
    fn linux_and_windows_are_peer_paths_not_a_windows_special_case() {
        let linux_cache = tempfile::tempdir().unwrap();
        let windows_cache = tempfile::tempdir().unwrap();
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/img",
            LabTier::Standard,
        );
        let linux = ensure(
            HostOs::Linux,
            &linux_standard(),
            Some(&trust),
            linux_cache.path(),
            Some(&spec),
            None,
            &ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES),
        )
        .unwrap();
        let windows = ensure(
            HostOs::Windows,
            &windows_standard(),
            Some(&trust),
            windows_cache.path(),
            Some(&spec),
            None,
            &ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES),
        )
        .unwrap();

        assert_eq!(linux.backend, ImageBackend::Qemu);
        assert_eq!(windows.backend, ImageBackend::Qemu);
        assert_eq!(linux.action, ImageEnsureAction::Fetched);
        assert_eq!(windows.action, ImageEnsureAction::Fetched);
        assert!(!linux.image_booted);
        assert!(!windows.image_booted);
    }

    #[test]
    fn file_url_fetcher_reads_local_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("fixture.img");
        fs::write(&file, FIXTURE_STANDARD_BYTES).unwrap();
        let dest = dir.path().join("out.img");
        let url = format!("file://{}", file.display());
        HttpImageFetcher::default().fetch_into(&url, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), FIXTURE_STANDARD_BYTES);
    }

    #[test]
    fn httptest_loopback_fetch_does_not_use_the_internet() {
        let payload = FIXTURE_STANDARD_BYTES;
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().expect("tcp");
        thread::spawn(move || {
            let request = server.recv().unwrap();
            request.respond(Response::from_data(payload)).unwrap();
        });

        let cache = tempfile::tempdir().unwrap();
        let url = format!("http://{addr}/standard.img");
        let (trust, spec) = spec_for(payload, &url, LabTier::Standard);
        let status = ensure(
            HostOs::Linux,
            &linux_standard(),
            Some(&trust),
            cache.path(),
            Some(&spec),
            None,
            &HttpImageFetcher::default(),
        )
        .unwrap();
        assert_eq!(status.action, ImageEnsureAction::Fetched);
        assert_eq!(fs::read(status.path.as_deref().unwrap()).unwrap(), payload);
        assert!(!status.image_booted);
    }

    #[test]
    fn http_fetcher_refuses_non_loopback_in_tests_by_default() {
        if image_network_env_enabled() {
            return;
        }
        let dest = tempfile::tempdir().unwrap().path().join("out.img");
        let error = HttpImageFetcher::default()
            .fetch_into("https://example.invalid/standard.img", &dest)
            .unwrap_err();
        assert!(matches!(error, ImageError::NetworkDisabled { .. }));
    }

    #[test]
    fn ensure_from_env_reuses_matchmaker_image_metadata() {
        let cache = tempfile::tempdir().unwrap();
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let cache_path = cache.path().to_path_buf();
        let url = spec.url.clone();
        let digest = spec.manifest.digest_hex();
        let signature = spec.manifest.signature_base64();
        let public = trust.public_key_hex();
        let local = trust.local_digest().to_hex();
        let status = ensure_from_env(
            HostOs::Linux,
            &linux_standard(),
            |key| match key {
                PUBLIC_KEY_ENV => Some(public.clone()),
                LOCAL_DIGEST_ENV => Some(local.clone()),
                STANDARD_DIGEST_ENV => Some(digest.clone()),
                STANDARD_SIG_ENV => Some(signature.clone()),
                STANDARD_URL_ENV => Some(url.clone()),
                CACHE_ENV => Some(cache_path.to_string_lossy().into_owned()),
                _ => None,
            },
            &ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES),
        )
        .unwrap();
        assert_eq!(status.action, ImageEnsureAction::Fetched);
        assert_eq!(status.digest.as_deref(), Some(digest.as_str()));
        assert!(!status.image_booted);
    }

    #[test]
    fn windows_ensure_from_env_fetches_standard_qemu_image() {
        let cache = tempfile::tempdir().unwrap();
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/qemu.img",
            LabTier::Standard,
        );
        let cache_path = cache.path().to_path_buf();
        let url = spec.url.clone();
        let digest = spec.manifest.digest_hex();
        let signature = spec.manifest.signature_base64();
        let public = trust.public_key_hex();
        let local = trust.local_digest().to_hex();
        let status = ensure_from_env(
            HostOs::Windows,
            &windows_standard(),
            |key| match key {
                PUBLIC_KEY_ENV => Some(public.clone()),
                LOCAL_DIGEST_ENV => Some(local.clone()),
                STANDARD_DIGEST_ENV => Some(digest.clone()),
                STANDARD_SIG_ENV => Some(signature.clone()),
                STANDARD_URL_ENV => Some(url.clone()),
                CACHE_ENV => Some(cache_path.to_string_lossy().into_owned()),
                _ => None,
            },
            &ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES),
        )
        .unwrap();
        assert_eq!(status.action, ImageEnsureAction::Fetched);
        assert_eq!(status.backend, ImageBackend::Qemu);
        assert_eq!(status.digest.as_deref(), Some(digest.as_str()));
        assert!(!status.image_booted);
    }

    #[test]
    fn ensure_from_env_fails_fast_when_local_digest_pin_differs() {
        let cache = tempfile::tempdir().unwrap();
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/standard.img",
            LabTier::Standard,
        );
        let other = Sha256Digest::of(FIXTURE_PRO_BYTES).to_hex();
        let error = ensure_from_env(
            HostOs::Linux,
            &linux_standard(),
            |key| match key {
                PUBLIC_KEY_ENV => Some(trust.public_key_hex()),
                LOCAL_DIGEST_ENV => Some(other.clone()),
                STANDARD_DIGEST_ENV => Some(spec.manifest.digest_hex()),
                STANDARD_SIG_ENV => Some(spec.manifest.signature_base64()),
                STANDARD_URL_ENV => Some(spec.url.clone()),
                CACHE_ENV => Some(cache.path().to_string_lossy().into_owned()),
                _ => None,
            },
            &ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES),
        )
        .unwrap_err();
        assert_eq!(error, ImageError::DigestMismatch);
    }

    #[test]
    fn cache_env_overrides_default_image_dir() {
        let dir = resolve_image_cache_dir(|key| {
            (key == CACHE_ENV).then(|| "/tmp/dcl-image-cache".to_string())
        });
        assert_eq!(dir, PathBuf::from("/tmp/dcl-image-cache"));
    }

    #[test]
    fn ensure_never_sets_image_booted() {
        let cache = tempfile::tempdir().unwrap();
        let (trust, spec) = spec_for(
            FIXTURE_STANDARD_BYTES,
            "https://example.invalid/img",
            LabTier::Standard,
        );
        let cases = [
            ensure(
                HostOs::MacOs,
                &macos_caps(),
                Some(&trust),
                cache.path(),
                Some(&spec),
                None,
                &ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES),
            )
            .unwrap(),
            ensure(
                HostOs::Linux,
                &linux_standard(),
                Some(&trust),
                cache.path(),
                Some(&spec),
                None,
                &ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES),
            )
            .unwrap(),
            ensure(
                HostOs::Windows,
                &windows_standard(),
                Some(&trust),
                cache.path(),
                Some(&spec),
                None,
                &ScriptedImageFetcher::new(FIXTURE_STANDARD_BYTES),
            )
            .unwrap(),
        ];
        for status in cases {
            assert!(!status.image_booted);
        }
    }
}
