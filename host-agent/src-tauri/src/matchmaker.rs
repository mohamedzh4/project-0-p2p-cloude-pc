//! Host Agent client for the Matchmaker host-inventory API.
//!
//! Calls stay in Rust: the webview CSP allows no remote origins, and the
//! webview is untrusted input. Caps are re-clamped here before anything is
//! serialized. The agent sends `HOST_BOOTSTRAP_TOKEN` only — it never reads
//! `HOST_APPROVE_TOKEN`, so it cannot self-approve.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

use crate::config;
use crate::identity::{self, HostRegistration};
use crate::models::{HardwareSnapshot, SharingConfig};

const DEFAULT_LAB_TIER: &str = "standard";
const DISPLAY_NAME_MAX: usize = 128;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const ERROR_BODY_MAX: usize = 400;

/// Fixture token for unit tests. Not a production secret and never shipped as
/// a default in the agent.
#[cfg(test)]
const TEST_BOOTSTRAP_TOKEN: &str = "test-host-bootstrap-token";

#[derive(Debug, Error)]
pub enum MatchmakerError {
    #[error("{0} is not set. The Host Agent does not hardcode a Matchmaker location or host credentials.")]
    MissingEnv(&'static str),
    #[error("MATCHMAKER_URL must be an http or https URL (origin only, no /v1 suffix required)")]
    InvalidUrl,
    #[error("shareGpu is true but sharedGpuId is missing; GPU assignment is exclusive for the session, not a concurrent slice")]
    GpuIdRequired,
    #[error("sharedLogicalCores must be at least 1 to register with the Matchmaker")]
    CoresRequired,
    #[error("sharedMemoryBytes must be at least 1 to register with the Matchmaker")]
    MemoryRequired,
    #[error("could not load sharing configuration: {0}")]
    Config(#[from] config::ConfigError),
    #[error("could not persist the Matchmaker host id: {0}")]
    Identity(#[from] identity::IdentityError),
    #[error("could not reach the Matchmaker: {0}")]
    Network(String),
    #[error("Matchmaker rejected host registration (HTTP {status}): {detail}")]
    Rejected { status: u16, detail: String },
    #[error("Matchmaker returned an unreadable host registration (HTTP {status})")]
    UnexpectedResponse { status: u16 },
}

/// Env-sourced client settings. The bootstrap token is never logged.
pub struct MatchmakerCredentials {
    pub base_url: String,
    bootstrap_token: String,
    pub display_name: String,
}

impl fmt::Debug for MatchmakerCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MatchmakerCredentials")
            .field("base_url", &self.base_url)
            .field("bootstrap_token", &"[redacted]")
            .field("display_name", &self.display_name)
            .finish()
    }
}

impl MatchmakerCredentials {
    pub fn from_lookup<F>(mut lookup: F) -> Result<Self, MatchmakerError>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let base_url = require_env(&mut lookup, "MATCHMAKER_URL")?;
        let bootstrap_token = require_env(&mut lookup, "HOST_BOOTSTRAP_TOKEN")?;
        let display_name = lookup("HOST_DISPLAY_NAME")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(machine_display_name);

        Ok(Self {
            base_url: normalize_base_url(&base_url)?,
            bootstrap_token,
            display_name: truncate_display_name(&display_name),
        })
    }

    fn authorization_header(&self) -> String {
        format!("Bearer {}", self.bootstrap_token)
    }
}

/// Phase 2 register/update body. `kasmUrl` is omitted so a PUT does not wipe a
/// URL some other process stored on the host row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInventoryBody {
    pub display_name: String,
    pub lab_tier: String,
    pub shared_logical_cores: u32,
    pub shared_memory_bytes: u64,
    pub share_gpu: bool,
    pub shared_gpu_id: Option<String>,
}

impl HostInventoryBody {
    /// Re-clamps even if the caller already did. The webview is untrusted and
    /// a config file can travel between machines.
    pub fn from_sharing(
        config: &SharingConfig,
        hardware: &HardwareSnapshot,
        display_name: &str,
    ) -> Result<Self, MatchmakerError> {
        let clamped = config.clamped_to(hardware);
        require_named_gpu(clamped.share_gpu, &clamped.shared_gpu_id)?;
        if clamped.shared_logical_cores < 1 {
            return Err(MatchmakerError::CoresRequired);
        }
        if clamped.shared_memory_bytes < 1 {
            return Err(MatchmakerError::MemoryRequired);
        }

        let display_name = truncate_display_name(display_name.trim());
        if display_name.is_empty() {
            return Err(MatchmakerError::MissingEnv("HOST_DISPLAY_NAME"));
        }

        Ok(Self {
            display_name,
            lab_tier: DEFAULT_LAB_TIER.to_string(),
            shared_logical_cores: clamped.shared_logical_cores,
            shared_memory_bytes: clamped.shared_memory_bytes,
            share_gpu: clamped.share_gpu,
            shared_gpu_id: clamped.shared_gpu_id,
        })
    }
}

pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    authorization: String,
    pub body: serde_json::Value,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("authorization", &"Bearer [redacted]")
            .field("body", &self.body)
            .finish()
    }
}

impl HttpRequest {
    fn authorization_header(&self) -> &str {
        &self.authorization
    }
}

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub trait Transport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, MatchmakerError>;
}

pub struct UreqTransport;

impl Transport for UreqTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, MatchmakerError> {
        // Do not follow redirects: the bootstrap bearer must not be replayed
        // against an unexpected host.
        let agent = ureq::AgentBuilder::new()
            .timeout(REQUEST_TIMEOUT)
            .redirects(0)
            .build();

        let call = match request.method {
            "POST" => agent.post(&request.url),
            "PUT" => agent.put(&request.url),
            other => {
                return Err(MatchmakerError::Network(format!(
                    "unsupported HTTP method {other}"
                )))
            }
        }
        .set("Authorization", request.authorization_header())
        .set("Accept", "application/json")
        .set("User-Agent", "host-agent/0.1");

        match call.send_json(request.body.clone()) {
            Ok(response) => read_ureq_response(response.status(), response),
            Err(ureq::Error::Status(status, response)) => read_ureq_response(status, response),
            Err(error) => Err(MatchmakerError::Network(error.to_string())),
        }
    }
}

fn read_ureq_response(
    status: u16,
    response: ureq::Response,
) -> Result<HttpResponse, MatchmakerError> {
    let body = response
        .into_string()
        .map_err(|error| MatchmakerError::Network(error.to_string()))?;
    Ok(HttpResponse { status, body })
}

/// Registers or updates declared caps from the on-disk (clamped) config.
pub fn register_from_env(
    dir: &Path,
    hardware: &HardwareSnapshot,
    transport: &impl Transport,
) -> Result<HostRegistration, MatchmakerError> {
    register_declared_caps(dir, hardware, |key| std::env::var(key).ok(), transport)
}

pub fn register_declared_caps(
    dir: &Path,
    hardware: &HardwareSnapshot,
    lookup: impl FnMut(&str) -> Option<String>,
    transport: &impl Transport,
) -> Result<HostRegistration, MatchmakerError> {
    let config = config::load_for_hardware(dir, hardware)?;
    let credentials = MatchmakerCredentials::from_lookup(lookup)?;
    sync_inventory(transport, &credentials, dir, &config, hardware)
}

pub fn sync_inventory(
    transport: &impl Transport,
    credentials: &MatchmakerCredentials,
    dir: &Path,
    config: &SharingConfig,
    hardware: &HardwareSnapshot,
) -> Result<HostRegistration, MatchmakerError> {
    let body = HostInventoryBody::from_sharing(config, hardware, &credentials.display_name)?;
    let payload = serde_json::to_value(&body)
        .map_err(|error| MatchmakerError::Network(error.to_string()))?;
    let existing = identity::load(dir)?;

    let (method, url) = match existing.as_ref() {
        Some(previous) => ("PUT", host_url(&credentials.base_url, &previous.host_id)),
        None => ("POST", hosts_url(&credentials.base_url)),
    };

    let request = HttpRequest {
        method,
        url,
        authorization: credentials.authorization_header(),
        body: payload,
    };
    let response = transport.send(&request)?;
    let registration = parse_registration(response.status, &response.body)?;
    identity::save(dir, &registration)?;
    Ok(registration)
}

fn parse_registration(status: u16, body: &str) -> Result<HostRegistration, MatchmakerError> {
    if status != 200 && status != 201 {
        return Err(MatchmakerError::Rejected {
            status,
            detail: truncate_detail(body),
        });
    }

    serde_json::from_str(body).map_err(|_| MatchmakerError::UnexpectedResponse { status })
}

