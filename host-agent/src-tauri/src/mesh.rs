//! Headscale mesh join for the Host Agent (phase 14).
//!
//! The webview must not `fetch` Matchmaker or Headscale (CSP allows no remote
//! origins). This module mints a preauth key through the Matchmaker, then
//! builds a `tailscale up` invocation from **env** (`HEADSCALE_LOGIN_SERVER`)
//! plus that key. No API key, preauth key, or login server is hardcoded.
//!
//! Matchmaker mint tags this node `tag:host` (not a per-session identity).
//! Default-deny ACLs live in Headscale's policy file, not in these join args.
//! This join must never pass `--advertise-exit-node` or `--exit-node`.
//! Guest VMs get `--exit-node` from Matchmaker session JSON (`HEADSCALE_EXIT_NODE`),
//! not from this Host Agent. See `docs/runbooks/headscale.md`.

use std::fmt;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use thiserror::Error;

use crate::identity;
use crate::matchmaker::{HttpRequest, MatchmakerCredentials, MatchmakerError, Transport};

#[derive(Debug, Error)]
pub enum MeshError {
    #[error("{0} is not set. The Host Agent does not hardcode a Headscale login server, Matchmaker location, or mesh credentials.")]
    MissingEnv(&'static str),
    #[error("this machine has no Matchmaker host id; register before joining the mesh")]
    NotRegistered,
    #[error("HEADSCALE_LOGIN_SERVER must be an http or https URL")]
    InvalidLoginServer,
    #[error("could not persist or load the Matchmaker host id: {0}")]
    Identity(#[from] identity::IdentityError),
    #[error("{0}")]
    Matchmaker(#[from] MatchmakerError),
    #[error("Matchmaker rejected mesh preauth (HTTP {status}): {detail}")]
    Rejected { status: u16, detail: String },
    #[error("Matchmaker returned an unreadable mesh preauth (HTTP {status})")]
    UnexpectedResponse { status: u16 },
    #[error("could not run the Tailscale client ({program}): {detail}")]
    JoinFailed { program: String, detail: String },
}

/// Env + minted key for `tailscale up`. `auth_key` is never logged.
pub struct TailscaleUp {
    pub program: String,
    pub login_server: String,
    auth_key: String,
    pub hostname: Option<String>,
}

impl fmt::Debug for TailscaleUp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TailscaleUp")
            .field("program", &self.program)
            .field("login_server", &self.login_server)
            .field("auth_key", &"[redacted]")
            .field("hostname", &self.hostname)
            .finish()
    }
}

impl TailscaleUp {
    pub fn from_parts(
        program: impl Into<String>,
        login_server: impl Into<String>,
        auth_key: impl Into<String>,
        hostname: Option<String>,
    ) -> Self {
        Self {
            program: program.into(),
            login_server: login_server.into(),
            auth_key: auth_key.into(),
            hostname,
        }
    }

    pub fn args(&self) -> Vec<String> {
        let mut args = vec![
            "up".to_string(),
            "--login-server".to_string(),
            self.login_server.clone(),
            "--authkey".to_string(),
            self.auth_key.clone(),
            "--accept-dns=false".to_string(),
            "--accept-routes=false".to_string(),
        ];
        if let Some(hostname) = &self.hostname {
            args.push("--hostname".to_string());
            args.push(hostname.clone());
        }
        args
    }
}

pub trait JoinRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<(), MeshError>;
}

pub struct ProcessJoinRunner;

impl JoinRunner for ProcessJoinRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<(), MeshError> {
        let status = Command::new(program).args(args).status().map_err(|error| {
            MeshError::JoinFailed {
                program: program.to_string(),
                detail: error.to_string(),
            }
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(MeshError::JoinFailed {
                program: program.to_string(),
                detail: format!("exit {status}"),
            })
        }
    }
}

/// True when the operator set a Headscale login server. Register skips join
/// otherwise so a UI-only machine is not forced to have Tailscale installed.
pub fn should_join<F>(mut lookup: F) -> bool
where
    F: FnMut(&str) -> Option<String>,
{
    lookup("HEADSCALE_LOGIN_SERVER")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .is_some()
}

pub fn join_from_env(
    dir: &Path,
    lookup: impl FnMut(&str) -> Option<String>,
    transport: &impl Transport,
    runner: &impl JoinRunner,
) -> Result<TailscaleUp, MeshError> {
    let plan = prepare_join(dir, lookup, transport)?;
    runner.run(&plan.program, &plan.args())?;
    Ok(plan)
}

