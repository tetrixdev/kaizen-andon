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

/// What the frontend receives for a deep link.
fn intent_payload(intent: &deeplink::Intent) -> serde_json::Value {
    match intent {
        deeplink::Intent::Connect { server } => json!({ "action": "connect", "server": server }),
        deeplink::Intent::Show => json!({ "action": "show" }),
    }
}

/// Compact card: one glyph, one number, one strip. Readable in a glance.
const COMPACT: (i32, i32) = (292, 88);

/// Expanded: the bar plus the ledger above it.
const EXPANDED_HEIGHT: i32 = 420;

const TRAY_ID: &str = "kaizen";

/// Put the window where it belongs and size it for the state it is in.
///
/// Compact and expanded share the bottom-right anchor, so opening grows the
/// card up and to the left and the lamp itself never moves.
#[tauri::command]
fn place_window(window: WebviewWindow, expanded: bool) -> Result<(), String> {
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

    let (width, height) = if expanded {
        (
            placement::expanded_width(area, scale),
            (EXPANDED_HEIGHT as f64 * scale).round() as i32,
        )
    } else {
        (
            (COMPACT.0 as f64 * scale).round() as i32,
            (COMPACT.1 as f64 * scale).round() as i32,
        )
    };

    let (x, y) = placement::anchor(area, width, height, scale);

    window
        .set_size(PhysicalSize::new(width as u32, height as u32))
        .map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// The stored configuration: which Kaizen this is pointed at, and what the
/// day currently looks like. Tokens never live here; they go to the OS
/// credential store once the OAuth flow exists.
#[tauri::command]
fn load_config(app: AppHandle) -> Result<state::Config, String> {
    state::Config::load(&app).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_config(app: AppHandle, config: state::Config) -> Result<(), String> {
    config.save(&app).map_err(|e| e.to_string())
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

/// Today, as Kaizen sees it. One retry through the refresh token, because an
/// expired access token is the ordinary case rather than a failure.
#[tauri::command(async)]
fn fetch_day(app: AppHandle, date: Option<String>) -> Result<api::Day, String> {
    let config = state::Config::load(&app).map_err(|e| e.to_string())?;
    let server = config.server_url.ok_or("not connected")?;
    let mut secrets = vault::load()?;
    let token = secrets.access_token.clone().ok_or("not connected")?;

    let call = |token: &str| {
        let mut url = api::url(&server, "/day");
        if let Some(date) = &date {
            url = format!("{url}?date={date}");
        }

        reqwest::blocking::Client::new()
            .get(url)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(20))
            .send()
    };

    let response = call(&token).map_err(|e| format!("could not reach Kaizen: {e}"))?;

    let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        let refresh_token = secrets
            .refresh_token
            .clone()
            .ok_or("the session expired; connect again")?;
        let client_id = config.client_id.ok_or("not connected")?;
        let discovery = auth::discover(&server)?;
        let tokens = auth::refresh(&discovery, &client_id, &refresh_token)?;

        secrets.access_token = Some(tokens.access_token.clone());
        if tokens.refresh_token.is_some() {
            secrets.refresh_token = tokens.refresh_token.clone();
        }
        vault::save(&secrets)?;

        call(&tokens.access_token).map_err(|e| format!("could not reach Kaizen: {e}"))?
    } else {
        response
    };

    response
        .error_for_status()
        .map_err(|e| format!("Kaizen refused the request: {e}"))?
        .json()
        .map_err(|e| format!("Kaizen answered something unexpected: {e}"))
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

/// The clipboard text: the day, its holes, and the context's own instructions.
#[tauri::command(async)]
fn fetch_prompt(app: AppHandle, date: Option<String>) -> Result<api::Prompt, String> {
    let config = state::Config::load(&app).map_err(|e| e.to_string())?;
    let server = config.server_url.ok_or("not connected")?;
    let token = vault::load()?.access_token.ok_or("not connected")?;

    let mut url = api::url(&server, "/prompt");
    if let Some(date) = date {
        url = format!("{url}?date={date}");
    }

    reqwest::blocking::Client::new()
        .get(url)
        .bearer_auth(&token)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .map_err(|e| format!("could not reach Kaizen: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Kaizen refused the request: {e}"))?
        .json()
        .map_err(|e| format!("Kaizen answered something unexpected: {e}"))
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
        .invoke_handler(tauri::generate_handler![
            place_window,
            load_config,
            save_config,
            connect,
            disconnect,
            fetch_day,
            fetch_prompt,
            set_tooltip,
            snooze,
            wake
        ])
        .setup(|app| {
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
                let _ = place_window(window, false);
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
