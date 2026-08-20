//! Kaizen Desktop: a lamp for time that is not accounted for yet.
//!
//! The rules live server-side in Kaizen's own `App\Support\Ledger`, so this
//! process holds almost no logic. What it owns is where the window sits, when
//! it is open, and which build is installed.

pub mod api;
pub mod auth;
pub mod deeplink;
pub mod ledger;
pub mod placement;
pub mod state;
pub mod vault;

use serde_json::json;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use tauri_plugin_updater::UpdaterExt;

/// What the frontend receives for a deep link.
fn intent_payload(intent: &deeplink::Intent) -> serde_json::Value {
    match intent {
        deeplink::Intent::Connect { server } => json!({ "action": "connect", "server": server }),
        deeplink::Intent::Show => json!({ "action": "show" }),
    }
}

/// Compact CARD: one glyph, one number, one strip. Readable in a glance.
/// The window is this plus the room its shadow needs on either side.
const COMPACT: (i32, i32) = (292, 88);

/// The shortest window worth drawing, in logical pixels.
///
/// There is no matching maximum here on purpose. HEIGHT IS NOT DECLARED
/// ANYWHERE IN THIS APP: the page measures whatever it currently is and sends
/// the number, for every state and every message inside a state. A constant
/// would be a guess about how text wraps in a font this machine may not have,
/// and it is wrong the first time an error message runs to three lines. The
/// only ceiling is the work area, applied below, because a window taller than
/// the screen helps nobody.
const MIN_HEIGHT: i32 = 56;

const TRAY_ID: &str = "kaizen";

