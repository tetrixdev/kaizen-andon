//! Kaizen Desktop: a lamp for time that is not accounted for yet.
//!
//! The rules live server-side in Kaizen's own `App\Support\Ledger`, so this
//! process holds almost no logic. What it owns is where the window sits, when
//! it is open, and which build is installed.

pub mod api;
pub mod auth;
pub mod ledger;
pub mod placement;
pub mod state;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// Compact card: one glyph, one number, one strip. Readable in a glance.
const COMPACT: (i32, i32) = (292, 88);

/// Expanded: the bar plus the ledger above it.
const EXPANDED_HEIGHT: i32 = 420;

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

/// Connect to a Kaizen: discover, register, and hand the browser the consent
/// page. Blocks until the user comes back or two minutes pass, so it runs off
/// the UI thread.
///
/// Re-registers on every connect rather than reusing a stored client, because
/// the redirect URI has to name the port we actually bound and that port is
/// not ours to reserve between runs.
#[tauri::command(async)]
fn connect(app: AppHandle, server: String) -> Result<String, String> {
    let server = auth::normalise_server(&server).ok_or("that does not look like an address")?;

    let discovery = auth::discover(&server)?;
    let (listener, redirect) = auth::bind_loopback()
        .map_err(|e| format!("could not open a local port to come back to: {e}"))?;
    let registration = auth::register(&discovery, &redirect)?;

    let pkce = auth::Pkce::generate();
    let expected_state = auth::random_token(24);
    let url = auth::authorize_url(
        &discovery,
        &registration.client_id,
        &redirect,
        &pkce,
        &expected_state,
    );

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

    let tokens = auth::exchange_code(
        &discovery,
        &registration.client_id,
        &redirect,
        &code,
        &pkce.verifier,
    )?;

    state::Secrets {
        access_token: Some(tokens.access_token),
        refresh_token: tokens.refresh_token,
    }
    .save(&app)
    .map_err(|e| format!("could not store the token: {e}"))?;

    let mut config = state::Config::load(&app).unwrap_or_default();
    config.server_url = Some(server.clone());
    config.client_id = Some(registration.client_id);
    config.save(&app).map_err(|e| e.to_string())?;

    Ok(server)
}

/// Forget this Kaizen. The token goes first: a half-disconnected app that
/// still holds a live token is worse than either state.
#[tauri::command]
fn disconnect(app: AppHandle) -> Result<(), String> {
    state::Secrets::clear(&app).map_err(|e| e.to_string())?;

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
    let mut secrets = state::Secrets::load(&app).map_err(|e| e.to_string())?;
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
        secrets.save(&app).map_err(|e| e.to_string())?;

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

/// The clipboard text: the day, its holes, and the context's own instructions.
#[tauri::command(async)]
fn fetch_prompt(app: AppHandle, date: Option<String>) -> Result<api::Prompt, String> {
    let config = state::Config::load(&app).map_err(|e| e.to_string())?;
    let server = config.server_url.ok_or("not connected")?;
    let secrets = state::Secrets::load(&app).map_err(|e| e.to_string())?;
    let token = secrets.access_token.ok_or("not connected")?;

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
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            place_window,
            load_config,
            save_config,
            connect,
            disconnect,
            fetch_day,
            fetch_prompt
        ])
        .setup(|app| {
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("Kaizen")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let _ = place_window(window, false);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Kaizen Desktop");
}
