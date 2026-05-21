use crate::api::ApiClient;
use crate::models::{Severity, Snapshot};
use crate::state::AppState;
use crate::tray;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// Notification thresholds — fire once per bucket per threshold crossing per session.
const THRESHOLDS: &[f64] = &[80.0, 95.0, 100.0];

pub fn spawn(app: AppHandle, api: ApiClient) {
    tauri::async_runtime::spawn(async move {
        // First fetch immediately, then settle into the configured interval.
        loop {
            let interval = {
                let state = app.state::<Arc<AppState>>();
                let s = state.settings.read().unwrap();
                Duration::from_secs(s.interval_secs.max(15))
            };
            fetch_once(&app, &api).await;
            tokio::time::sleep(interval).await;
        }
    });
}

pub async fn fetch_once(app: &AppHandle, api: &ApiClient) {
    let state = app.state::<Arc<AppState>>().inner().clone();

    let (token, org_id, org_name) = {
        let settings = state.settings.read().unwrap();
        let token = state.token();
        (token, settings.organization_id.clone(), settings.organization_name.clone())
    };

    let (Some(token), Some(org_id)) = (token, org_id) else {
        // Nothing to fetch yet — leave snapshot as-is and tray as Idle.
        update_tray(app, Severity::Idle, None);
        return;
    };

    log::info!("fetch_once: org_id={org_id}, token_len={}", token.len());
    let _ = app.emit("fetch-started", ());

    match api.get_plan_detail(&token, &org_id).await {
        Ok(plan) => {
            let (severity, worst_pct) = plan.usage.worst_severity();
            let snapshot = Snapshot {
                org_name: org_name.unwrap_or_else(|| "Organization".into()),
                plan_name: plan.plan_name.clone(),
                subscription_status: plan.subscription_status.clone(),
                current_period_start: plan.current_period_start,
                current_period_end: plan.current_period_end,
                usage: plan.usage,
                fetched_at_ms: chrono::Utc::now().timestamp_millis(),
                severity,
                worst_pct,
            };

            log::info!(
                "fetch_once OK: org={}, plan_name={:?}, status={:?}, period_end={:?}, buckets_present=[{}], worst_pct={worst_pct:.1}",
                snapshot.org_name,
                snapshot.plan_name,
                snapshot.subscription_status,
                snapshot.current_period_end,
                bucket_summary(&snapshot.usage),
            );

            check_thresholds(app, &state, &snapshot);

            *state.snapshot.write().unwrap() = Some(snapshot.clone());
            *state.last_error.write().unwrap() = None;

            // Title text on the tray: show worst pct when above the warn threshold.
            let title = if worst_pct >= 70.0 {
                Some(format!("{}%", worst_pct.round() as i64))
            } else {
                None
            };
            update_tray(app, severity, title);
            let _ = app.emit("snapshot-updated", ());
        }
        Err(e) => {
            let msg = format!("{e:#}");
            log::warn!("plan detail fetch failed: {msg}");
            *state.last_error.write().unwrap() = Some(msg);
            let _ = app.emit("snapshot-updated", ());
        }
    }
}

fn update_tray(app: &AppHandle, severity: Severity, title: Option<String>) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(tray::icon_for(severity)));
        let _ = tray.set_icon_as_template(false);
        let _ = tray.set_title(title.as_deref());
    }
}

fn check_thresholds(app: &AppHandle, state: &AppState, snap: &Snapshot) {
    let pct = snap.worst_pct;
    let mut last = state.last_notified_pct.write().unwrap();

    // Fire once when crossing a threshold upward.
    for &t in THRESHOLDS {
        if *last < t && pct >= t {
            let msg = if t >= 100.0 {
                format!("{} usage has reached 100%.", worst_bucket_label(snap))
            } else if t >= 95.0 {
                format!("{} is at {:.0}% — top it up soon.", worst_bucket_label(snap), pct)
            } else {
                format!("{} is at {:.0}%.", worst_bucket_label(snap), pct)
            };

            let _ = app
                .notification()
                .builder()
                .title("YGPTCreditBar")
                .body(&msg)
                .show();
        }
    }
    *last = pct;
}

fn bucket_summary(u: &crate::models::Usage) -> String {
    let pairs = [
        ("credits", &u.credits),
        ("voice_credits", &u.voice_credits),
        ("voice_lite_credits", &u.voice_lite_credits),
        ("campaign_credits", &u.campaign_credits),
        ("chatbot", &u.chatbot),
        ("members", &u.members),
        ("document", &u.document),
        ("webpages", &u.webpages),
    ];
    pairs
        .iter()
        .filter_map(|(name, opt)| opt.as_ref().map(|b| format!("{name}={}/{}", b.usage, b.limit)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn worst_bucket_label(snap: &Snapshot) -> &'static str {
    // Find which bucket is at the worst pct so we can label the notification.
    let buckets = [
        ("Credits", &snap.usage.credits),
        ("Voice Credits", &snap.usage.voice_credits),
        ("Voice Lite Credits", &snap.usage.voice_lite_credits),
        ("Campaign Credits", &snap.usage.campaign_credits),
        ("Chatbots", &snap.usage.chatbot),
        ("Members", &snap.usage.members),
        ("Documents", &snap.usage.document),
        ("Webpages", &snap.usage.webpages),
    ];
    let mut worst = ("Usage", 0.0);
    for (name, b) in buckets {
        if let Some(bucket) = b {
            let p = bucket.percent();
            if p > worst.1 {
                worst = (name, p);
            }
        }
    }
    worst.0
}