/// Put the window where it belongs and size it to what the page says it is.
///
/// Every state shares the bottom-right anchor, so growing happens up and to
/// the left and the lamp itself never moves.
///
/// `expanded` decides the WIDTH, which is a design decision. `height` is
/// whatever the page measured, which is not.
#[tauri::command]
fn place_window(window: WebviewWindow, expanded: bool, height: Option<i32>) -> Result<(), String> {
    let scale = window.scale_factor().unwrap_or(1.0);

    let area = placement::work_area().unwrap_or_else(|| {
        // No work area to ask for (or not Windows): fall back to the monitor's
        // own bounds, which means the taskbar is not excluded. Better than
        // refusing to show at all.
        let monitor = window
            .current_monitor()
            .ok()
            .flatten()
            .map(|m| {
                (
                    m.position().x,
                    m.position().y,
                    m.size().width as i32,
                    m.size().height as i32,
                )
            })
            .unwrap_or((0, 0, 1920, 1080));

        placement::Rect {
            left: monitor.0,
            top: monitor.1,
            right: monitor.0 + monitor.2,
            bottom: monitor.1 + monitor.3,
        }
    });

    let logical = |n: i32| (n as f64 * scale).round() as i32;

    let width = if expanded {
        placement::expanded_width(area, scale)
    } else {
        logical(COMPACT.0 + placement::SHADOW_ROOM * 2)
    };

    // The page has not measured itself yet on the very first call, so the
    // compact height stands in until it has.
    let asked = logical(height.unwrap_or(COMPACT.1));
    let ceiling = (area.height() - logical(placement::MARGIN) * 2).max(logical(MIN_HEIGHT));
    let height = asked.clamp(logical(MIN_HEIGHT), ceiling);

    let (x, y) = placement::anchor(area, width, height, scale);

    // Moved and resized as ONE change. Done as two calls the window is briefly
    // its old size at its new corner, which is a card that jumps and then
    // grows rather than a card that opens.
    window
        .set_bounds(tauri::Rect {
            position: tauri::Position::Physical(PhysicalPosition::new(x, y)),
            size: tauri::Size::Physical(PhysicalSize::new(width as u32, height as u32)),
        })
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// The stored configuration: which Kaizen this is pointed at, and what the
/// day currently looks like. Tokens never live here; they go to the OS
/// The stored configuration: which Kaizen this is pointed at, and what the
/// day currently looks like. Tokens never live here; they go to the OS
/// credential store once the OAuth flow exists.
#[tauri::command]
fn load_config(app: AppHandle) -> Result<state::Config, String> {
    state::Config::load(&app).map_err(|e| e.to_string())
}

/// What a copied error report carries besides the message.
///
/// A report saying only "error decoding response body" costs a round trip to
/// find out which build, which Kaizen and which machine, and the person who
/// hit it has usually moved on by then. None of this is secret: the version is
/// public, the address is one the user typed, and no token goes near it.
#[tauri::command]
fn diagnostics(app: AppHandle) -> Result<serde_json::Value, String> {
    let config = state::Config::load(&app).unwrap_or_default();

    Ok(serde_json::json!({
        "version": app.package_info().version.to_string(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "server": config.server_url,
        "connected": config.client_id.is_some(),
        "token_store": vault::backend(),
    }))
}

/// Connect to a Kaizen: discover, register if this install has not before, and
/// hand the browser the consent page. Blocks until the user comes back or
/// three minutes pass, so it runs off the UI thread.
///
/// The client is registered ONCE and reused. RFC 8252 section 7.3 requires an
/// authorization server to accept any port on a loopback redirect, precisely
/// so a native app need not reserve one, and Kaizen honours it (checked
/// against the live server). So the port may differ on every launch while the
/// client stays the same, and reconnecting does not litter the server with
/// dead clients nobody will ever revoke.
#[tauri::command(async)]
fn connect(app: AppHandle, server: String) -> Result<String, String> {
    let server = auth::normalise_server(&server).ok_or("that does not look like an address")?;

    let discovery = auth::discover(&server)?;
    let (listener, redirect) = auth::bind_loopback()
        .map_err(|e| format!("could not open a local port to come back to: {e}"))?;

    let client_id = match state::Config::load(&app)
        .unwrap_or_default()
        .client_for(&server)
    {
        Some(id) => id,
        None => auth::register(&discovery, &redirect)?.client_id,
    };

    let pkce = auth::Pkce::generate();
    let expected_state = auth::random_token(24);
    let url = auth::authorize_url(&discovery, &client_id, &redirect, &pkce, &expected_state);

    tauri_plugin_opener::open_url(&url, None::<&str>)
        .map_err(|e| format!("could not open your browser: {e}"))?;

    let callback = auth::await_callback(&listener, std::time::Duration::from_secs(180))
        .map_err(|e| format!("no answer came back from the browser: {e}"))?;

    let (code, returned_state) = match callback {
        auth::Callback::Granted { code, state } => (code, state),
        auth::Callback::Denied { error } => return Err(format!("access was refused: {error}")),
    };

    // The state check is the whole defence against a redirect somebody else
    // started being spent as though it were ours.
    if returned_state != expected_state {
        return Err("the answer did not match the request; nothing was connected".into());
    }

    let tokens = auth::exchange_code(&discovery, &client_id, &redirect, &code, &pkce.verifier)?;

    vault::save(&vault::Tokens {
        access_token: Some(tokens.access_token),
        refresh_token: tokens.refresh_token,
    })
    .map_err(|e| format!("could not store the token: {e}"))?;

    let mut config = state::Config::load(&app).unwrap_or_default();
    config.server_url = Some(server.clone());
    config.client_id = Some(client_id);
    config.save(&app).map_err(|e| e.to_string())?;

    Ok(server)
}

/// Forget this Kaizen. The token goes first: a half-disconnected app that
/// still holds a live token is worse than either state.
#[tauri::command]
fn disconnect(app: AppHandle) -> Result<(), String> {
    vault::clear()?;

    let mut config = state::Config::load(&app).unwrap_or_default();
    config.server_url = None;
    config.client_id = None;
    config.save(&app).map_err(|e| e.to_string())
}

/// Every call to Kaizen, with one retry through the refresh token.
///
/// Factored into one place because the retry is the part that gets forgotten:
/// an access token lives a day, so a command written without it works all
/// afternoon and fails the next morning, in a way that looks like the server
/// is down rather than like a missing five lines. `fetch_prompt` was exactly
/// that until this existed.
fn send<T: serde::de::DeserializeOwned>(
    app: &AppHandle,
    build: impl Fn(&reqwest::blocking::Client, &str, &str) -> reqwest::blocking::RequestBuilder,
) -> Result<T, String> {
    let config = state::Config::load(app).map_err(|e| e.to_string())?;
    let server = config.server_url.clone().ok_or("not connected")?;
    let mut secrets = vault::load()?;
    let token = secrets.access_token.clone().ok_or("not connected")?;
    let client = reqwest::blocking::Client::new();

    let once = |token: &str| {
        build(&client, &server, token)
            .timeout(std::time::Duration::from_secs(20))
            .send()
    };

    let response = once(&token).map_err(|e| format!("could not reach Kaizen: {e}"))?;

    let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let refresh_token = secrets
            .refresh_token
            .clone()
            .ok_or("the session expired; connect again")?;
        let client_id = config.client_id.clone().ok_or("not connected")?;
        let discovery = auth::discover(&server)?;
        let tokens = auth::refresh(&discovery, &client_id, &refresh_token)?;

        secrets.access_token = Some(tokens.access_token.clone());
        if tokens.refresh_token.is_some() {
            secrets.refresh_token = tokens.refresh_token.clone();
        }
        vault::save(&secrets)?;

        once(&tokens.access_token).map_err(|e| format!("could not reach Kaizen: {e}"))?
    } else {
        response
    };

    let status = response.status();

    if !status.is_success() {
        // Kaizen says WHY in the body, and for a refused entry that reason is
        // the whole message: "09:00–10:00 overlaps an entry already filed" is
        // something to act on, where "422 Unprocessable Content" is not.
        let body = response.text().unwrap_or_default();
        let said = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message")?.as_str().map(str::to_owned));

        return Err(said.unwrap_or_else(|| format!("Kaizen refused the request: {status}")));
    }

    response
        .json()
        .map_err(|e| format!("Kaizen answered something unexpected: {e}"))
}

/// A path with an optional `date`, which is what most of these are.
fn dated(server: &str, path: &str, date: &Option<String>) -> String {
    match date {
        Some(date) => format!("{}?date={date}", api::url(server, path)),
        None => api::url(server, path),
    }
}

/// A day, as Kaizen sees it. Today unless a date is given, which is what the
/// history grid asks for.
#[tauri::command(async)]
fn fetch_day(app: AppHandle, date: Option<String>) -> Result<api::Day, String> {
    send(&app, |client, server, token| {
        client.get(dated(server, "/day", &date)).bearer_auth(token)
    })
}

/// A month of tiles for the history grid.
#[tauri::command(async)]
fn fetch_month(app: AppHandle, month: Option<String>) -> Result<api::Month, String> {
    send(&app, |client, server, token| {
        let url = match &month {
            Some(month) => format!("{}?month={month}", api::url(server, "/month")),
            None => api::url(server, "/month"),
        };

        client.get(url).bearer_auth(token)
    })
}

/// File time. A batch, because accounting for a morning is one decision and
/// Kaizen refuses the whole set if any span overlaps another.
#[tauri::command(async)]
fn save_entries(
    app: AppHandle,
    entries: Vec<api::EntryDraft>,
    date: Option<String>,
) -> Result<api::Day, String> {
    if entries.is_empty() {
        return Err("nothing to file".into());
    }

    send(&app, |client, server, token| {
        client
            .post(dated(server, "/entries", &date))
            .bearer_auth(token)
            .json(&serde_json::json!({ "entries": entries }))
    })
}

/// Remove an entry, which reopens its span.
#[tauri::command(async)]
fn delete_entry(app: AppHandle, id: i64) -> Result<api::Day, String> {
    send(&app, |client, server, token| {
        client
            .delete(api::url(server, &format!("/entries/{id}")))
            .bearer_auth(token)
    })
}

/// Set the day's anchor. Without one nothing can be filed at all.
#[tauri::command(async)]
fn start_day(app: AppHandle, at: Option<String>, date: Option<String>) -> Result<api::Day, String> {
    send(&app, |client, server, token| {
        client
            .post(dated(server, "/day/start", &date))
            .bearer_auth(token)
            .json(&serde_json::json!({ "at": at }))
    })
}

/// Close a short day honestly, or reopen one closed too early.
#[tauri::command(async)]
fn end_day(
    app: AppHandle,
    at: Option<String>,
    reopen: Option<bool>,
    date: Option<String>,
) -> Result<api::Day, String> {
    send(&app, |client, server, token| {
        client
            .post(dated(server, "/day/end", &date))
            .bearer_auth(token)
            .json(&serde_json::json!({ "at": at, "reopen": reopen.unwrap_or(false) }))
    })
}

/// What the tray icon says on hover. The card may be hidden or snoozed; the
/// tray is the one surface that is always there, so it carries the number.
#[tauri::command]
fn set_tooltip(app: AppHandle, text: String) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_tooltip(Some(&text)).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Put the lamp down until it has something else to say.
///
/// Only from Running: from Attention up there is no hiding, which is the rule
/// that makes the end of the day always show.
#[tauri::command]
fn snooze(app: AppHandle, state: String, today: String) -> Result<(), String> {
    if state != "running" {
        return Err("only a quiet lamp can be put down".into());
    }

    let mut config = state::Config::load(&app).map_err(|e| e.to_string())?;
    config.snoozed_at_state = Some(state);
    config.snoozed_on = Some(today);
    config.save(&app).map_err(|e| e.to_string())?;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }

    Ok(())
}

