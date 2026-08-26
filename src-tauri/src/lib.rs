//! Kaizen Desktop: a lamp for time that is not accounted for yet.
//!
//! The rules live server-side in Kaizen's own `App\Support\Ledger`, so this
//! process holds almost no logic. What it owns is where the window sits, when
//! it is open, and which build is installed.

pub mod api;
pub mod auth;
pub mod capture;
pub mod deeplink;
pub mod ledger;
pub mod placement;
pub mod state;
pub mod vault;

use serde_json::json;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
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

/// What the tray knows about the lamp between polls.
///
/// The state lives in the frontend, which is the thing actually reading the
/// day; the tray is a second surface onto it. Rather than have Rust fetch the
/// day again on its own schedule (a second answer, free to drift), the page
/// pushes what it just drew, exactly as it already does for the tooltip.
#[derive(Default)]
struct Lamp {
    state: std::sync::Mutex<String>,
    hide: std::sync::Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>,

    /// Kaizen's answer to "is this minute inside a context that earns
    /// capture", and WHEN it said so.
    ///
    /// The rule for what a working day is lives in `App\Support\Ledger` and
    /// must not be reimplemented here, so the page pushes the answer with each
    /// poll. The timestamp is the important half: polling slows to five
    /// minutes while the lamp is quiet, so an answer can be stale, and a stale
    /// yes would keep recording past the end of the window. Staleness
    /// therefore reads as NO. Erring toward not capturing costs evidence;
    /// erring the other way records someone's evening.
    capture_window: std::sync::Mutex<Option<(CaptureKinds, std::time::Instant)>>,

    /// The two capture items, so their labels can follow the actual state
    /// rather than whatever they said when the tray was built.
    #[allow(clippy::type_complexity)]
    capture_menu: std::sync::Mutex<
        Option<(
            tauri::menu::MenuItem<tauri::Wry>,
            tauri::menu::MenuItem<tauri::Wry>,
        )>,
    >,
}

/// Make the tray say what is actually happening.
///
/// A toggle whose label lies about its own state is worse than no toggle: the
/// one thing it has to get right is whether the screen is being recorded.
fn refresh_capture_menu(app: &AppHandle) {
    let Ok(config) = state::Config::load(app) else {
        return;
    };

    if let Ok(items) = app.state::<Lamp>().capture_menu.lock() {
        if let Some((toggle, pause)) = items.as_ref() {
            let _ = toggle.set_text(if config.capture_enabled {
                "Stop capturing"
            } else {
                "Start capturing"
            });
            let _ = pause.set_text(if paused_now(&config) {
                "Resume capture"
            } else {
                "Pause capture for an hour"
            });
            let _ = pause.set_enabled(config.capture_enabled);
        }
    }
}

/// How old Kaizen's answer may be before it stops counting as an answer.
///
/// Comfortably longer than the five-minute quiet poll, so an ordinary slow
/// cycle does not stutter capture, and short enough that a page which has
/// stopped polling entirely stops capture within a couple of minutes.
const WINDOW_ANSWER_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 60);

use ledger::CaptureKinds;

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

    placement::set_bounds(&window, x, y, width, height)
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
/// What the lamp is showing right now, pushed from the page.
///
/// Hiding is allowed from every state EXCEPT call: amber is a nudge and may be
/// put away, red is the one that insists. The menu item is disabled rather
/// than merely refusing, so the rule is visible before it is discovered.
#[tauri::command]
fn set_lamp(app: AppHandle, state: String, capture: Option<CaptureKinds>) -> Result<(), String> {
    let lamp = app.state::<Lamp>();

    if let Ok(item) = lamp.hide.lock() {
        if let Some(item) = item.as_ref() {
            let _ = item.set_enabled(state != "call");
        }
    }
    if let Ok(mut held) = lamp.state.lock() {
        *held = state;
    }
    if let Ok(mut window) = lamp.capture_window.lock() {
        *window = capture.map(|kinds| (kinds, std::time::Instant::now()));
    }
    Ok(())
}

