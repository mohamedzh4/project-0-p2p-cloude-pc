//! Live Kasm / Sunshine locators after a successful invoke or GPU apply.
//!
//! Never invent a public URL. Construct from the session's Headscale guest
//! hostname (not a LAN IP) and poll until that URL answers, or fail. Tests
//! inject a loopback HTTP origin or a fake guest probe.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

/// KasmVNC browser desktop HTTPS listener in the Standard guest.
pub const KASM_HTTPS_PORT: u16 = 8443;

/// Sunshine HTTPS web UI. GameStream/Moonlight base port is 47989; the
/// configuration UI is 47990 (exposed in LizardByte's documented port range).
pub const SUNSHINE_WEBUI_PORT: u16 = 47990;

/// Sunshine GameStream HTTP port. The Host Agent probes this one, not the web
/// UI: the web UI can change credentials and run commands in the guest, so the
/// mesh ACL lets the host reach 47989 and never 47990. `/serverinfo` needs no
/// login; any HTTP answer, or an open port, counts as "up".
pub const SUNSHINE_PROBE_PORT: u16 = 47989;

pub const MOONLIGHT_APP: &str = "Desktop";

// A cold browser-desktop guest can need several minutes to boot even after QEMU
// starts successfully. The Matchmaker request timeout is longer than this.
const PRODUCTION_TIMEOUT: Duration = Duration::from_secs(300);
const PRODUCTION_INTERVAL: Duration = Duration::from_secs(1);
const HTTP_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamerKind {
    Kasm,
    Sunshine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveLocator {
    pub connection_url: String,
    pub sunshine_url: Option<String>,
    pub moonlight_host: Option<String>,
    pub moonlight_app: Option<String>,
    pub streamer: StreamerKind,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LocatorError {
    #[error(
        "guest mesh hostname is required to construct a live locator; refusing to invent a public URL"
    )]
    MissingMeshHostname,
    #[error("refusing placeholder locator {0}; a live streamer URL is required after provision")]
    Placeholder(String),
    #[error("streamer at {url} was not reachable in time")]
    Unreachable { url: String },
}

/// Probe a candidate locator. Tests inject a loopback server or a fake guest.
pub trait StreamerProbe: Send + Sync {
    fn is_reachable(&self, url: &str) -> bool;
}

/// GET the locator. Any HTTP status (including 401 on a login page) means the
/// streamer is up. TLS/connect failure falls back to TCP so Sunshine/Kasm
/// self-signed certs on the mesh still count as reachable.
pub struct HttpStreamerProbe {
    pub request_timeout: Duration,
}

impl Default for HttpStreamerProbe {
    fn default() -> Self {
        Self {
            request_timeout: HTTP_PROBE_TIMEOUT,
        }
    }
}

impl StreamerProbe for HttpStreamerProbe {
    fn is_reachable(&self, url: &str) -> bool {
        let Some(host) = hostname_from_url(url) else {
            return false;
        };
        if cfg!(test) && !is_loopback_host(&host) {
            // cargo test must not hit the network or treat a mesh name as live.
            return false;
        }
        if http_responded(url, self.request_timeout) {
            return true;
        }
        tcp_open(url, self.request_timeout)
    }
}

#[derive(Clone)]
pub struct StreamerWait {
    pub timeout: Duration,
    pub interval: Duration,
    pub probe: Arc<dyn StreamerProbe>,
    /// Tests inject a loopback origin. Production leaves this unset and
    /// constructs from the guest mesh hostname.
    pub override_url: Option<String>,
}

impl std::fmt::Debug for StreamerWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamerWait")
            .field("timeout", &self.timeout)
            .field("interval", &self.interval)
            .field("override_url", &self.override_url)
            .field("probe", &"StreamerProbe")
            .finish()
    }
}

impl StreamerWait {
    pub fn production() -> Self {
        Self {
            timeout: PRODUCTION_TIMEOUT,
            interval: PRODUCTION_INTERVAL,
            probe: Arc::new(HttpStreamerProbe::default()),
            override_url: None,
        }
    }

    /// Tests: poll this exact URL (loopback HTTP) and return it when reachable.
    pub fn injected_http(url: impl Into<String>) -> Self {
        Self {
            timeout: Duration::from_secs(2),
            interval: Duration::from_millis(20),
            probe: Arc::new(HttpStreamerProbe {
                request_timeout: Duration::from_millis(300),
            }),
            override_url: Some(url.into()),
        }
    }

