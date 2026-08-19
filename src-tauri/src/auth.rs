//! Connecting to a Kaizen.
//!
//! The build is generic: no instance is baked in, because one installer has to
//! serve a dev instance, a production one, or somebody else's entirely. So
//! everything is discovered from the address the user gives:
//!
//!   1. GET /.well-known/oauth-authorization-server
//!   2. Register this install as a client (RFC 7591). Kaizen already accepts
//!      this unauthenticated, which is how claude.ai registers its connector.
//!   3. Authorization code with PKCE and a loopback redirect (RFC 8252), which
//!      is the native-app flow: no client secret exists to leak, because the
//!      binary is on the user's machine and could always be read.
//!
//! The loopback port is bound BEFORE registering, so the redirect URI we
//! register is the one we are actually listening on rather than a guess.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::time::Duration;

/// What the discovery document tells us. Only the fields we act on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

impl Discovery {
    /// PKCE is not optional here. A public client with no secret and no proof
    /// key is an authorization code anyone who can see the redirect can spend.
    pub fn supports_pkce(&self) -> bool {
        self.code_challenge_methods_supported
            .iter()
            .any(|m| m == "S256")
    }

    pub fn scope(&self) -> String {
        if self.scopes_supported.is_empty() {
            "mcp:use".into()
        } else {
            self.scopes_supported.join(" ")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registration {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub token_type: Option<String>,
}

/// A PKCE pair: the secret we keep and the digest we publish.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        Self::from_verifier(random_token(64))
    }

    pub fn from_verifier(verifier: String) -> Self {
        let digest = Sha256::digest(verifier.as_bytes());

        Self {
            challenge: base64url(&digest),
            verifier,
        }
    }
}

/// Unreserved characters only, so the verifier never needs escaping.
/// RFC 7636 wants 43 to 128 of them.
pub fn random_token(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("the OS always has randomness");

    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

/// base64url without padding, which is what RFC 7636 asks for.
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);

        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }

    out
}

/// A Kaizen address as typed by a human, made into a base URL.
///
/// People paste "kaizen.tetrix.dev", "https://kaizen.tetrix.dev/" and
/// "https://kaizen.tetrix.dev/more/time" interchangeably; all three mean the
/// same instance.
pub fn normalise_server(input: &str) -> Option<String> {
    let trimmed = input.trim().trim_end_matches('/');

    if trimmed.is_empty() {
        return None;
    }

    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    // Keep scheme and authority, drop any path someone copied from the bar.
    let rest = with_scheme.split_once("://")?;
    let host = rest.1.split('/').next()?;

    if host.is_empty() {
        return None;
    }

    Some(format!("{}://{}", rest.0, host))
}

pub fn discovery_url(server: &str) -> String {
    format!("{server}/.well-known/oauth-authorization-server")
}

/// Percent-encode a query parameter value. Small on purpose: the values here
/// are URLs, tokens and scopes, not arbitrary user text.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }

    out
}

pub fn authorize_url(
    discovery: &Discovery,
    client_id: &str,
    redirect_uri: &str,
    pkce: &Pkce,
    state: &str,
) -> String {
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        discovery.authorization_endpoint,
        encode(client_id),
        encode(redirect_uri),
        encode(&discovery.scope()),
        encode(state),
        encode(&pkce.challenge),
    )
}

/// What came back on the loopback redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    Granted { code: String, state: String },
    Denied { error: String },
}

/// Pull the result out of the redirect's query string.
pub fn parse_callback(target: &str) -> Option<Callback> {
    let query = target.split_once('?')?.1;
    let mut code = None;
    let mut state = None;
    let mut error = None;

    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        let value = percent_decode(value);

        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }

    if let Some(error) = error {
        return Some(Callback::Denied { error });
    }

    Some(Callback::Granted {
        code: code?,
        state: state.unwrap_or_default(),
    })
}

fn percent_decode(value: &str) -> String {
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

/// Bind the loopback listener FIRST, so the redirect URI we register is the
/// one we are really listening on. 127.0.0.1 only: never 0.0.0.0, or the
/// authorization code would be offered to the whole network.
pub fn bind_loopback() -> std::io::Result<(TcpListener, String)> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    let port = listener.local_addr()?.port();

    Ok((listener, format!("http://127.0.0.1:{port}/callback")))
}

