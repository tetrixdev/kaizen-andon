//! The shapes Kaizen sends back, and the one rule the lamp needs locally.
//!
//! The rules themselves live server-side in `App\Support\Ledger`, so the widget
//! and the web page can never drift on what a hole is. What lives here is only
//! the reading: which state came back, and how to say a number of minutes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// The day has no start yet, and it is too early to complain about that.
    Waiting,
    /// Under the threshold. The only state that may be snoozed.
    Running,
    /// A hole has passed the threshold. No hiding from here up.
    Attention,
    /// Twice the threshold, or the day has run out of room. The lamp breathes.
    Call,
    /// Nothing owed: no target, everything accounted and referenced, or closed.
    Quiet,
}

impl State {
    /// The lamp's colour, as the CSS class the frontend switches on.
    pub fn class(&self) -> &'static str {
        match self {
            State::Waiting => "lit-wait",
            State::Running => "lit-ok",
            State::Attention => "lit-warm",
            State::Call => "lit-call",
            State::Quiet => "lit-off",
        }
    }

    pub fn is_snoozeable(&self) -> bool {
        matches!(self, State::Running)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            State::Waiting => "waiting",
            State::Running => "running",
            State::Attention => "attention",
            State::Call => "call",
            State::Quiet => "quiet",
        }
    }
}

