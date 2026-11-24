//! Fetch commands from the Matchmaker instead of waiting to be dialled.
//!
//! A volunteer's PC is behind a home router, so the Matchmaker cannot reach
//! a listener on it. When `HOST_AGENT_LISTEN_ADDR` is not set the agent
//! asks for work itself every [`POLL_INTERVAL`]:
//!
//! ```text
//! POST /v1/hosts/{id}/commands/poll           -> 204, or one command
//! POST /v1/hosts/{id}/commands/{cmd}/result   -> what happened
//! ```
//!
//! A command is run through exactly the same handler as a request to the
//! provision listener, including the Bearer check: the Matchmaker puts this
//! host's command token in the command, and the handler refuses anything
//! else. So nothing here can start a guest that the listener would refuse.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::agent_status::Occupancy;
use crate::diagnostics::ProvisionLog;
use crate::identity;
use crate::image::ImageTrust;
use crate::matchmaker::{truncate_detail, MatchmakerCredentials, MatchmakerError, Transport};
use crate::provision::HostOs;
use crate::provision_http::{
    respond_and_track_with_gate, HttpProvisionOutput, ProvisionGate, PROVISION_PATH, TEARDOWN_PATH,
};

/// How often to ask. A booking waits at most this long before the boot starts.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

const KIND_PROVISION: &str = "provision";
const KIND_TEARDOWN: &str = "teardown";
const MAX_COMMAND_ID_CHARS: usize = 64;
const REPORT_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolledCommand {
    pub command_id: String,
    pub kind: String,
    /// Bearer the handler checks. Never logged.
    command_token: String,
    #[serde(default)]
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    /// This machine has no host id yet.
    NotRegistered,
    /// The Matchmaker had nothing for this host.
    Idle,
    /// A command ran and its outcome was reported.
    Ran { kind: String, status: u16 },
}

fn poll_url(base: &str, host_id: &str) -> String {
    format!("{base}/v1/hosts/{host_id}/commands/poll")
}

fn result_url(base: &str, host_id: &str, command_id: &str) -> String {
    format!("{base}/v1/hosts/{host_id}/commands/{command_id}/result")
}

/// The id goes into a URL path; accept only what a UUID is made of.
fn is_command_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_COMMAND_ID_CHARS
        && value.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Ask for the next command. `Ok(None)` when there is none or this machine
/// is not registered yet.
pub fn poll(
    dir: &Path,
    lookup: impl FnMut(&str) -> Option<String>,
    transport: &impl Transport,
) -> Result<Option<(String, PolledCommand)>, MatchmakerError> {
    let Some(identity) = identity::load(dir)? else {
        return Ok(None);
    };
    let credentials = MatchmakerCredentials::from_lookup(lookup)?.with_host_secret(dir)?;
    let request = credentials.request(
        "POST",
        poll_url(&credentials.base_url, &identity.host_id),
        serde_json::json!({}),
    );
    let response = transport.send(&request)?;
    match response.status {
        204 => Ok(None),
        200 => {
            let command: PolledCommand = serde_json::from_str(&response.body)
                .map_err(|_| MatchmakerError::UnexpectedResponse { status: 200 })?;
            if !is_command_id(&command.command_id) {
                return Err(MatchmakerError::UnexpectedResponse { status: 200 });
            }
            Ok(Some((identity.host_id, command)))
        }
        status => Err(MatchmakerError::Rejected {
            status,
            detail: truncate_detail(&response.body),
        }),
    }
}

/// Run one command through the provision handler. The handler checks the
/// command's token against `expected_token`.
pub fn execute(
    command: &PolledCommand,
    expected_token: &str,
    os: HostOs,
    occupancy: &Occupancy,
    image_trust: &ImageTrust,
    gate: Option<&ProvisionGate>,
) -> HttpProvisionOutput {
    let (path, body) = match command.kind.as_str() {
        KIND_PROVISION => (
            PROVISION_PATH,
            serde_json::to_vec(&command.body).unwrap_or_default(),
        ),
        KIND_TEARDOWN => (TEARDOWN_PATH, Vec::new()),
        _ => {
            return HttpProvisionOutput {
                status: 400,
                body: "unknown command kind".to_string(),
            }
        }
    };
    let authorization = format!("Bearer {}", command.command_token);
    respond_and_track_with_gate(
        "POST",
        path,
        Some(&authorization),
        &body,
        expected_token,
        os,
        occupancy,
        image_trust,
        gate,
    )
}

