//! A plain-text report a host can copy and send when something fails.
//!
//! It holds versions, what this machine can host, and the outcome of the
//! last provision request. It never holds a token or a secret: values of
//! secret environment variables are only reported as "set" or "not set",
//! and any that appear inside an error message are blanked.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::identity::HostRegistration;

const MAX_DETAIL_CHARS: usize = 800;

/// Environment variables whose presence matters. `secret` ones are never
/// printed, only whether they are set.
const REPORTED_ENV: &[(&str, bool)] = &[
    ("MATCHMAKER_URL", false),
    ("HOST_BOOTSTRAP_TOKEN", true),
    ("HOST_AGENT_LISTEN_ADDR", false),
    ("HOST_AGENT_URL", false),
    ("HEADSCALE_LOGIN_SERVER", false),
    ("HOST_AGENT_STANDARD_IMAGE_URL", false),
    ("IMAGE_SIGNING_PUBLIC_KEY", false),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionOutcome {
    pub unix_seconds: u64,
    pub path: String,
    pub status: u16,
    /// Response body for failures, shortened. Empty on success.
    pub detail: String,
}

/// Last provision/teardown outcome, shared with the listener thread.
#[derive(Debug, Default)]
pub struct ProvisionLog {
    last: Mutex<Option<ProvisionOutcome>>,
    last_failure: Mutex<Option<ProvisionOutcome>>,
}

impl ProvisionLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, path: &str, status: u16, body: &str) {
        let path = path.split('?').next().unwrap_or(path).to_string();
        let failed = status != 200;
        let outcome = ProvisionOutcome {
            unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0),
            path,
            status,
            detail: if failed { shorten(body) } else { String::new() },
        };
        if failed {
            if let Ok(mut slot) = self.last_failure.lock() {
                *slot = Some(outcome.clone());
            }
        }
        if let Ok(mut slot) = self.last.lock() {
            *slot = Some(outcome);
        }
    }

    pub fn last(&self) -> Option<ProvisionOutcome> {
        self.last.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn last_failure(&self) -> Option<ProvisionOutcome> {
        self.last_failure.lock().ok().and_then(|slot| slot.clone())
    }
}

fn shorten(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= MAX_DETAIL_CHARS {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX_DETAIL_CHARS).collect();
    out.push_str("… (shortened)");
    out
}

pub struct ReportInput<'a, C: Serialize, H: Serialize, G: Serialize> {
    pub agent_version: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
    pub capabilities: &'a C,
    pub hypervisor: &'a H,
    pub guest_image: &'a G,
    pub registration: Option<&'a HostRegistration>,
    pub sharing_enabled: Option<bool>,
    pub in_session: bool,
    pub log: &'a ProvisionLog,
}

