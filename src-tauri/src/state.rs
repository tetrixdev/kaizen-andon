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
    /// threshold crossing rather than lasting the day.
    #[serde(default)]
    pub snoozed_until: Option<String>,

    /// Which display it was on, so it comes back where it was left.
    #[serde(default)]
    pub display: Option<String>,

    #[serde(default)]
    pub expanded: bool,
}

impl Config {
    fn path(app: &AppHandle) -> io::Result<PathBuf> {
        let dir = app
            .path()
            .app_config_dir()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

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
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        fs::write(&tmp, body)?;
        fs::rename(&tmp, &path)?;

        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.server_url
            .as_deref()
            .is_some_and(|u| !u.trim().is_empty())
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
    fn a_real_url_does() {
        let config = Config {
            server_url: Some("https://kaizen.tetrix.dev".into()),
            ..Default::default()
        };
        assert!(config.is_connected());
    }

    #[test]
    fn unknown_fields_do_not_break_an_older_build() {
        // A newer Kaizen may hand back more than this build knows about.
        let raw = r#"{"server_url": "https://x", "something_new": 42}"#;
        let config: Config = serde_json::from_str(raw).unwrap();

        assert!(config.is_connected());
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