/// Tell the Matchmaker what happened. Retried: a lost report fails a booking
/// whose guest is already running.
pub fn report(
    dir: &Path,
    mut lookup: impl FnMut(&str) -> Option<String>,
    transport: &impl Transport,
    host_id: &str,
    command_id: &str,
    output: &HttpProvisionOutput,
    sleep: impl Fn(Duration),
) -> Result<(), MatchmakerError> {
    let credentials = MatchmakerCredentials::from_lookup(&mut lookup)?.with_host_secret(dir)?;
    let request = credentials.request(
        "POST",
        result_url(&credentials.base_url, host_id, command_id),
        serde_json::json!({ "status": output.status, "body": output.body }),
    );
    let mut last = MatchmakerError::Network("the result was never sent".to_string());
    for attempt in 0..REPORT_ATTEMPTS {
        match transport.send(&request) {
            Ok(response) if response.status == 204 || response.status == 200 => return Ok(()),
            // The Matchmaker no longer wants it (withdrawn, or already answered).
            Ok(response) if response.status == 404 => {
                return Err(MatchmakerError::Rejected {
                    status: 404,
                    detail: truncate_detail(&response.body),
                })
            }
            Ok(response) => {
                last = MatchmakerError::Rejected {
                    status: response.status,
                    detail: truncate_detail(&response.body),
                }
            }
            Err(error) => last = error,
        }
        if attempt + 1 < REPORT_ATTEMPTS {
            sleep(POLL_INTERVAL);
        }
    }
    Err(last)
}

