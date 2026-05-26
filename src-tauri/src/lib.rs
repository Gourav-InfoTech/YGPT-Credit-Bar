mod api;
mod commands;
mod keychain;
mod models;
mod poller;
mod state;
mod tray;

use crate::api::ApiClient;
use crate::models::Severity;
use crate::state::AppState;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_positioner::{Position, WindowExt};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_positioner::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_plan_state,
            commands::get_settings,
            commands::list_orgs,
            commands::switch_org,
            commands::save_settings,
            commands::clear_account,
            commands::refresh_now,
            commands::open_external,
            commands::open_settings_window,
            commands::test_notification,
            commands::quit_app,
        ])
        .setup(|app| {
            log::info!("YGPTCreditBar starting up");

            // Hide the dock icon — this is a menu bar app, not a regular app.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Shared state
            let state = Arc::new(AppState::new());
            app.manage(state.clone());

            // Shared HTTP client
            let api = ApiClient::new();
            app.manage(api.clone());

            // Build the tray menu
            let open_item = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
            let settings_item =
                MenuItem::with_id(app, "settings", "Settings…", true, Some("Cmd+,"))?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, Some("Cmd+Q"))?;

            let menu = Menu::with_items(
                app,
                &[&open_item, &settings_item, &separator, &quit_item],
            )?;

            // Initial icon: idle (gray)
            let initial_icon = tray::icon_for(Severity::Idle);

            log::info!("Building tray icon");

            let _tray = TrayIconBuilder::with_id("main")
                .icon(initial_icon)
                .icon_as_template(false)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("YGPTCreditBar")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        show_popover(app);
                    }
                    "settings" => {
                        let _ = commands::open_settings_window(app.clone());
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    let app = tray.app_handle();
                    // Let the positioner plugin know where the tray is so we can dock the popover under it.
                    tauri_plugin_positioner::on_tray_event(app, &event);

                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        position,
                        ..
                    } = event
                    {
                        toggle_popover_at_cursor(app, position);
                    }
                })
                .build(app)?;

            log::info!("Tray icon built. Spawning poller…");

            // Spawn the background poller
            poller::spawn(app.handle().clone(), api.clone());

            // Apply native NSVisualEffectView vibrancy so the popover gets a real macOS glass look
            // instead of a fake CSS backdrop-filter (which only blurs the WebView's own content).
            #[cfg(target_os = "macos")]
            {
                use window_vibrancy::{
                    apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
                };
                if let Some(popover_window) = app.get_webview_window("popover") {
                    if let Err(e) = apply_vibrancy(
                        &popover_window,
                        NSVisualEffectMaterial::Popover,
                        Some(NSVisualEffectState::Active),
                        Some(14.0),
                    ) {
                        log::warn!("apply_vibrancy(popover) failed: {e:?}");
                    }
                }
            }

            log::info!("Setup complete — app is now running in menu bar");

            // Make sure all windows start hidden
            if let Some(win) = app.get_webview_window("popover") {
                let _ = win.hide();
            }
            if let Some(win) = app.get_webview_window("settings") {
                let _ = win.hide();
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // Auto-hide popover when it loses focus, mirroring native menu bar behavior.
            if window.label() == "popover" {
                if let tauri::WindowEvent::Focused(false) = event {
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn show_popover(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("popover") {
        let _ = win.move_window(Position::TrayBottomCenter);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Logical popover width — must match the value declared in `tauri.conf.json`.
const POPOVER_LOGICAL_WIDTH: f64 = 360.0;
/// Pixels below the menu bar where the popover top-edge should sit.
const MENU_BAR_GAP: f64 = 28.0;

/// Position the popover under the cursor (which is at the tray icon during a click).
///
/// Multi-monitor + HiDPI is messy in Tauri 2:
///   - `tauri-plugin-positioner`'s TrayCenter is broken on external monitors
///     (https://github.com/tauri-apps/plugins-workspace/issues/724)
///   - Physical-pixel coords across monitors with different scale factors don't form a
///     consistent grid (https://github.com/tauri-apps/tauri/issues/7890)
///
/// Fix: work entirely in *logical* coordinates, scoped to the monitor that contains the
/// cursor. We find that monitor by comparing physical bounds (which the API gives us), then
/// convert everything to logical using *that* monitor's scale factor, and set the popover
/// position with LogicalPosition.
fn toggle_popover_at_cursor(app: &tauri::AppHandle, cursor: tauri::PhysicalPosition<f64>) {
    let Some(win) = app.get_webview_window("popover") else { return; };

    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        return;
    }

    // Locate the monitor containing the cursor. Bounds are in physical pixels.
    let monitor = match win.available_monitors().ok().and_then(|mons| {
        mons.into_iter().find(|m| {
            let p = m.position();
            let s = m.size();
            let mx0 = p.x as f64;
            let my0 = p.y as f64;
            let mx1 = mx0 + s.width as f64;
            let my1 = my0 + s.height as f64;
            cursor.x >= mx0 && cursor.x < mx1 && cursor.y >= my0 && cursor.y < my1
        })
    }) {
        Some(m) => m,
        None => {
            // Fallback to primary monitor — better than placing the window off-screen.
            log::warn!(
                "no monitor contains cursor=({:.0},{:.0}); using primary",
                cursor.x, cursor.y
            );
            match win.primary_monitor().ok().flatten() {
                Some(m) => m,
                None => return,
            }
        }
    };

    let scale = monitor.scale_factor();
    let mp = monitor.position();
    let ms = monitor.size();

    // Convert the monitor's bounds (which Tauri reports in physical pixels) into the global
    // logical coordinate space by dividing by *that monitor's* scale factor.
    let mon_logical_x = mp.x as f64 / scale;
    let mon_logical_y = mp.y as f64 / scale;
    let mon_logical_w = ms.width as f64 / scale;

    // Cursor position within the active monitor, in logical pixels.
    let cursor_logical_in_mon = (cursor.x - mp.x as f64) / scale;

    // Center popover horizontally on the cursor, then clamp inside the active monitor.
    let pop_rel_x = cursor_logical_in_mon - (POPOVER_LOGICAL_WIDTH / 2.0);
    let pop_rel_x_clamped =
        pop_rel_x.max(6.0).min(mon_logical_w - POPOVER_LOGICAL_WIDTH - 6.0);

    let pop_global_x = mon_logical_x + pop_rel_x_clamped;
    let pop_global_y = mon_logical_y + MENU_BAR_GAP;

    log::info!(
        "popover -> ({pop_global_x:.0}, {pop_global_y:.0}) logical; scale={scale}; \
         mon_logical=({mon_logical_x:.0},{mon_logical_y:.0},w={mon_logical_w:.0}); \
         cursor_phys=({:.0},{:.0})",
        cursor.x, cursor.y
    );

    let _ = win.set_position(tauri::LogicalPosition::new(pop_global_x, pop_global_y));
    let _ = win.show();
    let _ = win.set_focus();
}
