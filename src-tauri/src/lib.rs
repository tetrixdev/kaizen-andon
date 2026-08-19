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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            place_window,
            load_config,
            save_config
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