/// One round: ask, run, report.
#[allow(clippy::too_many_arguments)]
pub fn tick(
    dir: &Path,
    lookup: &dyn Fn(&str) -> Option<String>,
    transport: &impl Transport,
    expected_token: &str,
    os: HostOs,
    occupancy: &Occupancy,
    image_trust: &ImageTrust,
    gate: Option<&ProvisionGate>,
    log: Option<&ProvisionLog>,
    sleep: impl Fn(Duration),
) -> Result<TickOutcome, MatchmakerError> {
    if identity::load(dir)?.is_none() {
        return Ok(TickOutcome::NotRegistered);
    }
    let Some((host_id, command)) = poll(dir, |key| lookup(key), transport)? else {
        return Ok(TickOutcome::Idle);
    };
    let output = execute(&command, expected_token, os, occupancy, image_trust, gate);
    if let Some(log) = log {
        let path = if command.kind == KIND_TEARDOWN {
            TEARDOWN_PATH
        } else {
            PROVISION_PATH
        };
        log.record(path, output.status, &output.body);
    }
    report(
        dir,
        |key| lookup(key),
        transport,
        &host_id,
        &command.command_id,
        &output,
        sleep,
    )?;
    Ok(TickOutcome::Ran {
        kind: command.kind,
        status: output.status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::HostRegistration;
    use crate::image::tests::{signed_fixture, FIXTURE_STANDARD_BYTES};
    use crate::matchmaker::{HttpRequest, HttpResponse};
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use tempfile::tempdir;

    const HOST_ID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    const COMMAND_ID: &str = "11111111-2222-3333-4444-555555555555";

    fn lookup(key: &str) -> Option<String> {
        match key {
            "MATCHMAKER_URL" => Some("https://mm.test.example".to_string()),
            "HOST_BOOTSTRAP_TOKEN" => Some("test-host-bootstrap-token".to_string()),
            _ => None,
        }
    }

    fn register(dir: &Path) {
        identity::save(
            dir,
            &HostRegistration {
                host_id: HOST_ID.to_string(),
                display_name: "lab-pc".to_string(),
                lab_tier: "standard".to_string(),
                status: "approved".to_string(),
                shared_logical_cores: 4,
                shared_memory_bytes: 8,
                share_gpu: false,
                shared_gpu_id: None,
            },
        )
        .unwrap();
    }

    /// Answers in order; records what was sent.
    struct Scripted {
        replies: Mutex<VecDeque<Result<(u16, String), String>>>,
        calls: Mutex<Vec<(String, String, Option<String>, serde_json::Value)>>,
    }

    impl Scripted {
        fn new(replies: Vec<Result<(u16, String), String>>) -> Self {
            Self {
                replies: Mutex::new(replies.into()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, String, Option<String>, serde_json::Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Transport for Scripted {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, MatchmakerError> {
            self.calls.lock().unwrap().push((
                request.method.to_string(),
                request.url.clone(),
                request.host_secret.clone(),
                request.body.clone(),
            ));
            match self.replies.lock().unwrap().pop_front() {
                Some(Ok((status, body))) => Ok(HttpResponse { status, body }),
                Some(Err(detail)) => Err(MatchmakerError::Network(detail)),
                None => panic!("unexpected Matchmaker call"),
            }
        }
    }

    fn command_json(kind: &str, token: &str) -> String {
        serde_json::json!({
            "commandId": COMMAND_ID,
            "kind": kind,
            "commandToken": token,
            "body": {},
        })
        .to_string()
    }

    fn own_token(dir: &Path) -> String {
        crate::host_secret::command_token(&crate::host_secret::load_or_create(dir).unwrap())
    }

    #[test]
    fn an_unregistered_machine_does_not_call_the_matchmaker() {
        let dir = tempdir().unwrap();
        let transport = Scripted::new(vec![]);

        assert!(poll(dir.path(), lookup, &transport).unwrap().is_none());
        assert!(transport.calls().is_empty());
    }

    #[test]
    fn poll_posts_to_this_hosts_queue_with_its_own_secret() {
        let dir = tempdir().unwrap();
        register(dir.path());
        let transport = Scripted::new(vec![Ok((204, String::new()))]);

        assert!(poll(dir.path(), lookup, &transport).unwrap().is_none());

        let calls = transport.calls();
        assert_eq!(calls[0].0, "POST");
        assert_eq!(
            calls[0].1,
            format!("https://mm.test.example/v1/hosts/{HOST_ID}/commands/poll")
        );
        let secret = crate::host_secret::load_or_create(dir.path()).unwrap();
        assert_eq!(calls[0].2.as_deref(), Some(secret.as_str()));
    }

    #[test]
    fn a_command_id_that_is_not_an_id_is_refused() {
        let dir = tempdir().unwrap();
        register(dir.path());
        let hostile = serde_json::json!({
            "commandId": "../../auth/login",
            "kind": "teardown",
            "commandToken": "x",
        })
        .to_string();
        let transport = Scripted::new(vec![Ok((200, hostile))]);

        assert!(matches!(
            poll(dir.path(), lookup, &transport),
            Err(MatchmakerError::UnexpectedResponse { status: 200 })
        ));
    }

    #[test]
    fn a_refused_poll_is_an_error_not_a_command() {
        let dir = tempdir().unwrap();
        register(dir.path());
        let transport = Scripted::new(vec![Ok((403, "{\"detail\":\"no\"}".to_string()))]);

        assert!(matches!(
            poll(dir.path(), lookup, &transport),
            Err(MatchmakerError::Rejected { status: 403, .. })
        ));
    }

    #[test]
    fn a_command_with_another_token_is_refused_like_on_the_listener() {
        let (trust, _) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let occupancy = Occupancy::new();
        occupancy.mark_in_session();
        let command: PolledCommand =
            serde_json::from_str(&command_json("teardown", "another-hosts-token")).unwrap();

        let output = execute(
            &command,
            "this-hosts-command-token",
            HostOs::Linux,
            &occupancy,
            &trust,
            None,
        );

        assert_eq!(output.status, 401);
        assert!(
            occupancy.is_in_session(),
            "a refused teardown changes nothing"
        );
    }

    #[test]
    fn teardown_with_the_right_token_runs() {
        let (trust, _) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let occupancy = Occupancy::new();
        occupancy.mark_in_session();
        let command: PolledCommand =
            serde_json::from_str(&command_json("teardown", "this-hosts-command-token")).unwrap();

        let output = execute(
            &command,
            "this-hosts-command-token",
            HostOs::Linux,
            &occupancy,
            &trust,
            None,
        );

        assert_eq!(output.status, 200);
        assert!(!occupancy.is_in_session());
    }

    #[test]
    fn provision_goes_through_the_same_checks_as_the_listener() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let occupancy = Occupancy::new();
        let mut command: PolledCommand =
            serde_json::from_str(&command_json("provision", "this-hosts-command-token")).unwrap();

        // An empty body is not a provision request.
        let bad = execute(
            &command,
            "this-hosts-command-token",
            HostOs::Linux,
            &occupancy,
            &trust,
            None,
        );
        assert_eq!(bad.status, 400);
        assert!(!occupancy.is_in_session());

        // A signed image is accepted (no gate: the stub path, as in the
        // listener's own tests).
        command.body = serde_json::json!({
            "labTier": "standard",
            "sharedGpuId": null,
            "imageDigest": manifest.digest_hex(),
            "imageSignature": manifest.signature_base64(),
        });
        let good = execute(
            &command,
            "this-hosts-command-token",
            HostOs::Linux,
            &occupancy,
            &trust,
            None,
        );
        assert_eq!(good.status, 200);
        assert!(occupancy.is_in_session());

        // A tampered signature is refused before anything starts.
        command.body["imageDigest"] = serde_json::json!("0".repeat(64));
        let tampered = execute(
            &command,
            "this-hosts-command-token",
            HostOs::Linux,
            &Occupancy::new(),
            &trust,
            None,
        );
        assert_eq!(tampered.status, 400);
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let (trust, _) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let command: PolledCommand =
            serde_json::from_str(&command_json("format-disk", "t")).unwrap();

        let output = execute(
            &command,
            "t",
            HostOs::Linux,
            &Occupancy::new(),
            &trust,
            None,
        );

        assert_eq!(output.status, 400);
    }

    #[test]
    fn tick_runs_a_command_and_reports_the_outcome() {
        let dir = tempdir().unwrap();
        register(dir.path());
        let token = own_token(dir.path());
        let (trust, _) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let occupancy = Occupancy::new();
        occupancy.mark_in_session();
        let log = ProvisionLog::new();
        let transport = Scripted::new(vec![
            Ok((200, command_json("teardown", &token))),
            Ok((204, String::new())),
        ]);

        let outcome = tick(
            dir.path(),
            &lookup,
            &transport,
            &token,
            HostOs::Linux,
            &occupancy,
            &trust,
            None,
            Some(&log),
            |_| {},
        )
        .unwrap();

        assert_eq!(
            outcome,
            TickOutcome::Ran {
                kind: "teardown".to_string(),
                status: 200
            }
        );
        assert!(!occupancy.is_in_session());
        let calls = transport.calls();
        assert_eq!(
            calls[1].1,
            format!("https://mm.test.example/v1/hosts/{HOST_ID}/commands/{COMMAND_ID}/result")
        );
        assert_eq!(calls[1].3["status"], 200);
        assert_eq!(calls[1].3["body"], "{\"teardown\":true}");
        assert_eq!(log.last().unwrap().path, "/v1/teardown");
        // The command token never appears in what is sent back.
        assert!(!calls[1].3.to_string().contains(&token));
    }

    #[test]
    fn tick_is_idle_when_there_is_nothing_and_quiet_when_unregistered() {
        let dir = tempdir().unwrap();
        let (trust, _) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let occupancy = Occupancy::new();
        let none = Scripted::new(vec![]);
        let unregistered = tick(
            dir.path(),
            &lookup,
            &none,
            "t",
            HostOs::Linux,
            &occupancy,
            &trust,
            None,
            None,
            |_| {},
        )
        .unwrap();
        assert_eq!(unregistered, TickOutcome::NotRegistered);

        register(dir.path());
        let empty = Scripted::new(vec![Ok((204, String::new()))]);
        let idle = tick(
            dir.path(),
            &lookup,
            &empty,
            "t",
            HostOs::Linux,
            &occupancy,
            &trust,
            None,
            None,
            |_| {},
        )
        .unwrap();
        assert_eq!(idle, TickOutcome::Idle);
    }

    #[test]
    fn a_lost_report_is_retried() {
        let dir = tempdir().unwrap();
        register(dir.path());
        let transport = Scripted::new(vec![
            Err("connection reset".to_string()),
            Ok((502, String::new())),
            Ok((204, String::new())),
        ]);
        let output = HttpProvisionOutput {
            status: 200,
            body: "{}".to_string(),
        };
        let sleeps = Mutex::new(0u32);

        report(
            dir.path(),
            lookup,
            &transport,
            HOST_ID,
            COMMAND_ID,
            &output,
            |_| *sleeps.lock().unwrap() += 1,
        )
        .unwrap();

        assert_eq!(transport.calls().len(), 3);
        assert_eq!(*sleeps.lock().unwrap(), 2);
    }

    #[test]
    fn a_withdrawn_command_is_not_retried() {
        let dir = tempdir().unwrap();
        register(dir.path());
        let transport = Scripted::new(vec![Ok((404, String::new()))]);
        let output = HttpProvisionOutput {
            status: 200,
            body: "{}".to_string(),
        };

        let result = report(
            dir.path(),
            lookup,
            &transport,
            HOST_ID,
            COMMAND_ID,
            &output,
            |_| panic!("must not wait"),
        );

        assert!(matches!(
            result,
            Err(MatchmakerError::Rejected { status: 404, .. })
        ));
        assert_eq!(transport.calls().len(), 1);
    }
}