/// What capture is doing, for the widget's indicator and the tray.
#[tauri::command]
fn capture_status(app: AppHandle) -> Result<serde_json::Value, String> {
    let config = state::Config::load(&app).map_err(|e| e.to_string())?;
    let paused = paused_now(&config);

    Ok(json!({
        "enabled": config.capture_enabled,
        "paused": paused,
        // The clock the pause runs to, so the widget can say "until 14:30"
        // rather than leaving the operator to work out when it lapses.
        "paused_until": paused
            .then(|| config.capture_paused_until.as_deref().and_then(from_rfc3339_clock))
            .flatten(),
        "recording": config.capture_enabled && !paused && window_kinds(&app).any(),
        "screen": config.capture_enabled && !paused && window_kinds(&app).screen,
    }))
}

/// Turn capture on or off, and say so on disk immediately.
#[tauri::command]
fn set_capture(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut config = state::Config::load(&app).map_err(|e| e.to_string())?;
    config.capture_enabled = enabled;

    // Turning it on clears any pause. Leaving a forgotten pause behind an
    // enabled switch is the one state where the indicator would say recording
    // while nothing was recorded.
    if enabled {
        config.capture_paused_until = None;
    }

    config.save(&app).map_err(|e| e.to_string())
}

/// Stop until a moment, for a screenshare or anything else not ours to keep.
///
/// `until` is a wall clock "HH:MM" in the operator's own day, because that is
/// how the reason is actually shaped: the meeting ends at half past, not in
/// fifty-one minutes. A time already past today is read as tomorrow, so
/// pausing at 23:50 until 00:30 means what it looks like. `minutes` remains
/// for the tray's one-click hour.
///
/// Always a pause, never a stop: it lapses on its own, because the failure
/// everyone has with a manual mute is forgetting to undo it, and here
/// forgetting means silently keeping no evidence of an afternoon.
#[tauri::command]
fn pause_capture(
    app: AppHandle,
    minutes: Option<i64>,
    until: Option<String>,
) -> Result<(), String> {
    let now = chrono::Local::now();

    let at = match until.as_deref() {
        Some(clock) => {
            let (h, m) = clock.split_once(':').ok_or("a pause time is \"HH:MM\"")?;
            let (h, m) = (
                h.parse::<u32>().map_err(|_| "a pause time is \"HH:MM\"")?,
                m.parse::<u32>().map_err(|_| "a pause time is \"HH:MM\"")?,
            );

            let today = now
                .date_naive()
                .and_hms_opt(h, m, 0)
                .ok_or("no such time of day")?
                .and_local_timezone(chrono::Local)
                .single()
                .ok_or("that clock time is ambiguous today")?;

            // A time already gone means the next one, not a pause that expired
            // before it began.
            if today > now {
                today
            } else {
                today + chrono::Duration::days(1)
            }
        }
        None => now + chrono::Duration::minutes(minutes.unwrap_or(60)),
    };

    let mut config = state::Config::load(&app).map_err(|e| e.to_string())?;
    config.capture_paused_until = Some(at.to_rfc3339());
    config.save(&app).map_err(|e| e.to_string())
}

/// End a pause early.
#[tauri::command]
fn resume_capture(app: AppHandle) -> Result<(), String> {
    let mut config = state::Config::load(&app).map_err(|e| e.to_string())?;
    config.capture_paused_until = None;
    config.save(&app).map_err(|e| e.to_string())
}

/// The wall clock of a stored instant, for showing a pause's end.
fn from_rfc3339_clock(at: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
}

/// Where this install keeps its capture archive.
fn capture_root(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| capture::root_in(&dir))
}

fn paused_now(config: &state::Config) -> bool {
    config
        .capture_paused_until
        .as_deref()
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .is_some_and(|at| at > chrono::Local::now())
}

