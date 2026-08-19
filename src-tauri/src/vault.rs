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

/// How much of a value one credential entry may hold, in characters.
///
/// Windows caps a credential blob at 2560 BYTES and the store encodes as
/// UTF-16, so the real ceiling is 1280 characters. A Passport access token is
/// a JWT of around nine hundred and the refresh token beside it adds several
/// hundred more, so the pair does not fit and the write fails outright with
/// "longer than platform limit of 2560 chars". Neither length is ours to
/// choose, so the value is split instead. A thousand leaves clear room under
/// the limit without making the number of entries interesting.
#[cfg_attr(not(windows), allow(dead_code))]
const CHUNK: usize = 1000;

/// Split a value into pieces the credential store will accept.
///
/// Split by characters rather than bytes: tokens are ASCII today, but slicing
/// a string by byte offset panics the moment one is not, and that is not a
/// failure worth risking to save a conversion.
#[cfg_attr(not(windows), allow(dead_code))]
fn split(raw: &str, size: usize) -> Vec<String> {
    raw.chars()
        .collect::<Vec<_>>()
        .chunks(size.max(1))
        .map(|piece| piece.iter().collect())
        .collect()
}

#[cfg(windows)]
mod platform {
    use super::{split, Tokens, ACCOUNT, CHUNK, SERVICE};

    fn entry(account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, account).map_err(|e| format!("credential store: {e}"))
    }

    /// The account holding piece `index`. The bare ACCOUNT holds the count.
    fn part(index: usize) -> String {
        format!("{ACCOUNT}.{index}")
    }

    fn read(account: &str) -> Result<Option<String>, String> {
        match entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            // Nothing stored yet is a first run, not a failure.
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("could not read the credential store: {e}")),
        }
    }

    fn remove(account: &str) -> Result<(), String> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("could not clear the credential store: {e}")),
        }
    }

    pub fn load() -> Result<Tokens, String> {
        let Some(count) = read(ACCOUNT)? else {
            return Ok(Tokens::default());
        };

        // Anything that is not a count was written by a build that did not
        // split, or is a write that did not finish. Neither is worth failing
        // over and neither is worth reporting: a first run is the honest
        // reading, and connecting again fixes it.
        let Ok(count) = count.parse::<usize>() else {
            return Ok(Tokens::default());
        };

        let mut raw = String::new();

        for index in 0..count {
            match read(&part(index))? {
                Some(piece) => raw.push_str(&piece),
                None => return Ok(Tokens::default()),
            }
        }

        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    pub fn save(tokens: &Tokens) -> Result<(), String> {
        let raw = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
        let pieces = split(&raw, CHUNK);

        for (index, piece) in pieces.iter().enumerate() {
            entry(&part(index))?
                .set_password(piece)
                .map_err(|e| format!("could not write to the credential store: {e}"))?;
        }

        // The count goes LAST. Until it lands, load() still sees the previous
        // consistent set, so a write that dies halfway leaves the old session
        // working rather than a half-written one that parses as nonsense.
        entry(ACCOUNT)?
            .set_password(&pieces.len().to_string())
            .map_err(|e| format!("could not write to the credential store: {e}"))?;

        // A previous, longer value leaves pieces past the new end. They are
        // unreachable now, but they are still tokens sitting in the store.
        let mut index = pieces.len();

        while read(&part(index))?.is_some() {
            remove(&part(index))?;
            index += 1;
        }

        Ok(())
    }

    pub fn clear() -> Result<(), String> {
        let mut index = 0;

        while read(&part(index))?.is_some() {
            remove(&part(index))?;
            index += 1;
        }

        remove(ACCOUNT)
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
    use std::sync::Mutex;

    /// The store is one place, not one per test: on Windows it is a single
    /// Credential Manager entry for the whole machine, and here it is a single
    /// process-wide cell. Cargo runs tests on parallel threads, so without
    /// this the round-trip test's `save` lands between the first-run test's
    /// `clear` and its `load` and the first-run test fails on a token it never
    /// wrote. That is a race, so it fails on some runs and not others.
    static STORE: Mutex<()> = Mutex::new(());

    /// A failing test leaves the lock poisoned, which would turn one real
    /// failure into every other test failing too and bury the cause.
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        STORE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn nothing_stored_reads_as_a_first_run() {
        let _guard = exclusive();

        clear().expect("clears");
        let tokens = load().expect("loads");

        assert!(
            !tokens.has_token(),
            "an empty store is a first run, not an error"
        );
    }

    #[test]
    fn a_token_round_trips_and_can_be_cleared() {
        let _guard = exclusive();

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
    fn splitting_loses_nothing_and_respects_the_limit() {
        // A realistic pair: a JWT access token and a refresh token beside it,
        // which together are what overran the 2560-byte blob.
        let raw = format!(
            r#"{{"access_token":"{}","refresh_token":"{}"}}"#,
            "e".repeat(900),
            "d".repeat(700)
        );

        let pieces = split(&raw, CHUNK);

        assert!(
            pieces.len() > 1,
            "this is exactly the value that did not fit"
        );
        assert!(
            pieces.iter().all(|p| p.chars().count() <= CHUNK),
            "no piece may exceed what one entry holds"
        );
        assert_eq!(pieces.concat(), raw, "rejoining must return the original");
    }

    #[test]
    fn splitting_a_short_value_leaves_it_whole() {
        assert_eq!(split(r#"{"access_token":null}"#, CHUNK).len(), 1);
    }

    #[test]
    fn splitting_never_cuts_a_character_in_half() {
        // Tokens are ASCII today, but a byte-sliced implementation panics the
        // first time one is not, and that would be a crash on save.
        let raw = "灯".repeat(10);
        let pieces = split(&raw, 3);

        assert_eq!(pieces.len(), 4);
        assert_eq!(pieces.concat(), raw);
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
