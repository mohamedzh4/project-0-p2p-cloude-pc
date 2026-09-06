//! Local host presence for FR10. Earnings are credits accrued, not paid out.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

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

pub fn earnings_note(credits_accrued: Option<i64>) -> String {
    match credits_accrued {
        Some(credits) => format!("{credits} credits accrued (not paid out)"),
        None => "earnings unavailable".to_string(),
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
    pub fn from_flags(
        sharing_enabled: bool,
        in_session: bool,
        credits_accrued: Option<i64>,
    ) -> Self {
        Self {
            presence: presence(sharing_enabled, in_session),
            sharing_enabled,
            in_session,
            earnings_note: earnings_note(credits_accrued),
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
    fn status_json_is_camel_case_with_accrued_earnings() {
        let status = AgentStatus::from_flags(true, true, Some(12));
        let json = serde_json::to_value(&status).unwrap();

        assert_eq!(json["presence"], "inSession");
        assert_eq!(json["sharingEnabled"], true);
        assert_eq!(json["inSession"], true);
        assert_eq!(json["earningsNote"], "12 credits accrued (not paid out)");
        assert_eq!(status.tooltip(), "Host Agent — In session");
        assert!(json.get("paid").is_none());
    }

    #[test]
    fn earnings_note_does_not_invent_a_payout() {
        assert_eq!(earnings_note(Some(0)), "0 credits accrued (not paid out)");
        assert_eq!(earnings_note(None), "earnings unavailable");
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
