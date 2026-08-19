//! A live probe of the connect flow against a real Kaizen, up to the point a
//! human has to approve in a browser.
//!
//! Run: cargo run --example probe -- https://kaizen.tetrix.dev [--wait]
//!
//! Exists because "it compiles" is not the same as "the server agrees". This
//! walks discovery, registration and the authorize URL, and prints the URL a
//! person would open.
use kaizen_andon_lib::auth;

fn main() {
    let server = std::env::args()
        .nth(1)
        .and_then(|s| auth::normalise_server(&s))
        .expect("pass a Kaizen address");

    println!("server:     {server}");

    let discovery = auth::discover(&server).expect("discovery");
    println!("issuer:     {}", discovery.issuer);
    println!("authorize:  {}", discovery.authorization_endpoint);
    println!("pkce S256:  {}", discovery.supports_pkce());
    println!("scope:      {}", discovery.scope());

    let (listener, redirect) = auth::bind_loopback().expect("loopback");
    println!("listening:  {redirect}");

    let registration = auth::register(&discovery, &redirect).expect("registration");
    println!("client_id:  {}", registration.client_id);
    println!(
        "secret:     {:?} (none is correct for a public client)",
        registration.client_secret
    );

    let pkce = auth::Pkce::generate();
    let state = auth::random_token(24);
    let url = auth::authorize_url(
        &discovery,
        &registration.client_id,
        &redirect,
        &pkce,
        &state,
    );

    println!("\nopen this to approve:\n{url}\n");

    if std::env::args().any(|a| a == "--wait") {
        println!("waiting up to 2 minutes for the browser...");
        match auth::await_callback(&listener, std::time::Duration::from_secs(120)) {
            Ok(auth::Callback::Granted { code, state: back }) => {
                assert_eq!(back, state, "state must come back unchanged");
                let tokens = auth::exchange_code(
                    &discovery,
                    &registration.client_id,
                    &redirect,
                    &code,
                    &pkce.verifier,
                )
                .expect("token exchange");
                println!(
                    "access token:  {}...",
                    &tokens.access_token[..24.min(tokens.access_token.len())]
                );
                println!("refresh token: {}", tokens.refresh_token.is_some());
            }
            Ok(auth::Callback::Denied { error }) => println!("denied: {error}"),
            Err(e) => println!("no callback: {e}"),
        }
    }
}