    /// Tests: streamer never answers. Not a fake public success URL.
    pub fn unreachable() -> Self {
        Self {
            timeout: Duration::from_millis(40),
            interval: Duration::from_millis(10),
            probe: Arc::new(NeverReady),
            override_url: Some("http://127.0.0.1:9/".to_string()),
        }
    }

    fn fail_fast() -> Self {
        Self {
            timeout: Duration::from_millis(20),
            interval: Duration::from_millis(5),
            probe: Arc::new(HttpStreamerProbe {
                request_timeout: Duration::from_millis(10),
            }),
            override_url: None,
        }
    }
}

/// Used when invoke/apply ran but the gate did not inject a probe.
pub fn wait_from_gate(wait: Option<&StreamerWait>) -> StreamerWait {
    wait.cloned().unwrap_or_else(|| {
        if cfg!(test) {
            StreamerWait::fail_fast()
        } else {
            StreamerWait::production()
        }
    })
}

pub fn kasm_workspace_url(mesh_hostname: &str) -> String {
    format!(
        "https://{}:{KASM_HTTPS_PORT}/",
        host_for_url(mesh_hostname.trim())
    )
}

pub fn sunshine_web_url(mesh_hostname: &str) -> String {
    format!(
        "https://{}:{SUNSHINE_WEBUI_PORT}/",
        host_for_url(mesh_hostname.trim())
    )
}

/// What the Host Agent polls for readiness. Kasm is probed at its own URL.
/// Sunshine is probed on the GameStream port, not the web UI (see
/// [`SUNSHINE_PROBE_PORT`]); the returned locator is unchanged.
pub fn sunshine_probe_url(mesh_hostname: &str) -> String {
    format!(
        "http://{}:{SUNSHINE_PROBE_PORT}/serverinfo",
        host_for_url(mesh_hostname.trim())
    )
}

pub fn locator_is_placeholder(url: &str) -> bool {
    let Some(host) = hostname_from_url(url) else {
        return true;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "invalid" || host.ends_with(".invalid")
}

pub fn await_live_locator(
    kind: StreamerKind,
    mesh_hostname: Option<&str>,
    wait: &StreamerWait,
) -> Result<LiveLocator, LocatorError> {
    let url = candidate_url(kind, mesh_hostname, wait)?;
    if locator_is_placeholder(&url) {
        return Err(LocatorError::Placeholder(url));
    }
    let probe_url = match (kind, wait.override_url.as_deref(), mesh_hostname) {
        (StreamerKind::Sunshine, None, Some(host)) => sunshine_probe_url(host),
        _ => url.clone(),
    };
    await_reachable(&probe_url, wait)?;
    // Return a concrete tailnet IP when possible. The Host Agent has native
    // Tailscale DNS, while the containerized public gateway may not inherit
    // the host resolver even though it shares the Tailscale network stack.
    let gateway_url = if wait.override_url.is_none() {
        mesh_hostname
            .and_then(|host| resolved_mesh_url(kind, host))
            .unwrap_or(url)
    } else {
        url
    };
    Ok(live_from_url(kind, gateway_url))
}

fn resolved_mesh_url(kind: StreamerKind, mesh_hostname: &str) -> Option<String> {
    let port = match kind {
        StreamerKind::Kasm => KASM_HTTPS_PORT,
        StreamerKind::Sunshine => SUNSHINE_WEBUI_PORT,
    };
    let mut addresses = (mesh_hostname.trim(), port).to_socket_addrs().ok()?;
    let address = addresses
        .find(|address| address.is_ipv4())
        .or_else(|| addresses.next())?;
    Some(format!("https://{}:{port}/", host_for_url(&address.ip().to_string())))
}

fn candidate_url(
    kind: StreamerKind,
    mesh_hostname: Option<&str>,
    wait: &StreamerWait,
) -> Result<String, LocatorError> {
    if let Some(over) = wait
        .override_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(over.to_string());
    }
    let host = mesh_hostname
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(LocatorError::MissingMeshHostname)?;
    Ok(match kind {
        StreamerKind::Kasm => kasm_workspace_url(host),
        StreamerKind::Sunshine => sunshine_web_url(host),
    })
}

fn await_reachable(url: &str, wait: &StreamerWait) -> Result<(), LocatorError> {
    let deadline = Instant::now() + wait.timeout;
    loop {
        if wait.probe.is_reachable(url) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(LocatorError::Unreachable {
                url: url.to_string(),
            });
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        thread::sleep(wait.interval.min(remaining));
    }
}

