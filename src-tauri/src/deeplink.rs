//! `kaizen-andon://connect?server=https://kaizen.example.com`
//!
//! The build is generic on purpose, so the address has to come from somewhere.
//! Typing it works, but the smooth path is the Connect button in Kaizen's own
//! settings: the URL arrives from the link you clicked rather than from the
//! binary you downloaded, which is what lets one installer serve a dev
//! instance, a production one, or somebody else's.
//!
//! A deep link is untrusted input. It arrives from a browser, and any page can
//! try to open one, so the address is normalised and checked here before
//! anything is done with it. Opening a connect flow is the whole extent of
//! what it can cause: the user still has to sign in and approve.

/// What a `kaizen-andon://` URL asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Point the app at a Kaizen and start connecting.
    Connect { server: String },
    /// Bring the window forward. No argument, nothing to validate.
    Show,
}

/// Read an intent out of a deep link, or nothing if it does not make sense.
///
/// Returning `None` rather than an error is deliberate: a malformed link is
/// something a stray web page did, not something the user needs told about.
pub fn parse(url: &str) -> Option<Intent> {
    let rest = url.strip_prefix("kaizen-andon://")?;
    let (action, query) = match rest.split_once('?') {
        Some((action, query)) => (action, Some(query)),
        None => (rest, None),
    };

    // Windows hands these over as kaizen-andon://connect/?server=... about as
    // often as without the slash.
    let action = action.trim_end_matches('/');

    match action {
        "show" | "" => Some(Intent::Show),
        "connect" => {
            let server = query?
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(key, _)| *key == "server")
                .map(|(_, value)| decode(value))?;

            // Anything that is not an address is not worth acting on, and
            // normalising here means the rest of the app never sees a path or
            // a stray trailing slash.
            crate::auth::normalise_server(&server).map(|server| Intent::Connect { server })
        }
        _ => None,
    }
}

fn decode(value: &str) -> String {
    let bytes = value.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

/// The first deep link in a set of launch arguments, if there is one. Windows
/// passes the URL as an argument when it starts the exe to handle a scheme.
pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Option<Intent> {
    args.into_iter().find_map(|arg| parse(&arg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connect_link_carries_a_normalised_address() {
        assert_eq!(
            parse("kaizen-andon://connect?server=https%3A%2F%2Fkaizen.tetrix.dev"),
            Some(Intent::Connect {
                server: "https://kaizen.tetrix.dev".into()
            })
        );
    }

    #[test]
    fn windows_adds_a_slash_about_half_the_time() {
        assert_eq!(
            parse("kaizen-andon://connect/?server=https%3A%2F%2Fkaizen.tetrix.dev"),
            Some(Intent::Connect {
                server: "https://kaizen.tetrix.dev".into()
            })
        );
    }

    #[test]
    fn a_bare_host_is_accepted_and_a_path_is_dropped() {
        assert_eq!(
            parse("kaizen-andon://connect?server=kaizen.tetrix.dev"),
            Some(Intent::Connect {
                server: "https://kaizen.tetrix.dev".into()
            })
        );
        assert_eq!(
            parse("kaizen-andon://connect?server=https%3A%2F%2Fkaizen.tetrix.dev%2Fmore%2Ftime"),
            Some(Intent::Connect {
                server: "https://kaizen.tetrix.dev".into()
            })
        );
    }

    #[test]
    fn show_needs_no_argument() {
        assert_eq!(parse("kaizen-andon://show"), Some(Intent::Show));
        assert_eq!(parse("kaizen-andon://"), Some(Intent::Show));
    }

    #[test]
    fn nonsense_is_ignored_rather_than_reported() {
        // A stray page can try to open any of these. None of them is something
        // the user did, so none of them is worth interrupting them about.
        assert_eq!(parse("https://kaizen.tetrix.dev"), None, "wrong scheme");
        assert_eq!(parse("kaizen-andon://connect"), None, "no server");
        assert_eq!(
            parse("kaizen-andon://connect?server="),
            None,
            "empty server"
        );
        assert_eq!(
            parse("kaizen-andon://connect?other=x"),
            None,
            "wrong parameter"
        );
        assert_eq!(
            parse("kaizen-andon://wipe?everything=1"),
            None,
            "unknown action"
        );
        assert_eq!(parse(""), None);
    }

    #[test]
    fn other_parameters_do_not_confuse_it() {
        assert_eq!(
            parse("kaizen-andon://connect?utm=x&server=kaizen.tetrix.dev&ref=y"),
            Some(Intent::Connect {
                server: "https://kaizen.tetrix.dev".into()
            })
        );
    }

    #[test]
    fn the_link_is_found_among_launch_arguments() {
        let args = vec![
            "C:\\Program Files\\Kaizen\\kaizen.exe".to_string(),
            "kaizen-andon://connect?server=kaizen.tetrix.dev".to_string(),
        ];

        assert_eq!(
            from_args(args),
            Some(Intent::Connect {
                server: "https://kaizen.tetrix.dev".into()
            })
        );

        assert_eq!(from_args(vec!["kaizen.exe".to_string()]), None);
    }
}
