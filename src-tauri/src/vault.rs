//! Where the tokens live.
//!
//! On Windows: the Credential Manager, which is DPAPI underneath. That matters
//! less for the threat people imagine than for the one they do not: `%APPDATA%`
//! is backed up, synced and folder-redirected, and a token here is valid for a
//! YEAR. A plaintext copy therefore travels off the machine in places nobody
//! is thinking about, while a DPAPI blob is bound to this machine and user and
//! is inert anywhere else.
//!
//! It is worth being plain about what this does NOT buy: DPAPI unlocks
//! automatically for the same user, so anything already running as you can
//! read either form. This is about copies at rest, not about malware.
//!
//! Everywhere else (and in CI, which is Linux) there is no Credential Manager,
//! so the file store stands in. Only Windows ships.

use serde::{Deserialize, Serialize};

#[cfg(windows)]
const SERVICE: &str = "dev.tetrix.kaizen";
#[cfg(windows)]
const ACCOUNT: &str = "oauth";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl Tokens {
    pub fn has_token(&self) -> bool {
        self.access_token.as_deref().is_some_and(|t| !t.is_empty())
    }
}

/// Whether this build keeps tokens in the OS credential store or a file, so
/// the app can say which rather than implying the stronger one.
pub fn backend() -> &'static str {
    if cfg!(windows) {
        "Windows Credential Manager"
    } else {
        "a file in the config directory"
    }
}

#[cfg(windows)]
mod platform {
    use super::{Tokens, ACCOUNT, SERVICE};

    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| format!("credential store: {e}"))
    }

    pub fn load() -> Result<Tokens, String> {
        match entry()?.get_password() {
            Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
            // Nothing stored yet is a first run, not a failure.
            Err(keyring::Error::NoEntry) => Ok(Tokens::default()),
            Err(e) => Err(format!("could not read the credential store: {e}")),
        }
    }

    pub fn save(tokens: &Tokens) -> Result<(), String> {
        let raw = serde_json::to_string(tokens).map_err(|e| e.to_string())?;

        entry()?
            .set_password(&raw)
            .map_err(|e| format!("could not write to the credential store: {e}"))
    }

    pub fn clear() -> Result<(), String> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("could not clear the credential store: {e}")),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Tokens;
    use std::sync::{Mutex, OnceLock};

    /// Only CI and local checks reach this. Keeping it in memory means no test
    /// ever writes a token to a developer's disk.
    fn cell() -> &'static Mutex<Tokens> {
        static CELL: OnceLock<Mutex<Tokens>> = OnceLock::new();
        CELL.get_or_init(|| Mutex::new(Tokens::default()))
    }

    pub fn load() -> Result<Tokens, String> {
        Ok(cell().lock().expect("not poisoned").clone())
    }

    pub fn save(tokens: &Tokens) -> Result<(), String> {
        *cell().lock().expect("not poisoned") = tokens.clone();
        Ok(())
    }

    pub fn clear() -> Result<(), String> {
        *cell().lock().expect("not poisoned") = Tokens::default();
        Ok(())
    }
}

pub use platform::{clear, load, save};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_reads_as_a_first_run() {
        clear().expect("clears");
        let tokens = load().expect("loads");

        assert!(
            !tokens.has_token(),
            "an empty store is a first run, not an error"
        );
    }

    #[test]
    fn a_token_round_trips_and_can_be_cleared() {
        let stored = Tokens {
            access_token: Some("eyJ0eXAi".into()),
            refresh_token: Some("def502".into()),
        };

        save(&stored).expect("saves");
        assert_eq!(load().expect("loads"), stored);

        clear().expect("clears");
        assert!(!load().expect("loads").has_token());
    }

    #[test]
    fn an_empty_string_is_not_a_token() {
        let tokens = Tokens {
            access_token: Some(String::new()),
            refresh_token: None,
        };

        assert!(!tokens.has_token());
    }

    #[test]
    fn the_backend_is_named_so_the_app_can_be_honest_about_it() {
        let named = backend();

        if cfg!(windows) {
            assert_eq!(named, "Windows Credential Manager");
        } else {
            assert!(named.contains("file"), "never claim more than it does");
        }
    }
}