/// What the browser is left looking at once the code is captured.
const CLOSING_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Kaizen</title>\
<style>html{background:#f7f4ee;color:#1c1917;font-family:'Segoe UI',system-ui,sans-serif}\
div{position:absolute;inset:0;display:grid;place-items:center;text-align:center}\
p{max-width:32ch;line-height:1.6}span{font-family:'Iowan Old Style',Palatino,Georgia,serif;\
font-size:56px;color:#d9483b;display:block;margin-bottom:12px}</style>\
<div><p><span>灯</span>Connected. You can close this tab and go back to the app.</p></div>";

/// Wait for the browser to come back. Gives up rather than hanging forever:
/// a user who closed the tab should get a plain failure, not a stuck app.
pub fn await_callback(listener: &TcpListener, timeout: Duration) -> std::io::Result<Callback> {
    listener.set_nonblocking(false)?;
    let deadline = std::time::Instant::now() + timeout;

    for stream in listener.incoming() {
        let mut stream = stream?;

        if std::time::Instant::now() > deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the browser never came back",
            ));
        }

        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;

        // "GET /callback?code=...&state=... HTTP/1.1"
        let target = request_line.split_whitespace().nth(1).unwrap_or("");

        if let Some(callback) = parse_callback(target) {
            let body = CLOSING_PAGE.as_bytes();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes())?;
            stream.write_all(body)?;
            stream.flush()?;

            return Ok(callback);
        }

        // Browsers ask for /favicon.ico on the way; ignore anything that is
        // not the redirect rather than treating it as a failed login.
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "the listener closed before the browser came back",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_the_rfc_examples() {
        // RFC 4648 test vectors, minus the padding RFC 7636 forbids.
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64url_uses_the_url_safe_alphabet() {
        // These bytes produce + and / in standard base64, which would need
        // escaping in a query string.
        let encoded = base64url(&[0xfb, 0xff, 0xfe]);

        assert!(!encoded.contains('+'), "{encoded} must not contain +");
        assert!(!encoded.contains('/'), "{encoded} must not contain /");
        assert!(!encoded.contains('='), "{encoded} must not be padded");
    }

    #[test]
    fn the_pkce_challenge_matches_the_rfc_worked_example() {
        // RFC 7636 appendix B.
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());

        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_generated_verifier_is_long_enough_and_needs_no_escaping() {
        let pkce = Pkce::generate();

        assert!(pkce.verifier.len() >= 43 && pkce.verifier.len() <= 128);
        assert_eq!(encode(&pkce.verifier), pkce.verifier, "unreserved only");
        assert_ne!(Pkce::generate().verifier, pkce.verifier, "not a constant");
    }

    #[test]
    fn an_address_is_accepted_however_it_was_typed() {
        for input in [
            "kaizen.tetrix.dev",
            "https://kaizen.tetrix.dev",
            "https://kaizen.tetrix.dev/",
            "  https://kaizen.tetrix.dev/more/time  ",
        ] {
            assert_eq!(
                normalise_server(input).as_deref(),
                Some("https://kaizen.tetrix.dev"),
                "for {input:?}"
            );
        }

        assert_eq!(
            normalise_server("http://localhost:8080/x").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(normalise_server("   "), None);
        assert_eq!(normalise_server(""), None);
    }

    #[test]
    fn the_authorize_url_carries_the_challenge_and_never_the_verifier() {
        let discovery = Discovery {
            issuer: "https://kaizen.tetrix.dev".into(),
            authorization_endpoint: "https://kaizen.tetrix.dev/oauth/authorize".into(),
            token_endpoint: "https://kaizen.tetrix.dev/oauth/token".into(),
            registration_endpoint: None,
            scopes_supported: vec!["mcp:use".into()],
            code_challenge_methods_supported: vec!["S256".into()],
        };
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        let url = authorize_url(
            &discovery,
            "abc-123",
            "http://127.0.0.1:51789/callback",
            &pkce,
            "st8",
        );

        assert!(url.contains("code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A51789%2Fcallback"));
        assert!(url.contains("state=st8"));
        assert!(
            !url.contains(&pkce.verifier),
            "the verifier must never leave this process until the token call"
        );
    }

    #[test]
    fn pkce_is_required_of_the_server() {
        let mut discovery = Discovery {
            issuer: "x".into(),
            authorization_endpoint: "x".into(),
            token_endpoint: "x".into(),
            registration_endpoint: None,
            scopes_supported: vec![],
            code_challenge_methods_supported: vec!["S256".into()],
        };
        assert!(discovery.supports_pkce());

        discovery.code_challenge_methods_supported = vec!["plain".into()];
        assert!(!discovery.supports_pkce(), "plain is not proof of anything");

        // And a server that says nothing about scopes still gets the default.
        assert_eq!(discovery.scope(), "mcp:use");
    }

    #[test]
    fn the_callback_is_read_off_the_redirect() {
        assert_eq!(
            parse_callback("/callback?code=abc&state=xyz"),
            Some(Callback::Granted {
                code: "abc".into(),
                state: "xyz".into()
            })
        );

        assert_eq!(
            parse_callback("/callback?error=access_denied&state=xyz"),
            Some(Callback::Denied {
                error: "access_denied".into()
            })
        );

        // Percent-encoded values survive the trip.
        assert_eq!(
            parse_callback("/callback?code=a%2Bb%2Fc&state=s"),
            Some(Callback::Granted {
                code: "a+b/c".into(),
                state: "s".into()
            })
        );

        assert_eq!(parse_callback("/favicon.ico"), None, "not the redirect");
    }

    #[test]
    fn the_loopback_binds_to_localhost_only() {
        let (listener, redirect) = bind_loopback().expect("binds");
        let addr = listener.local_addr().expect("has an address");

        assert_eq!(
            addr.ip().to_string(),
            "127.0.0.1",
            "never the whole network"
        );
        assert!(addr.port() > 0, "an ephemeral port was chosen");
        assert!(redirect.starts_with("http://127.0.0.1:"));
        assert!(redirect.ends_with("/callback"));
    }

    #[test]
    fn discovery_parses_what_kaizen_actually_returns() {
        // Copied from kaizen.tetrix.dev.
        let json = r#"{
            "issuer": "https://kaizen.tetrix.dev",
            "authorization_endpoint": "https://kaizen.tetrix.dev/oauth/authorize",
            "token_endpoint": "https://kaizen.tetrix.dev/oauth/token",
            "registration_endpoint": "https://kaizen.tetrix.dev/oauth/register",
            "response_types_supported": ["code"],
            "code_challenge_methods_supported": ["S256"],
            "scopes_supported": ["mcp:use"],
            "grant_types_supported": ["authorization_code", "refresh_token"]
        }"#;

        let discovery: Discovery = serde_json::from_str(json).expect("parses");

        assert!(discovery.supports_pkce());
        assert_eq!(discovery.scope(), "mcp:use");
        assert_eq!(
            discovery.registration_endpoint.as_deref(),
            Some("https://kaizen.tetrix.dev/oauth/register")
        );
    }

    #[test]
    fn a_registration_response_parses_without_a_secret() {
        // Kaizen returns a public client: there is no secret to keep, because
        // the binary is on the user's machine and could always be read.
        let json = r#"{
            "client_id": "01a01b01-c563-72dc-9ed2-a0dc5cb9d3c4",
            "grant_types": ["authorization_code", "refresh_token"],
            "redirect_uris": ["http://127.0.0.1:51789/callback"],
            "token_endpoint_auth_method": "none"
        }"#;

        let registration: Registration = serde_json::from_str(json).expect("parses");

        assert_eq!(
            registration.client_id,
            "01a01b01-c563-72dc-9ed2-a0dc5cb9d3c4"
        );
        assert!(registration.client_secret.is_none());
    }
}