#[tauri::command]
fn wake(app: AppHandle) -> Result<(), String> {
    let mut config = state::Config::load(&app).map_err(|e| e.to_string())?;
    config.snoozed_at_state = None;
    config.snoozed_on = None;
    config.save(&app).map_err(|e| e.to_string())?;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
    }

    Ok(())
}

/// The clipboard text: the day as it stands, its holes, and the user's own
/// instructions verbatim.
#[tauri::command(async)]
fn fetch_prompt(app: AppHandle, date: Option<String>) -> Result<api::Prompt, String> {
    send(&app, |client, server, token| {
        client
            .get(dated(server, "/prompt", &date))
            .bearer_auth(token)
    })
}

/// Fetch and install a newer build, if there is one.
///
/// This runs once, at launch, and it does not ask. A lamp has no unsaved work
/// to lose, and a 292px card has nowhere to put a dialog, so a prompt here
/// would only ever be something to dismiss. What makes not asking safe is the
/// signature: the download is verified against the public key compiled into
/// this binary before any of it is run, so whoever can serve a `latest.json`
/// still cannot serve code.
async fn install_any_update(app: tauri::AppHandle) -> tauri_plugin_updater::Result<()> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(());
    };

    update.download_and_install(|_, _| {}, || {}).await?;

    // Diverges, so nothing follows it. On Windows the installer usually ends
    // the process before this is reached anyway.
    app.restart()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Single instance first, and it matters: a deep link launches the exe
        // again, so without this a second copy would sit holding the URL while
        // the first shows an unconnected card.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }

            if let Some(intent) = deeplink::from_args(argv) {
                let _ = app.emit("deep-link", intent_payload(&intent));
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            place_window,
            load_config,
            diagnostics,
            connect,
            disconnect,
            fetch_day,
            fetch_month,
            fetch_prompt,
            save_entries,
            delete_entry,
            start_day,
            end_day,
            set_tooltip,
            snooze,
            wake
        ])
        .setup(|app| {
            // Deliberately not awaited: the card belongs on screen whether or
            // not GitHub is reachable, and a failed check is not worth saying
            // anything about. The next launch tries again.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = install_any_update(handle).await {
                    eprintln!("update check: {e}");
                }
            });

            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("Kaizen")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        // Showing from the tray also wakes it: somebody
                        // reaching for the menu is asking to see it again.
                        let _ = wake(app.clone());
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let _ = place_window(window, false, None);
            }

            // A cold start from a deep link: the URL is in our own arguments.
            if let Some(intent) = deeplink::from_args(std::env::args()) {
                let handle = app.handle().clone();
                let payload = intent_payload(&intent);
                // After setup, or the frontend is not listening yet.
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    let _ = handle.emit("deep-link", payload);
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Kaizen Desktop");
}