fn require_named_gpu(
    share_gpu: bool,
    shared_gpu_id: &Option<String>,
) -> Result<(), MatchmakerError> {
    if share_gpu && shared_gpu_id.as_deref().map(str::trim).unwrap_or("").is_empty() {
        Err(MatchmakerError::GpuIdRequired)
    } else {
        Ok(())
    }
}

fn require_env<F>(lookup: &mut F, key: &'static str) -> Result<String, MatchmakerError>
where
    F: FnMut(&str) -> Option<String>,
{
    lookup(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or(MatchmakerError::MissingEnv(key))
}

fn normalize_base_url(raw: &str) -> Result<String, MatchmakerError> {
    let mut trimmed = raw.trim().trim_end_matches('/').to_string();
    if let Some(without_v1) = trimmed.strip_suffix("/v1") {
        trimmed = without_v1.trim_end_matches('/').to_string();
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) || trimmed.len() < 8 {
        return Err(MatchmakerError::InvalidUrl);
    }
    Ok(trimmed)
}

fn hosts_url(base: &str) -> String {
    format!("{base}/v1/hosts")
}

fn host_url(base: &str, host_id: &str) -> String {
    format!("{base}/v1/hosts/{host_id}")
}

fn truncate_display_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= DISPLAY_NAME_MAX {
        return trimmed.to_string();
    }
    trimmed.chars().take(DISPLAY_NAME_MAX).collect()
}

fn truncate_detail(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= ERROR_BODY_MAX {
        compact
    } else {
        format!("{}…", &compact[..ERROR_BODY_MAX])
    }
}

fn machine_display_name() -> String {
    hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "host-agent".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BYTES_PER_GIB, CONFIG_VERSION};
    use crate::test_support::{nvidia_gpu, snapshot};
    use std::sync::Mutex;
    use tempfile::tempdir;

    fn sharing(
        cores: u32,
        memory_bytes: u64,
        share_gpu: bool,
        gpu_id: Option<&str>,
    ) -> SharingConfig {
        SharingConfig {
            version: CONFIG_VERSION,
            sharing_enabled: true,
            shared_logical_cores: cores,
            shared_memory_bytes: memory_bytes,
            share_gpu,
            shared_gpu_id: gpu_id.map(str::to_string),
        }
    }

    fn test_credentials() -> MatchmakerCredentials {
        MatchmakerCredentials::from_lookup(|key| match key {
            "MATCHMAKER_URL" => Some("http://matchmaker.test".to_string()),
            "HOST_BOOTSTRAP_TOKEN" => Some(TEST_BOOTSTRAP_TOKEN.to_string()),
            "HOST_DISPLAY_NAME" => Some("lab-pc".to_string()),
            _ => None,
        })
        .unwrap()
    }

    fn api_body(
        host_id: &str,
        cores: u32,
        memory_bytes: u64,
        share_gpu: bool,
        gpu_id: Option<&str>,
    ) -> String {
        serde_json::json!({
            "hostId": host_id,
            "displayName": "lab-pc",
            "labTier": "standard",
            "status": "pending",
            "sharedLogicalCores": cores,
            "sharedMemoryBytes": memory_bytes,
            "shareGpu": share_gpu,
            "sharedGpuId": gpu_id,
            "kasmUrl": null
        })
        .to_string()
    }

    struct RecordingTransport {
        status: u16,
        body: String,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl RecordingTransport {
        fn ok(host_id: &str, share_gpu: bool, gpu_id: Option<&str>) -> Self {
            Self {
                status: 201,
                body: api_body(host_id, 4, 8 * BYTES_PER_GIB, share_gpu, gpu_id),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn last(&self) -> HttpRequest {
            let calls = self.calls.lock().unwrap();
            assert_eq!(calls.len(), 1, "expected a single Matchmaker call");
            HttpRequest {
                method: calls[0].method,
                url: calls[0].url.clone(),
                authorization: calls[0].authorization.clone(),
                body: calls[0].body.clone(),
            }
        }
    }

    impl Transport for RecordingTransport {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, MatchmakerError> {
            self.calls.lock().unwrap().push(HttpRequest {
                method: request.method,
                url: request.url.clone(),
                authorization: request.authorization.clone(),
                body: request.body.clone(),
            });
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
    }

    #[test]
    fn inventory_body_re_clamps_to_the_machine() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        let untrusted = sharing(64, 512 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));

        let body = HostInventoryBody::from_sharing(&untrusted, &hardware, "lab-pc").unwrap();

        assert_eq!(body.shared_logical_cores, 8);
        assert_eq!(body.shared_memory_bytes, 16 * BYTES_PER_GIB);
        assert!(body.share_gpu);
        assert_eq!(body.shared_gpu_id.as_deref(), Some("GPU-TEST-4070"));
        assert_eq!(body.lab_tier, "standard");
    }

    #[test]
    fn inventory_json_includes_gpu_id_when_sharing_gpu() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        let config = sharing(4, 8 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));
        let body = HostInventoryBody::from_sharing(&config, &hardware, "lab-pc").unwrap();
        let json = serde_json::to_value(&body).unwrap();

        assert_eq!(json["shareGpu"], true);
        assert_eq!(json["sharedGpuId"], "GPU-TEST-4070");
        assert_eq!(json["sharedLogicalCores"], 4);
        assert_eq!(json["displayName"], "lab-pc");
        assert!(json.get("kasmUrl").is_none());
    }

    #[test]
    fn inventory_json_keeps_gpu_id_when_sharing_is_off() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        let config = sharing(4, 8 * BYTES_PER_GIB, false, Some("GPU-TEST-4070"));
        let body = HostInventoryBody::from_sharing(&config, &hardware, "lab-pc").unwrap();
        let json = serde_json::to_value(&body).unwrap();

        assert_eq!(json["shareGpu"], false);
        assert_eq!(json["sharedGpuId"], "GPU-TEST-4070");
    }

    #[test]
    fn sharing_a_gpu_without_an_id_is_rejected_before_http() {
        assert!(matches!(
            require_named_gpu(true, &None),
            Err(MatchmakerError::GpuIdRequired)
        ));
        let message = require_named_gpu(true, &Some(String::new()))
            .unwrap_err()
            .to_string();
        assert!(message.contains("exclusive"));
        assert!(message.contains("concurrent slice"));
    }

    #[test]
    fn gpu_share_on_a_machine_without_a_gpu_is_turned_off_in_the_payload() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, Vec::new());
        let config = sharing(4, 8 * BYTES_PER_GIB, true, Some("GPU-GONE"));
        let body = HostInventoryBody::from_sharing(&config, &hardware, "lab-pc").unwrap();

        assert!(!body.share_gpu);
        assert!(body.shared_gpu_id.is_none());
    }

    #[test]
    fn missing_matchmaker_url_fails_fast() {
        let error = MatchmakerCredentials::from_lookup(|key| match key {
            "HOST_BOOTSTRAP_TOKEN" => Some(TEST_BOOTSTRAP_TOKEN.to_string()),
            _ => None,
        })
        .unwrap_err();

        assert!(matches!(error, MatchmakerError::MissingEnv("MATCHMAKER_URL")));
        assert!(!error.to_string().contains(TEST_BOOTSTRAP_TOKEN));
    }

    #[test]
    fn missing_bootstrap_token_fails_fast() {
        let error = MatchmakerCredentials::from_lookup(|key| match key {
            "MATCHMAKER_URL" => Some("http://matchmaker.test".to_string()),
            _ => None,
        })
        .unwrap_err();

        assert!(matches!(
            error,
            MatchmakerError::MissingEnv("HOST_BOOTSTRAP_TOKEN")
        ));
    }

    #[test]
    fn credentials_debug_redacts_the_token() {
        let credentials = test_credentials();
        let debug = format!("{credentials:?}");

        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(TEST_BOOTSTRAP_TOKEN));
    }

    #[test]
    fn posts_when_this_machine_has_no_host_id() {
        let dir = tempdir().unwrap();
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        let config = sharing(4, 8 * BYTES_PER_GIB, true, Some("GPU-TEST-4070"));
        config::save(dir.path(), &config).unwrap();
        let transport = RecordingTransport::ok(
            "11111111-2222-3333-4444-555555555555",
            true,
            Some("GPU-TEST-4070"),
        );

        let registration = register_declared_caps(
            dir.path(),
            &hardware,
            |key| match key {
                "MATCHMAKER_URL" => Some("http://matchmaker.test".to_string()),
                "HOST_BOOTSTRAP_TOKEN" => Some(TEST_BOOTSTRAP_TOKEN.to_string()),
                "HOST_DISPLAY_NAME" => Some("lab-pc".to_string()),
                _ => None,
            },
            &transport,
        )
        .unwrap();

        let call = transport.last();
        assert_eq!(call.method, "POST");
        assert_eq!(call.url, "http://matchmaker.test/v1/hosts");
        assert_eq!(
            call.authorization_header(),
            format!("Bearer {TEST_BOOTSTRAP_TOKEN}")
        );
        assert_eq!(call.body["shareGpu"], true);
        assert_eq!(call.body["sharedGpuId"], "GPU-TEST-4070");
        assert_eq!(call.body["sharedLogicalCores"], 4);
        assert!(call.body.get("kasmUrl").is_none());
        assert!(!format!("{call:?}").contains(TEST_BOOTSTRAP_TOKEN));
        assert_eq!(registration.host_id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(registration.status, "pending");
        assert_eq!(
            identity::load(dir.path()).unwrap().unwrap().host_id,
            registration.host_id
        );
    }

    #[test]
    fn http_payload_is_clamped_before_send() {
        let dir = tempdir().unwrap();
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        config::save(
            dir.path(),
            &sharing(64, 512 * BYTES_PER_GIB, true, Some("GPU-TEST-4070")),
        )
        .unwrap();
        let transport = RecordingTransport::ok(
            "11111111-2222-3333-4444-555555555555",
            true,
            Some("GPU-TEST-4070"),
        );

        register_declared_caps(
            dir.path(),
            &hardware,
            |key| match key {
                "MATCHMAKER_URL" => Some("http://matchmaker.test".to_string()),
                "HOST_BOOTSTRAP_TOKEN" => Some(TEST_BOOTSTRAP_TOKEN.to_string()),
                "HOST_DISPLAY_NAME" => Some("lab-pc".to_string()),
                _ => None,
            },
            &transport,
        )
        .unwrap();

        let call = transport.last();
        assert_eq!(call.body["sharedLogicalCores"], 8);
        assert_eq!(call.body["sharedMemoryBytes"], 16 * BYTES_PER_GIB);
        assert_eq!(call.body["shareGpu"], true);
        assert_eq!(call.body["sharedGpuId"], "GPU-TEST-4070");
    }

    #[test]
    fn puts_when_a_host_id_was_already_persisted() {
        let dir = tempdir().unwrap();
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, vec![nvidia_gpu()]);
        let config = sharing(4, 8 * BYTES_PER_GIB, false, Some("GPU-TEST-4070"));
        config::save(dir.path(), &config).unwrap();
        identity::save(
            dir.path(),
            &HostRegistration {
                host_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
                display_name: "lab-pc".to_string(),
                lab_tier: "standard".to_string(),
                status: "pending".to_string(),
                shared_logical_cores: 4,
                shared_memory_bytes: 8 * BYTES_PER_GIB,
                share_gpu: false,
                shared_gpu_id: Some("GPU-TEST-4070".to_string()),
            },
        )
        .unwrap();
        let transport = RecordingTransport {
            status: 200,
            body: api_body(
                "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                4,
                8 * BYTES_PER_GIB,
                false,
                Some("GPU-TEST-4070"),
            ),
            calls: Mutex::new(Vec::new()),
        };

        sync_inventory(
            &transport,
            &test_credentials(),
            dir.path(),
            &config,
            &hardware,
        )
        .unwrap();

        let call = transport.last();
        assert_eq!(call.method, "PUT");
        assert_eq!(
            call.url,
            "http://matchmaker.test/v1/hosts/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        );
        assert_eq!(call.body["shareGpu"], false);
        assert_eq!(call.body["sharedGpuId"], "GPU-TEST-4070");
    }

    #[test]
    fn zero_cores_never_reach_the_matchmaker() {
        let hardware = snapshot(8, 16 * BYTES_PER_GIB, Vec::new());
        let config = sharing(0, 8 * BYTES_PER_GIB, false, None);

        let error = HostInventoryBody::from_sharing(&config, &hardware, "lab-pc").unwrap_err();
        assert!(matches!(error, MatchmakerError::CoresRequired));
    }

    #[test]
    fn strips_a_trailing_v1_from_the_base_url() {
        let credentials = MatchmakerCredentials::from_lookup(|key| match key {
            "MATCHMAKER_URL" => Some("https://mm.example/v1/".to_string()),
            "HOST_BOOTSTRAP_TOKEN" => Some(TEST_BOOTSTRAP_TOKEN.to_string()),
            "HOST_DISPLAY_NAME" => Some("lab-pc".to_string()),
            _ => None,
        })
        .unwrap();

        assert_eq!(credentials.base_url, "https://mm.example");
    }
}