// ── The flow, end to end ────────────────────────────────────────────────

/// Everything worth keeping once a connection is made. The refresh token is
/// the sensitive half; `state.rs` decides where it lands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub server: String,
    pub client_id: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("KaizenDesktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("a client with no proxy configuration always builds")
}

pub fn discover(server: &str) -> Result<Discovery, String> {
    let discovery: Discovery = client()
        .get(discovery_url(server))
        .send()
        .map_err(|e| format!("could not reach {server}: {e}"))?
        .error_for_status()
        .map_err(|_| format!("{server} does not look like a Kaizen"))?
        .json()
        .map_err(|e| format!("{server} answered something unexpected: {e}"))?;

    if !discovery.supports_pkce() {
        return Err(
            "that server will not do PKCE, and this app will not connect without it".into(),
        );
    }

    Ok(discovery)
}

/// Register this install. Kaizen accepts this unauthenticated, which is how
/// claude.ai registers its connector, so nothing has to ship in the binary.
pub fn register(discovery: &Discovery, redirect_uri: &str) -> Result<Registration, String> {
    let endpoint = discovery
        .registration_endpoint
        .as_ref()
        .ok_or("that server does not offer client registration")?;

    client()
        .post(endpoint)
        .json(&serde_json::json!({
            "client_name": "Kaizen Desktop",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .map_err(|e| format!("registration failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("registration was refused: {e}"))?
        .json()
        .map_err(|e| format!("registration answered something unexpected: {e}"))
}

pub fn exchange_code(
    discovery: &Discovery,
    client_id: &str,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<Tokens, String> {
    client()
        .post(&discovery.token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("code", code),
            ("code_verifier", verifier),
        ])
        .send()
        .map_err(|e| format!("the token call failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("the token call was refused: {e}"))?
        .json()
        .map_err(|e| format!("the token call answered something unexpected: {e}"))
}

/// Access tokens expire; the refresh token is what keeps the lamp lit without
/// sending the user back to a browser every hour.
pub fn refresh(
    discovery: &Discovery,
    client_id: &str,
    refresh_token: &str,
) -> Result<Tokens, String> {
    client()
        .post(&discovery.token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
        ])
        .send()
        .map_err(|e| format!("could not refresh: {e}"))?
        .error_for_status()
        .map_err(|_| "the refresh token is no longer good; connect again".to_string())?
        .json()
        .map_err(|e| format!("the refresh answered something unexpected: {e}"))
}
