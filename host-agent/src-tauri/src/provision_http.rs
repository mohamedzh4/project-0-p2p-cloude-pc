//! Inbound provision HTTP port. Calls [`crate::provision::plan`]; never starts
//! a hypervisor. The Matchmaker POSTs here after host selection. The webview
//! must not `fetch` this URL (CSP allows no remote origins).

use std::io::Read;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::agent_status::Occupancy;
use crate::provision::{plan, HostOs, LabTier, ProvisionError, ProvisionRequest};

const PROVISION_PATH: &str = "/v1/provision";
const BODY_MAX: usize = 16 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProvisionListenError {
    #[error("HOST_BOOTSTRAP_TOKEN is not set. The provision listener does not hardcode a token.")]
    MissingToken,
    #[error("HOST_AGENT_LISTEN_ADDR must be host:port (or \"off\" to disable)")]
    InvalidAddr,
    #[error("could not bind the provision listener at {addr}: {detail}")]
    Bind { addr: String, detail: String },
}

#[derive(Clone, PartialEq, Eq)]
pub struct ListenConfig {
    pub addr: String,
    token: String,
}

impl ListenConfig {
    pub fn new(addr: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            token: token.into(),
        }
    }
}

impl std::fmt::Debug for ListenConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenConfig")
            .field("addr", &self.addr)
            .field("token", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionHttpRequest {
    pub lab_tier: String,
    pub shared_gpu_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionHttpResponse {
    pub connection_url: String,
    pub hypervisor_invoked: bool,
    pub gpu_exclusive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpProvisionOutput {
    pub status: u16,
    pub body: String,
}

/// Env: `HOST_AGENT_LISTEN_ADDR=127.0.0.1:7842` (or `off`). Token required if listening.
pub fn listen_config_from_env<F>(
    mut lookup: F,
) -> Result<Option<ListenConfig>, ProvisionListenError>
where
    F: FnMut(&str) -> Option<String>,
{
    let addr = nonempty(lookup("HOST_AGENT_LISTEN_ADDR"));
    let Some(addr) = addr else {
        return Ok(None);
    };
    if addr.eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    if addr.parse::<SocketAddr>().is_err() {
        return Err(ProvisionListenError::InvalidAddr);
    }
    let token =
        nonempty(lookup("HOST_BOOTSTRAP_TOKEN")).ok_or(ProvisionListenError::MissingToken)?;
    Ok(Some(ListenConfig::new(addr, token)))
}

pub fn plan_http(
    os: HostOs,
    request: &ProvisionHttpRequest,
) -> Result<ProvisionHttpResponse, ProvisionError> {
    let lab_tier = LabTier::parse(&request.lab_tier)?;
    let planned = plan(
        os,
        &ProvisionRequest {
            lab_tier,
            shared_gpu_id: request.shared_gpu_id.clone(),
        },
    )?;
    Ok(ProvisionHttpResponse {
        connection_url: planned.connection_url,
        hypervisor_invoked: planned.hypervisor_invoked,
        gpu_exclusive: planned.gpu_exclusive,
    })
}

pub fn respond(
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: &[u8],
    expected_token: &str,
    os: HostOs,
) -> HttpProvisionOutput {
    let path = path.split('?').next().unwrap_or(path);
    if !method.eq_ignore_ascii_case("POST") {
        return text(405, "method not allowed");
    }
    if path != PROVISION_PATH {
        return text(404, "not found");
    }
    if !bearer_matches(authorization, expected_token) {
        return text(401, "Not authenticated");
    }
    if body.len() > BODY_MAX {
        return text(413, "payload too large");
    }

    let parsed: ProvisionHttpRequest = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => return text(400, "invalid provision request"),
    };

    match plan_http(os, &parsed) {
        Ok(planned) => {
            let json = serde_json::to_string(&planned).unwrap_or_else(|_| {
                "{\"connectionUrl\":\"\",\"hypervisorInvoked\":false,\"gpuExclusive\":false}"
                    .to_string()
            });
            HttpProvisionOutput {
                status: 200,
                body: json,
            }
        }
        Err(error) => text(400, &error.to_string()),
    }
}

/// Same as [`respond`], then records local occupancy on HTTP 200 so the tray
/// can show in-session. Pause still wins in [`crate::agent_status::presence`].
pub fn respond_and_track(
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: &[u8],
    expected_token: &str,
    os: HostOs,
    occupancy: &Occupancy,
) -> HttpProvisionOutput {
    let output = respond(method, path, authorization, body, expected_token, os);
    if output.status == 200 {
        occupancy.mark_in_session();
    }
    output
}

pub fn serve(
    config: &ListenConfig,
    os: HostOs,
    occupancy: &Occupancy,
) -> Result<(), ProvisionListenError> {
    let server = Server::http(&config.addr).map_err(|error| ProvisionListenError::Bind {
        addr: config.addr.clone(),
        detail: error.to_string(),
    })?;
    for request in server.incoming_requests() {
        let _ = dispatch(request, &config.token, os, occupancy);
    }
    Ok(())
}

fn dispatch(
    mut request: Request,
    expected_token: &str,
    os: HostOs,
    occupancy: &Occupancy,
) -> std::io::Result<()> {
    let method = method_name(request.method());
    let url = request.url().to_string();
    let authorization = header_value(&request, "Authorization");
    let mut body = Vec::new();
    let _ = request
        .as_reader()
        .take(BODY_MAX as u64 + 1)
        .read_to_end(&mut body);

    let output = respond_and_track(
        &method,
        &url,
        authorization.as_deref(),
        &body,
        expected_token,
        os,
        occupancy,
    );
    let mut response =
        Response::from_string(output.body).with_status_code(StatusCode(output.status));
    if let Ok(header) = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]) {
        response = response.with_header(header);
    }
    request.respond(response)
}