fn live_from_url(kind: StreamerKind, url: String) -> LiveLocator {
    match kind {
        StreamerKind::Kasm => LiveLocator {
            connection_url: url,
            sunshine_url: None,
            moonlight_host: None,
            moonlight_app: None,
            streamer: StreamerKind::Kasm,
        },
        StreamerKind::Sunshine => {
            let host = hostname_from_url(&url).unwrap_or_default();
            LiveLocator {
                connection_url: url.clone(),
                sunshine_url: Some(url),
                moonlight_host: Some(host),
                moonlight_app: Some(MOONLIGHT_APP.to_string()),
                streamer: StreamerKind::Sunshine,
            }
        }
    }
}

fn host_for_url(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

pub fn hostname_from_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next()?.to_string()
    } else {
        authority
            .rsplit_once('@')
            .map(|(_, hostport)| hostport)
            .unwrap_or(authority)
            .split(':')
            .next()?
            .to_string()
    };
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

fn port_from_url(url: &str) -> u16 {
    let https = url.starts_with("https://");
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or(rest);
    if let Some(inner) = authority.strip_prefix('[') {
        if let Some((_, port)) = inner.split_once("]:") {
            if let Ok(parsed) = port.split('/').next().unwrap_or(port).parse() {
                return parsed;
            }
        }
        return if https { 443 } else { 80 };
    }
    if let Some((_, port)) = authority.rsplit_once(':') {
        if let Ok(parsed) = port.parse() {
            return parsed;
        }
    }
    if https {
        443
    } else {
        80
    }
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

fn http_responded(url: &str, timeout: Duration) -> bool {
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(0)
        .build();
    match agent.get(url).call() {
        Ok(_) => true,
        Err(ureq::Error::Status(_, _)) => true,
        Err(_) => false,
    }
}

fn tcp_open(url: &str, timeout: Duration) -> bool {
    let Some(host) = hostname_from_url(url) else {
        return false;
    };
    let port = port_from_url(url);
    let Ok(mut addrs) = (host.as_str(), port).to_socket_addrs() else {
        return false;
    };
    addrs.any(|addr| TcpStream::connect_timeout(&addr, timeout).is_ok())
}

struct NeverReady;

impl StreamerProbe for NeverReady {
    fn is_reachable(&self, _url: &str) -> bool {
        false
    }
}

#[cfg(test)]
struct AlwaysReady;

#[cfg(test)]
impl StreamerProbe for AlwaysReady {
    fn is_reachable(&self, _url: &str) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_http::{Response, Server};

    fn spawn_ok_http() -> String {
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server
            .server_addr()
            .to_ip()
            .expect("locator test listens on TCP");
        thread::spawn(move || {
            for request in server.incoming_requests() {
                let _ = request.respond(Response::from_string("ok"));
            }
        });
        format!("http://{addr}/")
    }

    #[test]
    fn kasm_url_uses_mesh_hostname_and_https_8443() {
        assert_eq!(
            kasm_workspace_url("dcl-guest-aaaa"),
            "https://dcl-guest-aaaa:8443/"
        );
        assert_eq!(KASM_HTTPS_PORT, 8443);
    }

    #[test]
    fn sunshine_is_probed_on_47989_never_the_web_ui_port() {
        assert_eq!(
            sunshine_probe_url("dcl-guest-bbbb"),
            "http://dcl-guest-bbbb:47989/serverinfo"
        );
        assert_ne!(SUNSHINE_PROBE_PORT, SUNSHINE_WEBUI_PORT);
        assert_eq!(port_from_url(&sunshine_probe_url("dcl-guest-bbbb")), 47989);
    }

    #[test]
    fn sunshine_wait_probes_47989_but_returns_the_web_ui_locator() {
        use std::sync::Mutex;
        struct Recording(Mutex<Vec<String>>);
        impl StreamerProbe for Recording {
            fn is_reachable(&self, url: &str) -> bool {
                self.0.lock().unwrap().push(url.to_string());
                true
            }
        }
        let probe = Arc::new(Recording(Mutex::new(Vec::new())));
        let wait = StreamerWait {
            timeout: Duration::from_millis(50),
            interval: Duration::from_millis(5),
            probe: probe.clone(),
            override_url: None,
        };
        let live = await_live_locator(StreamerKind::Sunshine, Some("dcl-guest-eeee"), &wait).unwrap();
        let probed = probe.0.lock().unwrap().clone();
        assert_eq!(probed, vec!["http://dcl-guest-eeee:47989/serverinfo".to_string()]);
        assert!(probed.iter().all(|u| !u.contains("47990")));
        assert!(live.connection_url.contains(":47990"));
    }

    #[test]
    fn sunshine_url_uses_mesh_hostname_and_web_ui_47990() {
        assert_eq!(
            sunshine_web_url("dcl-guest-bbbb"),
            "https://dcl-guest-bbbb:47990/"
        );
        assert_eq!(SUNSHINE_WEBUI_PORT, 47990);
    }

    #[test]
    fn placeholder_detects_rfc2606_invalid() {
        assert!(locator_is_placeholder("https://provision.invalid/qemu-kvm"));
        assert!(locator_is_placeholder("https://example.invalid/kasm"));
        assert!(locator_is_placeholder("https://provision.invalid/sunshine"));
        assert!(!locator_is_placeholder("https://kasm.test.example/desktop"));
        assert!(!locator_is_placeholder("http://127.0.0.1:8443/"));
    }

    #[test]
    fn mesh_kasm_locator_when_probe_succeeds() {
        let wait = StreamerWait {
            timeout: Duration::from_millis(50),
            interval: Duration::from_millis(5),
            probe: Arc::new(AlwaysReady),
            override_url: None,
        };
        let live = await_live_locator(StreamerKind::Kasm, Some("dcl-guest-cccc"), &wait).unwrap();
        assert_eq!(live.connection_url, "https://dcl-guest-cccc:8443/");
        assert_eq!(live.streamer, StreamerKind::Kasm);
        assert!(live.sunshine_url.is_none());
        assert!(!locator_is_placeholder(&live.connection_url));
    }

    #[test]
    fn resolved_mesh_locator_uses_a_concrete_ip_for_the_gateway() {
        assert_eq!(
            resolved_mesh_url(StreamerKind::Kasm, "127.0.0.1").as_deref(),
            Some("https://127.0.0.1:8443/")
        );
    }

    #[test]
    fn mesh_sunshine_locator_when_probe_succeeds() {
        let wait = StreamerWait {
            timeout: Duration::from_millis(50),
            interval: Duration::from_millis(5),
            probe: Arc::new(AlwaysReady),
            override_url: None,
        };
        let live =
            await_live_locator(StreamerKind::Sunshine, Some("dcl-guest-dddd"), &wait).unwrap();
        assert_eq!(live.sunshine_url.as_deref(), Some("https://dcl-guest-dddd:47990/"));
        assert_eq!(live.moonlight_host.as_deref(), Some("dcl-guest-dddd"));
        assert_eq!(live.moonlight_app.as_deref(), Some(MOONLIGHT_APP));
        assert_eq!(live.streamer, StreamerKind::Sunshine);
    }

    #[test]
    fn missing_mesh_hostname_does_not_invent_a_url() {
        let wait = StreamerWait {
            timeout: Duration::from_millis(10),
            interval: Duration::from_millis(5),
            probe: Arc::new(AlwaysReady),
            override_url: None,
        };
        let error = await_live_locator(StreamerKind::Kasm, None, &wait).unwrap_err();
        assert_eq!(error, LocatorError::MissingMeshHostname);
    }

    #[test]
    fn injected_loopback_http_is_returned_when_reachable() {
        let url = spawn_ok_http();
        let live =
            await_live_locator(StreamerKind::Kasm, Some("dcl-guest-ignored"), &StreamerWait::injected_http(&url))
                .unwrap();
        assert_eq!(live.connection_url, url);
        assert!(!locator_is_placeholder(&live.connection_url));
    }

    #[test]
    fn timeout_when_streamer_never_answers() {
        let error = await_live_locator(StreamerKind::Kasm, None, &StreamerWait::unreachable())
            .unwrap_err();
        assert!(matches!(error, LocatorError::Unreachable { .. }));
    }

    #[test]
    fn placeholder_override_is_refused() {
        let wait = StreamerWait {
            timeout: Duration::from_millis(10),
            interval: Duration::from_millis(5),
            probe: Arc::new(AlwaysReady),
            override_url: Some("https://provision.invalid/kasm".to_string()),
        };
        let error = await_live_locator(StreamerKind::Kasm, Some("dcl-guest"), &wait).unwrap_err();
        assert!(matches!(error, LocatorError::Placeholder(_)));
    }

    #[test]
    fn http_probe_treats_401_as_reachable() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        thread::spawn(move || {
            for request in server.incoming_requests() {
                let _ = request.respond(
                    Response::from_string("login").with_status_code(tiny_http::StatusCode(401)),
                );
            }
        });
        let url = format!("http://{addr}/");
        assert!(HttpStreamerProbe::default().is_reachable(&url));
    }

    #[test]
    fn http_probe_does_not_treat_closed_loopback_as_live() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let url = format!("http://127.0.0.1:{port}/");
        assert!(!HttpStreamerProbe {
            request_timeout: Duration::from_millis(50),
        }
        .is_reachable(&url));
    }
}