pub fn build_report<C: Serialize, H: Serialize, G: Serialize>(
    input: &ReportInput<'_, C, H, G>,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> String {
    let mut secrets: Vec<String> = Vec::new();
    let mut out = String::new();
    out.push_str("Distributed Compute Lab — Host Agent report\n");
    out.push_str(&format!("agent version: {}\n", input.agent_version));
    out.push_str(&format!("system: {} {}\n", input.os, input.arch));

    out.push_str("\n[registration]\n");
    match input.registration {
        Some(registration) => {
            out.push_str(&format!("host id: {}\n", registration.host_id));
            out.push_str(&format!("status: {}\n", registration.status));
            out.push_str(&format!("tier: {}\n", registration.lab_tier));
        }
        None => out.push_str("not registered\n"),
    }
    out.push_str(&format!(
        "sharing: {}\n",
        match input.sharing_enabled {
            Some(true) => "on",
            Some(false) => "paused",
            None => "not configured",
        }
    ));
    out.push_str(&format!("in session: {}\n", input.in_session));

    out.push_str("\n[settings]\n");
    for (name, secret) in REPORTED_ENV {
        let value = lookup(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let shown = match (&value, secret) {
            (None, _) => "not set".to_string(),
            (Some(value), true) => {
                secrets.push(value.clone());
                "set".to_string()
            }
            (Some(value), false) => value.clone(),
        };
        out.push_str(&format!("{name}: {shown}\n"));
    }

    out.push_str("\n[last provision request]\n");
    out.push_str(&outcome_lines(input.log.last()));
    out.push_str("\n[last failed provision request]\n");
    out.push_str(&outcome_lines(input.log.last_failure()));

    out.push_str("\n[capabilities]\n");
    out.push_str(&pretty(input.capabilities));
    out.push_str("\n[hypervisor]\n");
    out.push_str(&pretty(input.hypervisor));
    out.push_str("\n[guest image]\n");
    out.push_str(&pretty(input.guest_image));

    for secret in secrets {
        out = out.replace(&secret, "[redacted]");
    }
    out
}

fn outcome_lines(outcome: Option<ProvisionOutcome>) -> String {
    match outcome {
        None => "none since the agent started\n".to_string(),
        Some(outcome) => {
            let mut lines = format!(
                "time (unix seconds): {}\nrequest: {}\nresult: HTTP {}\n",
                outcome.unix_seconds, outcome.path, outcome.status
            );
            if !outcome.detail.is_empty() {
                lines.push_str(&format!("detail: {}\n", outcome.detail));
            }
            lines
        }
    }
}

fn pretty<T: Serialize>(value: &T) -> String {
    match serde_json::to_string_pretty(value) {
        Ok(text) => format!("{text}\n"),
        Err(_) => "unavailable\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration() -> HostRegistration {
        HostRegistration {
            host_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
            display_name: "lab-pc".to_string(),
            lab_tier: "standard".to_string(),
            status: "approved".to_string(),
            shared_logical_cores: 4,
            shared_memory_bytes: 8,
            share_gpu: false,
            shared_gpu_id: None,
        }
    }

    fn report(log: &ProvisionLog, registration: Option<&HostRegistration>) -> String {
        let input = ReportInput {
            agent_version: "0.1.0",
            os: "linux",
            arch: "x86_64",
            capabilities: &serde_json::json!({"kvm": true}),
            hypervisor: &serde_json::json!({"state": "ready"}),
            guest_image: &serde_json::json!({"state": "missing"}),
            registration,
            sharing_enabled: Some(true),
            in_session: false,
            log,
        };
        build_report(&input, |key| match key {
            "MATCHMAKER_URL" => Some("https://mm.example".to_string()),
            "HOST_BOOTSTRAP_TOKEN" => Some("super-secret-bootstrap".to_string()),
            _ => None,
        })
    }

    #[test]
    fn report_names_versions_registration_and_settings() {
        let log = ProvisionLog::new();
        let text = report(&log, Some(&registration()));

        assert!(text.contains("agent version: 0.1.0"));
        assert!(text.contains("system: linux x86_64"));
        assert!(text.contains("host id: aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
        assert!(text.contains("status: approved"));
        assert!(text.contains("MATCHMAKER_URL: https://mm.example"));
        assert!(text.contains("HOST_AGENT_LISTEN_ADDR: not set"));
        assert!(text.contains("\"kvm\": true"));
        assert!(text.contains("none since the agent started"));
    }

    #[test]
    fn secret_settings_are_reported_as_set_never_printed() {
        let log = ProvisionLog::new();
        let text = report(&log, None);

        assert!(text.contains("HOST_BOOTSTRAP_TOKEN: set"));
        assert!(!text.contains("super-secret-bootstrap"));
        assert!(text.contains("not registered"));
    }

    #[test]
    fn a_secret_inside_an_error_message_is_blanked() {
        let log = ProvisionLog::new();
        log.record(
            "/v1/provision",
            500,
            "boot failed with token super-secret-bootstrap in argv",
        );
        let text = report(&log, None);

        assert!(text.contains("result: HTTP 500"));
        assert!(text.contains("boot failed with token [redacted] in argv"));
        assert!(!text.contains("super-secret-bootstrap"));
    }

    #[test]
    fn the_last_failure_survives_a_later_success() {
        let log = ProvisionLog::new();
        log.record("/v1/provision?x=1", 503, "QEMU did not start");
        log.record("/v1/teardown", 200, "{\"ok\":true}");

        let last = log.last().unwrap();
        assert_eq!((last.path.as_str(), last.status), ("/v1/teardown", 200));
        assert!(last.detail.is_empty(), "success bodies are not kept");
        let failure = log.last_failure().unwrap();
        assert_eq!(failure.path, "/v1/provision");
        assert_eq!(failure.detail, "QEMU did not start");
    }

    #[test]
    fn long_error_bodies_are_shortened() {
        let log = ProvisionLog::new();
        log.record("/v1/provision", 500, &"x".repeat(5000));

        let detail = log.last_failure().unwrap().detail;
        assert!(detail.chars().count() < 900);
        assert!(detail.ends_with("(shortened)"));
    }
}