/// Whether a snooze still holds.
///
/// It lapses three ways, and all three matter: the day rolled over, the lamp
/// found something else to say, or there was never a snooze. Hiding a card
/// that has since gone from running to call would be the one failure this
/// whole widget exists to prevent.
pub fn snooze_holds(
    snoozed_at_state: Option<&str>,
    snoozed_on: Option<&str>,
    today: &str,
    now: State,
) -> bool {
    match (snoozed_at_state, snoozed_on) {
        (Some(was), Some(day)) => day == today && was == now.as_str() && now.is_snoozeable(),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// 灯: is every minute since the start work, rest, or a hole?
    Accounting,
    /// 印: did the work reach the external system?
    Referencing,
}

impl Phase {
    pub fn glyph(&self) -> &'static str {
        match self {
            Phase::Accounting => "灯",
            Phase::Referencing => "印",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: i64,
    pub from: String,
    pub to: String,
    pub minutes: i64,
    pub kind: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub referenced: bool,
    #[serde(default)]
    pub logged: bool,
}

impl Entry {
    pub fn is_work(&self) -> bool {
        self.kind == "work"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gap {
    pub from: String,
    pub to: String,
    pub minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    pub context: String,
    pub date: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub target_minutes: Option<i64>,
    #[serde(default)]
    pub threshold_minutes: i64,
    #[serde(default)]
    pub work_minutes: i64,
    #[serde(default)]
    pub referenced_minutes: i64,
    #[serde(default)]
    pub rest_minutes: i64,
    #[serde(default)]
    pub gap_minutes: i64,
    #[serde(default)]
    pub unreferenced_minutes: i64,
    #[serde(default)]
    pub logs_externally: bool,
    pub phase: Phase,
    pub state: State,
    #[serde(default)]
    pub gaps: Vec<Gap>,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Ledger {
    /// The number the lamp shows, and what it is a number OF. In the second
    /// phase the same position on the card means something different from what
    /// it meant an hour earlier, which is exactly why the glyph changes too.
    pub fn headline(&self) -> (String, &'static str) {
        match self.phase {
            Phase::Referencing if self.unreferenced_minutes > 0 => {
                (hhmm(self.unreferenced_minutes), "not in the other system")
            }
            _ if self.gap_minutes > 0 => (hhmm(self.gap_minutes), "unaccounted"),
            _ if self.started_at.is_none() => ("—:—".into(), "day not started"),
            _ => (hhmm(self.work_minutes), "accounted for"),
        }
    }
}

/// Minutes as a clock reading. 165 is 2:45, never 2.75 and never 165m.
pub fn hhmm(minutes: i64) -> String {
    let m = minutes.max(0);
    format!("{}:{:02}", m / 60, m % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Ledger {
        Ledger {
            context: "Work".into(),
            date: "2026-08-19".into(),
            started_at: Some("08:30".into()),
            ended_at: None,
            window: Some("Mon–Fri 08:00–18:00".into()),
            target_minutes: Some(450),
            threshold_minutes: 30,
            work_minutes: 165,
            referenced_minutes: 0,
            rest_minutes: 30,
            gap_minutes: 0,
            unreferenced_minutes: 0,
            logs_externally: false,
            phase: Phase::Accounting,
            state: State::Running,
            gaps: vec![],
            entries: vec![],
        }
    }

    #[test]
    fn minutes_read_as_a_clock() {
        assert_eq!(hhmm(0), "0:00");
        assert_eq!(hhmm(45), "0:45");
        assert_eq!(hhmm(165), "2:45");
        assert_eq!(hhmm(450), "7:30");
        assert_eq!(hhmm(-5), "0:00", "a negative hole is not a thing");
    }

    #[test]
    fn only_running_may_be_snoozed() {
        assert!(State::Running.is_snoozeable());
        for state in [State::Waiting, State::Attention, State::Call, State::Quiet] {
            assert!(!state.is_snoozeable(), "{state:?} must not be hideable");
        }
    }

    #[test]
    fn a_snooze_lapses_the_moment_the_lamp_has_something_else_to_say() {
        let held = |state| snooze_holds(Some("running"), Some("2026-08-19"), "2026-08-19", state);

        assert!(held(State::Running), "still quiet, still hidden");
        assert!(!held(State::Attention), "a hole crossed the threshold");
        assert!(!held(State::Call), "and this one especially");
    }

    #[test]
    fn a_snooze_does_not_survive_the_day_boundary() {
        assert!(!snooze_holds(
            Some("running"),
            Some("2026-08-18"),
            "2026-08-19",
            State::Running
        ));
    }

    #[test]
    fn no_snooze_means_no_snooze() {
        assert!(!snooze_holds(None, None, "2026-08-19", State::Running));
        assert!(!snooze_holds(
            Some("running"),
            None,
            "2026-08-19",
            State::Running
        ));
    }

    #[test]
    fn the_glyph_carries_the_question() {
        assert_eq!(Phase::Accounting.glyph(), "灯");
        assert_eq!(Phase::Referencing.glyph(), "印");
    }

    #[test]
    fn the_headline_follows_the_hole_then_the_reference() {
        let mut l = base();
        l.gap_minutes = 155;
        assert_eq!(l.headline(), ("2:35".into(), "unaccounted"));

        l.gap_minutes = 0;
        l.phase = Phase::Referencing;
        l.unreferenced_minutes = 240;
        assert_eq!(l.headline(), ("4:00".into(), "not in the other system"));

        l.unreferenced_minutes = 0;
        assert_eq!(l.headline(), ("2:45".into(), "accounted for"));
    }

    #[test]
    fn a_day_with_no_start_says_so_rather_than_showing_a_zero() {
        let mut l = base();
        l.started_at = None;
        l.work_minutes = 0;
        assert_eq!(l.headline().1, "day not started");
    }

    #[test]
    fn a_ledger_parses_from_what_kaizen_sends() {
        let json = r#"{
            "context": "Work", "date": "2026-08-19", "started_at": "08:30",
            "ended_at": null, "window": "Mon–Fri 08:00–18:00",
            "target_minutes": 450, "threshold_minutes": 30,
            "work_minutes": 165, "referenced_minutes": 45, "rest_minutes": 30,
            "gap_minutes": 155, "unreferenced_minutes": 120,
            "logs_externally": true, "phase": "accounting", "state": "call",
            "gaps": [{"from": "11:15", "to": "12:15", "minutes": 60}],
            "entries": [{
                "id": 1, "from": "08:30", "to": "09:15", "minutes": 45,
                "kind": "work", "description": "Standup", "reference": "TE-1",
                "link": "https://x/1", "referenced": true, "logged": true,
                "version": "2026-08-19T09:00:00.000000Z"
            }]
        }"#;

        let ledger: Ledger = serde_json::from_str(json).expect("parses");

        assert_eq!(ledger.state, State::Call);
        assert_eq!(ledger.phase, Phase::Accounting);
        assert_eq!(ledger.gaps[0].minutes, 60);
        assert!(ledger.entries[0].is_work());
        assert_eq!(ledger.headline(), ("2:35".into(), "unaccounted"));
    }
}