fn method_name(method: &Method) -> String {
    match method {
        Method::Get => "GET".into(),
        Method::Head => "HEAD".into(),
        Method::Post => "POST".into(),
        Method::Put => "PUT".into(),
        Method::Delete => "DELETE".into(),
        Method::Connect => "CONNECT".into(),
        Method::Options => "OPTIONS".into(),
        Method::Trace => "TRACE".into(),
        Method::Patch => "PATCH".into(),
        Method::NonStandard(value) => value.as_str().to_string(),
    }
}

fn header_value(request: &Request, name: &'static str) -> Option<String> {
    request.headers().iter().find_map(|header| {
        if header.field.equiv(name) {
            Some(header.value.as_str().to_string())
        } else {
            None
        }
    })
}

fn bearer_matches(authorization: Option<&str>, expected: &str) -> bool {
    let Some(header) = authorization else {
        return false;
    };
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .unwrap_or("");
    if token.len() != expected.len() {
        return false;
    }
    token
        .as_bytes()
        .iter()
        .zip(expected.as_bytes())
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn text(status: u16, message: &str) -> HttpProvisionOutput {
    HttpProvisionOutput {
        status,
        body: serde_json::json!({ "detail": message }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provision::STUB_CONNECTION_ORIGIN;
    use std::thread;

    const TOKEN: &str = "test-host-bootstrap-token";

    fn standard_body() -> Vec<u8> {
        serde_json::json!({ "labTier": "standard", "sharedGpuId": null })
            .to_string()
            .into_bytes()
    }

    #[test]
    fn linux_standard_returns_stub_url_without_invoking_a_hypervisor() {
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &standard_body(),
            TOKEN,
            HostOs::Linux,
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/firecracker")
        );
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(body["gpuExclusive"], false);
    }

    #[test]
    fn windows_standard_returns_a_peer_stub_url() {
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &standard_body(),
            TOKEN,
            HostOs::Windows,
        );
        assert_eq!(output.status, 200);
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/qemu-whpx")
        );
        assert_eq!(body["hypervisorInvoked"], false);
    }

    #[test]
    fn macos_does_not_pretend_to_host() {
        let output = respond(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &standard_body(),
            TOKEN,
            HostOs::MacOs,
        );
        assert_eq!(output.status, 400);
        assert!(output.body.contains("macOS"));
        let body: serde_json::Value = serde_json::from_str(&output.body).unwrap();
        assert!(body.get("connectionUrl").is_none());
    }

    #[test]
    fn missing_bearer_is_unauthorized() {
        let output = respond(
            "POST",
            "/v1/provision",
            None,
            &standard_body(),
            TOKEN,
            HostOs::Linux,
        );
        assert_eq!(output.status, 401);
    }

    #[test]
    fn wrong_bearer_is_unauthorized() {
        let output = respond(
            "POST",
            "/v1/provision",
            Some("Bearer other-token"),
            &standard_body(),
            TOKEN,
            HostOs::Linux,
        );
        assert_eq!(output.status, 401);
    }

    #[test]
    fn other_paths_are_not_found() {
        let output = respond(
            "POST",
            "/v1/hosts",
            Some(&format!("Bearer {TOKEN}")),
            &standard_body(),
            TOKEN,
            HostOs::Linux,
        );
        assert_eq!(output.status, 404);
    }

    #[test]
    fn listen_config_skips_when_addr_is_unset() {
        let result = listen_config_from_env(|key| match key {
            "HOST_BOOTSTRAP_TOKEN" => Some(TOKEN.to_string()),
            _ => None,
        })
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn listen_config_fails_fast_without_a_token() {
        let error = listen_config_from_env(|key| match key {
            "HOST_AGENT_LISTEN_ADDR" => Some("127.0.0.1:7842".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, ProvisionListenError::MissingToken);
        assert!(!error.to_string().contains(TOKEN));
    }

    #[test]
    fn listen_config_debug_redacts_the_token() {
        let config = ListenConfig::new("127.0.0.1:7842", TOKEN);
        let debug = format!("{config:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains(TOKEN));
    }

    #[test]
    fn http_bind_round_trip_stays_a_stub() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server
            .server_addr()
            .to_ip()
            .expect("provision test listens on TCP");
        thread::spawn(move || {
            let occupancy = Occupancy::new();
            let request = server.recv().unwrap();
            dispatch(request, TOKEN, HostOs::Linux, &occupancy).unwrap();
        });

        let agent = ureq::AgentBuilder::new().redirects(0).build();
        let response = agent
            .post(&format!("http://{addr}/v1/provision"))
            .set("Authorization", &format!("Bearer {TOKEN}"))
            .send_json(serde_json::json!({ "labTier": "standard" }))
            .unwrap();
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.into_json().unwrap();
        assert_eq!(body["hypervisorInvoked"], false);
        assert_eq!(
            body["connectionUrl"],
            format!("{STUB_CONNECTION_ORIGIN}/firecracker")
        );
    }

    #[test]
    fn successful_provision_marks_local_occupancy() {
        let occupancy = Occupancy::new();
        let output = respond_and_track(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &standard_body(),
            TOKEN,
            HostOs::Linux,
            &occupancy,
        );
        assert_eq!(output.status, 200);
        assert!(occupancy.is_in_session());
    }

    #[test]
    fn rejected_provision_does_not_mark_occupancy() {
        let occupancy = Occupancy::new();
        let output = respond_and_track(
            "POST",
            "/v1/provision",
            None,
            &standard_body(),
            TOKEN,
            HostOs::Linux,
            &occupancy,
        );
        assert_eq!(output.status, 401);
        assert!(!occupancy.is_in_session());
    }

    #[test]
    fn macos_reject_does_not_mark_occupancy() {
        let occupancy = Occupancy::new();
        let output = respond_and_track(
            "POST",
            "/v1/provision",
            Some(&format!("Bearer {TOKEN}")),
            &standard_body(),
            TOKEN,
            HostOs::MacOs,
            &occupancy,
        );
        assert_eq!(output.status, 400);
        assert!(!occupancy.is_in_session());
    }
}
