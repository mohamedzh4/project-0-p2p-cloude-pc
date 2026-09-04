//! Local host presence for FR10. Earnings stay a placeholder until phase 11.
//!
//! Pause (`sharingEnabled=false`) always wins: occupancy indexes can still hold
//! an active session, but this machine is not advertising itself.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

/// Copy only — phase 11 meters usage. Do not invent a number here.
pub const EARNINGS_PLACEHOLDER: &str = "not metered yet";

/// Process-wide occupancy for the provision listener thread (no Tauri types).
/// Cleared only when a later teardown path exists (phase 12).
#[derive(Debug)]
pub struct Occupancy {
    in_session: AtomicBool,
}

impl Occupancy {
    pub fn new() -> Self {
        Self {
            in_session: AtomicBool::new(false),
        }
    }

    pub fn mark_in_session(&self) {
        self.in_session.store(true, Ordering::SeqCst);
    }

    pub fn is_in_session(&self) -> bool {
        self.in_session.load(Ordering::SeqCst)
    }
}

impl Default for Occupancy {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SharingPresence {
    Paused,
    Idle,
    InSession,
}

impl SharingPresence {
    pub fn label(self) -> &'static str {
        match self {
            Self::Paused => "Paused",
            Self::Idle => "Idle",
            Self::InSession => "In session",
        }
    }
}

/// Paused beats occupancy: FR11 can stop sharing without ending the guest.
pub fn presence(sharing_enabled: bool, in_session: bool) -> SharingPresence {
    if !sharing_enabled {
        SharingPresence::Paused
    } else if in_session {
        SharingPresence::InSession
    } else {
        SharingPresence::Idle
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub presence: SharingPresence,
    pub sharing_enabled: bool,
    pub in_session: bool,
    pub earnings_note: String,
}

impl AgentStatus {
    pub fn from_flags(sharing_enabled: bool, in_session: bool) -> Self {
        Self {
            presence: presence(sharing_enabled, in_session),
            sharing_enabled,
            in_session,
            earnings_note: EARNINGS_PLACEHOLDER.to_string(),
        }
    }

    pub fn tooltip(&self) -> String {
        format!("Host Agent — {}", self.presence.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_when_sharing_is_off_even_if_occupied() {
        assert_eq!(presence(false, false), SharingPresence::Paused);
        assert_eq!(presence(false, true), SharingPresence::Paused);
    }

    #[test]
    fn idle_when_sharing_is_on_and_no_local_occupancy() {
        assert_eq!(presence(true, false), SharingPresence::Idle);
    }

    #[test]
    fn in_session_when_sharing_is_on_and_occupied() {
        assert_eq!(presence(true, true), SharingPresence::InSession);
    }

    #[test]
    fn status_json_is_camel_case_with_placeholder_earnings() {
        let status = AgentStatus::from_flags(true, true);
        let json = serde_json::to_value(&status).unwrap();

        assert_eq!(json["presence"], "inSession");
        assert_eq!(json["sharingEnabled"], true);
        assert_eq!(json["inSession"], true);
        assert_eq!(json["earningsNote"], EARNINGS_PLACEHOLDER);
        assert_eq!(status.tooltip(), "Host Agent — In session");
    }

    #[test]
    fn occupancy_starts_clear_and_marks_in_session() {
        let occupancy = Occupancy::new();
        assert!(!occupancy.is_in_session());
        occupancy.mark_in_session();
        assert!(occupancy.is_in_session());
        assert_eq!(
            presence(true, occupancy.is_in_session()),
            SharingPresence::InSession
        );
    }
}
