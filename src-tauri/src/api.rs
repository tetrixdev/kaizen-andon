//! Talking to Kaizen.
//!
//! Deliberately thin. Kaizen's `App\Support\Ledger` decides what a hole is,
//! what state the lamp is in and when the question changes from 灯 to 印; this
//! module only carries the answer across. Two implementations of those rules
//! would drift within a week.

use crate::ledger::Ledger;
use serde::{Deserialize, Serialize};

/// What `POST /api/desktop/hello` answers: enough to place ourselves, plus
/// whether a newer build exists. That is the whole update check, and it costs
/// no polling on either side because the app was calling anyway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub user: HelloUser,
    #[serde(default)]
    pub day_start_hour: i64,
    #[serde(default)]
    pub contexts: Vec<HelloContext>,
    #[serde(default)]
    pub release: Option<Release>,
    #[serde(default)]
    pub update_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloUser {
    pub name: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloContext {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub logs_externally: bool,
    #[serde(default)]
    pub has_integration_prompt: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Day {
    pub date: String,
    #[serde(default)]
    pub local_time: Option<String>,
    #[serde(default)]
    pub ledgers: Vec<Ledger>,
}

impl Day {
    /// The one the lamp shows. Where several contexts carry a target, the one
    /// with something to say wins, because a widget this small can only ask
    /// one question at a time.
    pub fn leading(&self) -> Option<&Ledger> {
        use crate::ledger::State;

        let rank = |l: &&Ledger| match l.state {
            State::Call => 0,
            State::Attention => 1,
            State::Waiting => 2,
            State::Running => 3,
            State::Quiet => 4,
        };

        self.ledgers.iter().min_by_key(rank)
    }
}

/// The clipboard text from `GET /api/desktop/prompt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub context: String,
    pub date: String,
    /// True when the context has no stored instructions yet, so the prompt
    /// asks to be taught rather than assuming it knows where time lives.
    #[serde(default)]
    pub bootstrap: bool,
    pub prompt: String,
}

/// A month of day tiles, for the history grid.
///
/// Deliberately not thirty ledgers: a tile carries four facts and a ledger
/// carries fifty, and the grid exists to glance at a month and pick one day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Month {
    pub month: String,
    pub label: String,
    #[serde(default)]
    pub context: String,
    /// ISO weekday of the 1st, 1 = Monday, so the grid knows where to start.
    #[serde(default = "one")]
    pub first_weekday: u32,
    #[serde(default)]
    pub days: Vec<MonthDay>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonthDay {
    pub date: String,
    #[serde(default)]
    pub has_target: bool,
    #[serde(default)]
    pub work_minutes: i64,
    #[serde(default)]
    pub entries: i64,
    #[serde(default)]
    pub accounted: bool,
    #[serde(default)]
    pub referenced: bool,
    #[serde(default)]
    pub is_today: bool,
    #[serde(default)]
    pub is_future: bool,
}

/// One entry on its way to Kaizen. `id` set is an edit, absent is a new row.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntryDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub from: String,
    pub to: String,
    pub kind: String,
    // A plain String, not Option, matching from/to/kind above: the widget
    // refuses to build this payload at all until a title is typed, so by the
    // time one of these is serialized it is never actually absent.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// Where each call goes. Kept in one place so a typo is a compile error rather
/// than a 404 at runtime.
pub fn url(server: &str, path: &str) -> String {
    format!("{}/api/desktop{}", server.trim_end_matches('/'), path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{CaptureKinds, Phase, State};

    #[test]
    fn paths_survive_a_trailing_slash() {
        assert_eq!(
            url("https://kaizen.tetrix.dev", "/day"),
            "https://kaizen.tetrix.dev/api/desktop/day"
        );
        assert_eq!(
            url("https://kaizen.tetrix.dev/", "/day"),
            "https://kaizen.tetrix.dev/api/desktop/day"
        );
    }

    #[test]
    fn hello_parses_what_kaizen_answers() {
        let json = r#"{
            "user": {"name": "Jasper", "timezone": "Europe/Amsterdam"},
            "day_start_hour": 4,
            "contexts": [
                {"id": 2, "name": "Work", "schedule": [], "logs_externally": true,
                 "has_integration_prompt": false}
            ],
            "release": {"version": "1.2.0", "url": "https://example.invalid/s.exe"},
            "update_available": true
        }"#;

        let hello: Hello = serde_json::from_str(json).expect("parses");

        assert_eq!(hello.user.timezone, "Europe/Amsterdam");
        assert_eq!(hello.day_start_hour, 4);
        assert!(hello.contexts[0].logs_externally);
        assert!(!hello.contexts[0].has_integration_prompt);
        assert!(hello.update_available);
        assert_eq!(hello.release.unwrap().version.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn hello_survives_a_server_with_no_release_published() {
        let json = r#"{
            "user": {"name": "Jasper", "timezone": "Europe/Amsterdam"},
            "day_start_hour": 4, "contexts": [], "release": null,
            "update_available": false
        }"#;

        let hello: Hello = serde_json::from_str(json).expect("parses");

        assert!(
            hello.release.is_none(),
            "no build published is a normal state"
        );
        assert!(!hello.update_available);
    }

    fn ledger(context: &str, state: State) -> Ledger {
        Ledger {
            context: context.into(),
            date: "2026-08-19".into(),
            started_at: None,
            ended_at: None,
            window: None,
            target_minutes: Some(480),
            threshold_minutes: 30,
            work_minutes: 0,
            referenced_minutes: 0,
            rest_minutes: 0,
            gap_minutes: 0,
            unreferenced_minutes: 0,
            logs_externally: false,
            capture: CaptureKinds::default(),
            phase: Phase::Accounting,
            state,
            gaps: vec![],
            entries: vec![],
        }
    }

    #[test]
    fn the_loudest_context_is_the_one_the_lamp_shows() {
        let day = Day {
            date: "2026-08-19".into(),
            local_time: Some("14:20".into()),
            ledgers: vec![
                ledger("Personal", State::Quiet),
                ledger("Work", State::Call),
                ledger("Side", State::Running),
            ],
        };

        assert_eq!(day.leading().unwrap().context, "Work");
    }

    #[test]
    fn an_unstarted_day_outranks_one_that_is_merely_running() {
        // Waiting means nothing can be filed yet, which is more urgent than a
        // day already ticking along under its threshold.
        let day = Day {
            date: "2026-08-19".into(),
            local_time: None,
            ledgers: vec![
                ledger("Running", State::Running),
                ledger("Waiting", State::Waiting),
            ],
        };

        assert_eq!(day.leading().unwrap().context, "Waiting");
    }

    #[test]
    fn a_day_with_nothing_on_it_leads_with_nothing() {
        let day = Day {
            date: "2026-08-22".into(),
            local_time: None,
            ledgers: vec![],
        };

        assert!(day.leading().is_none(), "a weekend has no lamp");
    }

    #[test]
    fn a_day_parses_whole() {
        let json = r#"{
            "date": "2026-08-19", "local_time": "14:20",
            "ledgers": [{
                "context": "Work", "date": "2026-08-19", "started_at": "08:30",
                "window": "Mon–Fri 08:00–18:00", "target_minutes": 480,
                "threshold_minutes": 30, "work_minutes": 165,
                "referenced_minutes": 45, "rest_minutes": 30, "gap_minutes": 155,
                "unreferenced_minutes": 120, "logs_externally": true,
                "phase": "accounting", "state": "call", "snoozeable": false,
                "gaps": [{"from": "11:15", "to": "12:15", "minutes": 60}],
                "entries": []
            }]
        }"#;

        let day: Day = serde_json::from_str(json).expect("parses");
        let leading = day.leading().expect("has one");

        assert_eq!(leading.context, "Work");
        assert_eq!(leading.headline(), ("2:35".into(), "unaccounted"));
        assert_eq!(leading.gaps[0].minutes, 60);
    }

    #[test]
    fn the_prompt_says_when_it_is_asking_to_be_taught() {
        let json = r##"{
            "context": "Work", "date": "2026-08-19", "bootstrap": true,
            "prompt": "# Account for Work..."
        }"##;

        let prompt: Prompt = serde_json::from_str(json).expect("parses");

        assert!(prompt.bootstrap);
        assert!(prompt.prompt.starts_with("# Account for"));
    }
}
