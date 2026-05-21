use crate::api::ApiClient;
use crate::keychain;
use crate::models::{Org, Severity, Snapshot};
use crate::poller;
use crate::state::{AppState, UserSettings};
use crate::tray;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

#[derive(Serialize)]
pub struct PlanStateResp {
    pub has_token: bool,
    pub snapshot: Option<Snapshot>,
    pub last_error: Option<String>,
    pub has_org: bool,
    pub organization_id: Option<String>,
}

#[derive(Serialize)]
pub struct SettingsResp {
    pub has_token: bool,
    pub organization_id: Option<String>,
    pub organization_name: Option<String>,
    pub interval_secs: u64,
}

#[derive(Serialize)]
pub struct OrgsResp {
    pub orgs: Vec<Org>,
}

#[tauri::command]
pub fn get_plan_state(state: State<'_, Arc<AppState>>) -> PlanStateResp {
    let has_token = state.has_token();
    let snapshot = state.snapshot.read().unwrap().clone();
    let last_error = state.last_error.read().unwrap().clone();
    let settings = state.settings.read().unwrap();
    PlanStateResp {
        has_token,
        snapshot,
        last_error,
        has_org: settings.organization_id.is_some(),
        organization_id: settings.organization_id.clone(),
    }
}

#[tauri::command]
pub fn get_settings(state: State<'_, Arc<AppState>>) -> SettingsResp {
    let settings = state.settings.read().unwrap().clone();
    SettingsResp {
        has_token: state.has_token(),
        organization_id: settings.organization_id,
        organization_name: settings.organization_name,
        interval_secs: settings.interval_secs,
    }
}

#[tauri::command]
pub async fn list_orgs(
    token: Option<String>,
    state: State<'_, Arc<AppState>>,
    api: State<'_, ApiClient>,
) -> Result<OrgsResp, String> {
    let t = match token {
        Some(t) if !t.trim().is_empty() => t,
        _ => state
            .token()
            .ok_or_else(|| "No token saved. Open Settings to connect your account.".to_string())?,
    };
    api.list_orgs(&t)
        .await
        .map(|orgs| OrgsResp { orgs })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn switch_org(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    api: State<'_, ApiClient>,
    organization_id: String,
    organization_name: String,
) -> Result<(), String> {
    log::info!("switch_org called: id={organization_id}, name={organization_name}");

    let mut updated = state.settings.read().unwrap().clone();
    updated.organization_id = Some(organization_id.clone());
    updated.organization_name = Some(organization_name);
    updated.save().map_err(|e| e.to_string())?;
    *state.settings.write().unwrap() = updated;
    // Clear stale data so the popover shows a clean loading state until the new fetch lands.
    *state.snapshot.write().unwrap() = None;
    *state.last_error.write().unwrap() = None;
    *state.last_notified_pct.write().unwrap() = 0.0;

    let api_clone = api.inner().clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        log::info!("switch_org: spawning fetch_once for org={organization_id}");
        crate::poller::fetch_once(&app_clone, &api_clone).await;
        log::info!("switch_org: fetch_once done for org={organization_id}");
    });
    Ok(())
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    api: State<'_, ApiClient>,
    token: Option<String>,
    organization_id: String,
    organization_name: String,
    interval_secs: u64,
) -> Result<(), String> {
    if let Some(t) = token.as_deref() {
        let t = t.trim();
        if !t.is_empty() {
            keychain::save_token(t).map_err(|e| e.to_string())?;
        }
    }

    let new_settings = UserSettings {
        organization_id: Some(organization_id),
        organization_name: Some(organization_name),
        interval_secs: interval_secs.clamp(15, 300),
    };
    new_settings.save().map_err(|e| e.to_string())?;
    *state.settings.write().unwrap() = new_settings;
    // Reset throttle so a fresh setup doesn't reuse stale notification suppression.
    *state.last_notified_pct.write().unwrap() = 0.0;

    // Trigger an immediate fetch so the popover refreshes right away.
    let api_clone = api.inner().clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        poller::fetch_once(&app_clone, &api_clone).await;
    });

    Ok(())
}

#[tauri::command]
pub fn clear_account(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    keychain::delete_token().map_err(|e| e.to_string())?;
    keychain::delete_settings().map_err(|e| e.to_string())?;
    *state.settings.write().unwrap() = UserSettings::defaults();
    *state.snapshot.write().unwrap() = None;
    *state.last_error.write().unwrap() = None;
    *state.last_notified_pct.write().unwrap() = 0.0;

    // Reset the tray icon to the idle (gray) state and clear the percentage title.
    if let Some(tray_icon) = app.tray_by_id("main") {
        let _ = tray_icon.set_icon(Some(tray::icon_for(Severity::Idle)));
        let _ = tray_icon.set_icon_as_template(false);
        let _ = tray_icon.set_title(None::<&str>);
    }

    // Tell the popover to re-render with the empty state.
    let _ = app.emit("snapshot-updated", ());
    Ok(())
}

#[tauri::command]
pub async fn refresh_now(app: AppHandle, api: State<'_, ApiClient>) -> Result<(), String> {
    poller::fetch_once(&app, &api).await;
    Ok(())
}

#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.center();
        return Ok(());
    }
    // Fall back: create it from scratch (shouldn't be needed since it's declared in tauri.conf.json).
    let win = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
        .title("YGPTCreditBar Settings")
        .inner_size(480.0, 460.0)
        .resizable(false)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