/// What Kaizen last said may be recorded, if that answer still counts.
///
/// A stale answer is nothing at all rather than a stale yes: polling slows to
/// five minutes when the lamp is quiet, and a stale yes would keep recording
/// past the end of a window. Erring this way costs evidence; erring the other
/// way records someone's evening.
fn window_kinds(app: &AppHandle) -> CaptureKinds {
    app.state::<Lamp>()
        .capture_window
        .lock()
        .ok()
        .and_then(|held| *held)
        .filter(|(_, at)| at.elapsed() < WINDOW_ANSWER_TTL)
        .map(|(kinds, _)| kinds)
        .unwrap_or_default()
}

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
    let mut prompt: api::Prompt = send(&app, |client, server, token| {
        client
            .get(dated(server, "/prompt", &date))
            .bearer_auth(token)
    })?;

    // The activity lines are attached HERE rather than by Kaizen, because
    // Kaizen has never seen them and must not: the archive is local, and the
    // recorder uploads nothing. Handing them to an AI is the operator's own
    // act, which is what pasting a prompt is.
    //
    // Gated on captures_activity, not on the file being non-empty: a
    // screen-only day still writes a line per tick (see
    // capture::Recorder::push), just with no process or title in it, so a
    // non-empty file alone cannot say whether the table is worth showing —
    // only Kaizen's own config for this context can.
    let day = prompt.date.clone();

    let activity = prompt
        .captures_activity
        .then(|| capture_root(&app).and_then(|root| capture::activity_for(&root, &day)))
        .flatten();

    if let Some(activity) = activity {
        prompt.prompt.push_str(
            "\n\n## What this machine was doing\n\n\
             One line per minute, local to this machine and never uploaded: time, seconds \
             idle, whether the frame was kept, the program in front, and its window title. \
             Use it to account for the gaps above, not as a record of hours in itself: it \
             says what was on screen, not what the work was.\n\n```\n",
        );
        prompt.prompt.push_str(activity.trim());
        prompt.prompt.push_str("\n```");
    }

    // The pictures themselves are still never attached — a title is a line
    // naming a program and a window, a picture is everything that was on the
    // screen, including whatever somebody else sent you — but an AI reading
    // this has no way to know they even exist unless told, so a context that
    // takes them gets one line saying so and how to ask for a specific one.
    if prompt.captures_screen {
        prompt.prompt.push_str(
            "\n\nScreenshots are also captured here, roughly one a minute, never uploaded. \
             If the log above still leaves a gap unclear, ask the person you're talking to \
             to open that minute's screenshot (tray icon → Open capture folder) rather than \
             guessing what filled it.",
        );
    }

    Ok(prompt)
}

