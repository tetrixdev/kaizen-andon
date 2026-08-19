//! What survives a restart.
//!
//! One small JSON file, written atomically. No local database: single writer,
//! single reader, nothing to query. Tokens are deliberately NOT here; those go
//! to the OS credential store once the OAuth flow lands.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Which Kaizen this is pointed at. Never baked into the build: a generic
    /// binary is what lets one installer serve a dev instance, a production
    /// one, or somebody else's entirely.
    #[serde(default)]
    pub server_url: Option<String>,

    /// The kaizen day the rest of this file describes, computed from the
    /// user's own `day_start_hour` rather than calendar midnight.
    #[serde(default)]
    pub kaizen_day: Option<String>,

    /// Snoozing is a Running-only privilege, and it lapses at the next
    /// threshold crossing rather than lasting the day. Storing the state it
    /// was snoozed at is what enforces that: the moment the lamp has something
    /// else to say, the snooze is spent.
    #[serde(default)]
    pub snoozed_at_state: Option<String>,

    /// The kaizen day the snooze belongs to, so it cannot survive into
    /// tomorrow and hide a fresh morning.
    #[serde(default)]
    pub snoozed_on: Option<String>,

    /// Which display it was on, so it comes back where it was left.
    #[serde(default)]
    pub display: Option<String>,

    #[serde(default)]
    pub expanded: bool,

    /// The client this install registered as. Not a secret: Kaizen issues
    /// public clients, because a binary on the user's machine can always be
    /// read and a secret in it would be a secret in name only.
    #[serde(default)]
    pub client_id: Option<String>,
}

impl Config {
    fn path(app: &AppHandle) -> io::Result<PathBuf> {
        let dir = app
            .path()
            .app_config_dir()
            .map_err(|e| io::Error::other(e.to_string()))?;

        fs::create_dir_all(&dir)?;

        Ok(dir.join("config.json"))
    }

    /// A missing or unreadable file is a first run, not a failure. The widget
    /// must never refuse to start because its own config got mangled.
    pub fn load(app: &AppHandle) -> io::Result<Self> {
        let path = Self::path(app)?;

        match fs::read_to_string(&path) {
            Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Write to a temporary file and rename over the target, so a crash
    /// mid-write leaves the previous config intact rather than a half one.
    pub fn save(&self, app: &AppHandle) -> io::Result<()> {
        let path = Self::path(app)?;
        let tmp = path.with_extension("json.tmp");

        let body = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        fs::write(&tmp, body)?;
        fs::rename(&tmp, &path)?;

        Ok(())
    }

    /// The client already registered for this server, if it is the same one.
    ///
    /// Pointing the app at a different Kaizen has to register afresh: a client
    /// belongs to the server that issued it, and reusing an id across servers
    /// would simply be rejected.
    pub fn client_for(&self, server: &str) -> Option<String> {
        match (self.server_url.as_deref(), self.client_id.as_deref()) {
            (Some(known), Some(id)) if known == server => Some(id.to_string()),
            _ => None,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.server_url
            .as_deref()
            .is_some_and(|u| !u.trim().is_empty())
            && self.client_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_config_is_not_connected() {
        assert!(!Config::default().is_connected());
    }

    #[test]
    fn a_blank_url_does_not_count_as_connected() {
        let config = Config {
            server_url: Some("   ".into()),
            ..Default::default()
        };
        assert!(!config.is_connected());
    }

    #[test]
    fn a_url_without_a_client_is_not_yet_connected() {
        let config = Config {
            server_url: Some("https://kaizen.tetrix.dev".into()),
            ..Default::default()
        };
        assert!(
            !config.is_connected(),
            "an address alone is an intention, not a connection"
        );
    }

    #[test]
    fn a_url_and_a_client_is_connected() {
        let config = Config {
            server_url: Some("https://kaizen.tetrix.dev".into()),
            client_id: Some("01a01b20-c1bb-72bd-8484-711e07c45fe5".into()),
            ..Default::default()
        };
        assert!(config.is_connected());
    }

    #[test]
    fn a_client_is_reused_only_for_the_server_that_issued_it() {
        let config = Config {
            server_url: Some("https://kaizen.tetrix.dev".into()),
            client_id: Some("c1".into()),
            ..Default::default()
        };

        assert_eq!(
            config.client_for("https://kaizen.tetrix.dev").as_deref(),
            Some("c1")
        );
        assert_eq!(
            config.client_for("https://other.example.com"),
            None,
            "a client belongs to the server that issued it"
        );
        assert_eq!(Config::default().client_for("https://x"), None);
    }

    #[test]
    fn unknown_fields_do_not_break_an_older_build() {
        // A newer build may write more than this one knows about, and the
        // older one still has to start. What matters is that the fields it
        // does understand survive, not that the file describes a connection.
        let raw = r#"{"server_url": "https://x", "client_id": "c1", "something_new": 42}"#;
        let config: Config = serde_json::from_str(raw).unwrap();

        assert_eq!(config.server_url.as_deref(), Some("https://x"));
        assert_eq!(config.client_id.as_deref(), Some("c1"));
    }

    #[test]
    fn a_mangled_file_reads_as_a_first_run() {
        let config: Config = serde_json::from_str("not json at all").unwrap_or_default();

        assert!(
            !config.is_connected(),
            "never refuse to start over a bad config"
        );
    }
}