/// Mint a preauth key and build `tailscale up` args. Does not spawn a process.
pub fn prepare_join(
    dir: &Path,
    mut lookup: impl FnMut(&str) -> Option<String>,
    transport: &impl Transport,
) -> Result<TailscaleUp, MeshError> {
    let login_server = require_login_server(&mut lookup)?;
    let program = lookup("TAILSCALE_BIN")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "tailscale".to_string());
    let credentials = MatchmakerCredentials::from_lookup(&mut lookup)?;
    let Some(identity) = identity::load(dir)? else {
        return Err(MeshError::NotRegistered);
    };

    let request = HttpRequest {
        method: "POST",
        url: preauth_url(&credentials.base_url, &identity.host_id),
        authorization: credentials.authorization_header(),
        body: serde_json::json!({}),
    };
    let response = transport.send(&request)?;
    let grant = parse_preauth(response.status, &response.body)?;

    Ok(TailscaleUp::from_parts(
        program,
        login_server,
        grant.preauth_key,
        Some(credentials.display_name.clone()),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreauthBody {
    preauth_key: String,
}

fn parse_preauth(status: u16, body: &str) -> Result<PreauthBody, MeshError> {
    if status != 200 {
        return Err(MeshError::Rejected {
            status,
            detail: truncate_detail(body),
        });
    }
    let parsed: PreauthBody =
        serde_json::from_str(body).map_err(|_| MeshError::UnexpectedResponse { status })?;
    if parsed.preauth_key.trim().is_empty() {
        return Err(MeshError::UnexpectedResponse { status });
    }
    Ok(parsed)
}

fn require_login_server<F>(lookup: &mut F) -> Result<String, MeshError>
where
    F: FnMut(&str) -> Option<String>,
{
    let raw = lookup("HEADSCALE_LOGIN_SERVER")
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .ok_or(MeshError::MissingEnv("HEADSCALE_LOGIN_SERVER"))?;
    if !(raw.starts_with("http://") || raw.starts_with("https://")) || raw.len() < 8 {
        return Err(MeshError::InvalidLoginServer);
    }
    Ok(raw)
}

fn preauth_url(base: &str, host_id: &str) -> String {
    format!("{base}/v1/hosts/{host_id}/mesh/preauth")
}

fn truncate_detail(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= 400 {
        compact
    } else {
        format!("{}…", &compact[..400])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::HostRegistration;
    use crate::matchmaker::{HttpRequest, HttpResponse, MatchmakerError, Transport};
    use crate::models::BYTES_PER_GIB;
    use std::sync::Mutex;
    use tempfile::tempdir;

    const TEST_BOOTSTRAP: &str = "test-host-bootstrap-token";
    const TEST_PREAUTH: &str = "tskey-auth-test-not-a-secret";
    const HOST_ID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    fn persist_identity(dir: &Path) {
        identity::save(
            dir,
            &HostRegistration {
                host_id: HOST_ID.to_string(),
                display_name: "lab-pc".to_string(),
                lab_tier: "standard".to_string(),
                status: "pending".to_string(),
                shared_logical_cores: 4,
                shared_memory_bytes: 8 * BYTES_PER_GIB,
                share_gpu: false,
                shared_gpu_id: None,
            },
        )
        .unwrap();
    }

    fn lookup(key: &str) -> Option<String> {
        match key {
            "MATCHMAKER_URL" => Some("http://matchmaker.test".to_string()),
            "HOST_BOOTSTRAP_TOKEN" => Some(TEST_BOOTSTRAP.to_string()),
            "HOST_DISPLAY_NAME" => Some("lab-pc".to_string()),
            "HEADSCALE_LOGIN_SERVER" => Some("http://headscale.test".to_string()),
            "TAILSCALE_BIN" => Some("/usr/local/bin/tailscale".to_string()),
            _ => None,
        }
    }

    struct RecordingTransport {
        status: u16,
        body: String,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl RecordingTransport {
        fn ok() -> Self {
            Self {
                status: 200,
                body: serde_json::json!({
                    "loginServer": "http://headscale.from-matchmaker.test",
                    "preauthKey": TEST_PREAUTH,
                    "user": "1",
                    "reusable": false,
                    "ephemeral": false,
                    "expiresAt": "2026-09-05T12:00:00Z"
                })
                .to_string(),
                calls: Mutex::new(Vec::new()),
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

    struct RecordingRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl RecordingRunner {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl JoinRunner for RecordingRunner {
        fn run(&self, program: &str, args: &[String]) -> Result<(), MeshError> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            Ok(())
        }
    }

    #[test]
    fn missing_login_server_fails_fast_before_http() {
        let dir = tempdir().unwrap();
        persist_identity(dir.path());
        let transport = RecordingTransport::ok();

        let error = prepare_join(
            dir.path(),
            |key| match key {
                "MATCHMAKER_URL" => Some("http://matchmaker.test".to_string()),
                "HOST_BOOTSTRAP_TOKEN" => Some(TEST_BOOTSTRAP.to_string()),
                _ => None,
            },
            &transport,
        )
        .unwrap_err();

        assert!(matches!(error, MeshError::MissingEnv("HEADSCALE_LOGIN_SERVER")));
        assert!(transport.calls.lock().unwrap().is_empty());
        assert!(!error.to_string().contains(TEST_BOOTSTRAP));
    }

    #[test]
    fn missing_matchmaker_url_fails_fast() {
        let dir = tempdir().unwrap();
        persist_identity(dir.path());
        let transport = RecordingTransport::ok();

        let error = prepare_join(
            dir.path(),
            |key| match key {
                "HEADSCALE_LOGIN_SERVER" => Some("http://headscale.test".to_string()),
                "HOST_BOOTSTRAP_TOKEN" => Some(TEST_BOOTSTRAP.to_string()),
                _ => None,
            },
            &transport,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            MeshError::Matchmaker(MatchmakerError::MissingEnv("MATCHMAKER_URL"))
        ));
        assert!(transport.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn unregistered_host_does_not_call_matchmaker() {
        let dir = tempdir().unwrap();
        let transport = RecordingTransport::ok();
        let error = prepare_join(dir.path(), lookup, &transport).unwrap_err();
        assert!(matches!(error, MeshError::NotRegistered));
        assert!(transport.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn prepare_join_posts_to_matchmaker_and_builds_tailscale_up_from_env() {
        let dir = tempdir().unwrap();
        persist_identity(dir.path());
        let transport = RecordingTransport::ok();

        let plan = prepare_join(dir.path(), lookup, &transport).unwrap();

        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "POST");
        assert_eq!(
            calls[0].url,
            format!("http://matchmaker.test/v1/hosts/{HOST_ID}/mesh/preauth")
        );
        assert_eq!(
            calls[0].authorization,
            format!("Bearer {TEST_BOOTSTRAP}")
        );
        assert!(!format!("{:?}", calls[0]).contains(TEST_BOOTSTRAP));

        assert_eq!(plan.program, "/usr/local/bin/tailscale");
        assert_eq!(plan.login_server, "http://headscale.test");
        assert_eq!(plan.auth_key, TEST_PREAUTH);
        let args = plan.args();
        assert_eq!(args[0], "up");
        assert!(args.contains(&"--login-server".to_string()));
        assert!(args.contains(&"http://headscale.test".to_string()));
        assert!(args.contains(&"--authkey".to_string()));
        assert!(args.contains(&TEST_PREAUTH.to_string()));
        assert!(args.contains(&"--accept-dns=false".to_string()));
        assert!(args.contains(&"--accept-routes=false".to_string()));
        assert!(!args.iter().any(|a| a.contains("advertise-exit-node")));
        assert!(!args.iter().any(|a| a.contains("advertise-tags")));
        assert!(!args.iter().any(|a| a == "--exit-node" || a.starts_with("--exit-node=")));
        assert!(!args.iter().any(|a| a.contains("headscale.from-matchmaker")));
        let debug = format!("{plan:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(TEST_PREAUTH));
    }

    #[test]
    fn join_from_env_invokes_runner_with_built_args() {
        let dir = tempdir().unwrap();
        persist_identity(dir.path());
        let transport = RecordingTransport::ok();
        let runner = RecordingRunner::new();

        join_from_env(dir.path(), lookup, &transport, &runner).unwrap();

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "/usr/local/bin/tailscale");
        assert!(calls[0].1.contains(&TEST_PREAUTH.to_string()));
        assert!(!calls[0].1.iter().any(|a| a.contains("advertise-exit-node")));
    }

    #[test]
    fn mesh_source_does_not_embed_preauth_or_api_secrets() {
        let source = include_str!("mesh.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("mesh.rs should have a test module");
        assert!(
            !production.contains("HEADSCALE_API_KEY"),
            "Host Agent must not read the Matchmaker Headscale API secret"
        );
        assert!(!production.contains("\"--advertise-exit-node\""));
        assert!(!production.contains("advertise-tags"));
        assert!(
            !production.contains("tskey-auth-"),
            "production mesh.rs must not embed a tskey"
        );
    }

    #[test]
    fn should_join_is_false_when_login_server_is_unset() {
        assert!(!should_join(|_| None));
        assert!(should_join(|key| {
            (key == "HEADSCALE_LOGIN_SERVER").then(|| "http://headscale.test".to_string())
        }));
    }
}