/// Show the archive in Explorer.
#[tauri::command]
fn open_capture_folder(app: AppHandle) -> Result<(), String> {
    let root = capture_root(&app).ok_or("no archive directory on this machine")?;

    // Created on demand: before the first frame there is nothing to open, and
    // opening nothing reads as the feature being broken.
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;

    tauri_plugin_opener::open_path(root.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
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

/// The minute loop that records, and the three questions it asks first.
///
/// Every tick re-reads the config rather than caching it, because the switch
/// and the pause are changed from the tray while this runs and a cached copy
/// would keep recording after being told to stop. It is one small file read a
/// minute against the alternative of ignoring an instruction to stop, which is
/// not a trade worth making.
///
/// Errors are swallowed on purpose. A monitor that will not capture, a disk
/// that will not take the frame, a locked screen: none of them are worth
/// killing the widget over, and the tray already reports what capture is doing.
fn start_capture(app: AppHandle) {
    std::thread::spawn(move || {
        let Some(root) = capture_root(&app) else {
            return;
        };

        let source = match capture::Screens::new() {
            Ok(source) => source,
            Err(_) => return,
        };

        // A crash or a forced quit leaves loose frames exactly where they
        // were written, since nothing here is buffered only in memory. This
        // is what turns them into their zip on the next launch, so a stretch
        // never sits unarchived just because it happened to be running when
        // the app last closed. The bucket the clock is inside right now is
        // deliberately skipped: it may still be genuinely open, and it is
        // this loop, not a one-off sweep, that owns closing it.
        let now = chrono::Local::now();
        let today = now.format("%Y-%m-%d").to_string();
        let now_bucket = capture::bucket_start(&now.format("%H%M").to_string());
        let _ = capture::recover(&root, &today, &now_bucket);

        // A gigabyte, well above one bucket, so capture stops with room to
        // spare rather than at the point the disk is already unusable. Nothing
        // is ever deleted to make room; refusing to be the process that fills
        // a disk is a different promise from pruning, and only the first is
        // made here.
        let mut recorder = capture::Recorder::new(source, root, 1_024 * 1_024 * 1_024);
        let mut was_recording = false;

        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));

            let Ok(config) = state::Config::load(&app) else {
                continue;
            };

            let allowed = if config.capture_enabled && !paused_now(&config) {
                window_kinds(&app)
            } else {
                CaptureKinds::default()
            };
            let recording = allowed.any();

            let now = chrono::Local::now();
            let stamp = (
                now.format("%Y-%m-%d").to_string(),
                now.format("%H%M").to_string(),
            );
            let _ = recorder.tick((&stamp.0, &stamp.1), allowed.activity, allowed.screen);

            // Tell the page only when it changes, so the indicator can appear
            // and disappear without the widget polling for it.
            if recording != was_recording {
                was_recording = recording;
                let _ = app.emit("capture", json!({ "recording": recording }));
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Lamp::default())
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
            set_lamp,
            capture_status,
            set_capture,
            pause_capture,
            resume_capture,
            open_capture_folder,
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
            // Starts disabled: until the page has drawn a day there is no
            // reading, and defaulting to "hideable" would let a red lamp be put
            // away in the second before the first poll lands.
            let hide = MenuItem::with_id(app, "hide", "Hide", false, None::<&str>)?;

            // Capture is operated from here rather than only from the widget,
            // because the moment it most needs stopping is a screenshare, and
            // that is exactly when the widget is either hidden or the last
            // thing anyone wants to go clicking around in on a shared screen.
            let capturing = state::Config::load(app.handle())
                .map(|c| c.capture_enabled)
                .unwrap_or(false);
            let toggle = MenuItem::with_id(
                app,
                "capture",
                if capturing {
                    "Stop capturing"
                } else {
                    "Start capturing"
                },
                true,
                None::<&str>,
            )?;
            let pause = MenuItem::with_id(
                app,
                "pause-capture",
                "Pause capture for an hour",
                capturing,
                None::<&str>,
            )?;
            // Reachable even when capture is off: the archive outlives the
            // switch, and looking at what was kept is not the same act as
            // keeping more.
            let folder = MenuItem::with_id(
                app,
                "capture-folder",
                "Open capture folder",
                true,
                None::<&str>,
            )?;
            let menu = Menu::with_items(app, &[&show, &hide, &toggle, &pause, &folder, &quit])?;

            if let Ok(mut slot) = app.state::<Lamp>().capture_menu.lock() {
                *slot = Some((toggle.clone(), pause.clone()));
            }

            if let Ok(mut slot) = app.state::<Lamp>().hide.lock() {
                *slot = Some(hide.clone());
            }

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
                    "capture" => {
                        let on = state::Config::load(app)
                            .map(|c| c.capture_enabled)
                            .unwrap_or(false);
                        let _ = set_capture(app.clone(), !on);
                        refresh_capture_menu(app);
                    }
                    "pause-capture" => {
                        let paused = state::Config::load(app)
                            .map(|c| paused_now(&c))
                            .unwrap_or(false);

                        let _ = if paused {
                            resume_capture(app.clone())
                        } else {
                            pause_capture(app.clone(), Some(60), None)
                        };
                        refresh_capture_menu(app);
                    }
                    "capture-folder" => {
                        let _ = open_capture_folder(app.clone());
                    }
                    "hide" => {
                        // Checked again here rather than trusted from the
                        // disabled flag: the state can turn red between the
                        // menu opening and the click landing on it.
                        let red = app
                            .state::<Lamp>()
                            .state
                            .lock()
                            .map(|s| *s == "call")
                            .unwrap_or(true);

                        if !red {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.hide();
                            }
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let _ = place_window(window.clone(), false, None);

                // DISABLED, 25 Aug 2026 — not removed, deliberately paused.
                //
                // This held the front of the topmost band every 2 seconds,
                // because another always-on-top app (Claude's desktop app is
                // one) lands in the same band and sits above us the moment it
                // is activated. It also fought a screenshot tool and, until
                // the GUI_INMENUMODE check landed in `raise`, the tray icon's
                // own right-click menu.
                //
                // Off for a while to see whether it is still earning its
                // keep now that the menu case is fixed, or whether the
                // screenshot friction outweighs the Claude-Desktop case it
                // was built for. `raise` and its menu check are untouched
                // below; re-enable by uncommenting the spawn.
                let _ = window;
                // std::thread::spawn(move || loop {
                //     std::thread::sleep(std::time::Duration::from_secs(2));
                //     let _ = placement::raise(&window);
                // });

                start_capture(app.handle().clone());
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
