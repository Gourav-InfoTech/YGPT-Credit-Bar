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

/// Whitelist of notification `type` values we surface in the menu bar. This app is for
/// billing/credit/plan awareness — project-scoped events (CHAT_ASSIGNED, ESCALATED_TO_HUMAN,
/// etc.) belong in the dashboard, not here.
const BILLING_NOTIFICATION_TYPES: &[&str] = &[
    "CREDIT_LIMIT_50_PERCENT_REACHED",
    "CREDIT_LIMIT_75_PERCENT_REACHED",
    "CREDIT_EXHAUSTED",
    "PAST_DUE_SUBSCRIPTION",
    "VOICE_CREDIT_LIMIT_75_PERCENT_REACHED",
    "VOICE_CREDIT_EXHAUSTED",
];

fn is_billing_type(t: &str) -> bool {
    BILLING_NOTIFICATION_TYPES.contains(&t)
}

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
        update_tray(app, Severity::Idle, String::new());
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
            // Empty string (not None) — tray-icon's macOS impl ignores None updates,
            // so we always pass Some(...) and use "" to clear.
            let title = if worst_pct >= 70.0 {
                format!("{}%", worst_pct.round() as i64)
            } else {
                String::new()
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

    // Notifications run on the same cycle. Independent of plan-detail success/failure so a
    // notif-API hiccup doesn't poison quota rendering and vice versa.
    fetch_notifications_once(app, api, &token, &state).await;
}

/// Pulls the latest notification feed and fires a native macOS banner for any new
/// **billing/credit/plan** notification we haven't already announced this session.
///
/// No UI is involved — this is fire-and-forget. We don't store the list in state and
/// there's no in-app feed. The user sees these alerts the same way they see any other
/// macOS notification (Notification Center, banner, etc.).
async fn fetch_notifications_once(
    app: &AppHandle,
    api: &ApiClient,
    token: &str,
    state: &Arc<AppState>,
) {
    let list = match api.get_notifications(token).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("notification fetch failed: {e:#}");
            return;
        }
    };

    // Filter to billing/credit/plan types AND items we haven't already announced.
    let new_items: Vec<_> = list
        .into_iter()
        .filter(|n| {
            n.notification_data
                .as_ref()
                .and_then(|d| d.kind.as_deref())
                .map(is_billing_type)
                .unwrap_or(false)
        })
        .filter(|n| n.is_unread())
        .filter(|n| {
            !state
                .announced_notification_ids
                .read()
                .unwrap()
                .contains(&n.id)
        })
        .collect();

    for n in &new_items {
        let title = n.title.clone().unwrap_or_else(|| "YourGPT".to_string());
        // Keep banner body short — macOS truncates aggressively anyway.
        let mut body = n.body.clone();
        if body.len() > 200 {
            body.truncate(200);
            body.push('…');
        }
        if let Err(e) = app
            .notification()
            .builder()
            .title(&title)
            .body(&body)
            .show()
        {
            log::warn!("native notification show failed: {e}");
        }
        state
            .announced_notification_ids
            .write()
            .unwrap()
            .insert(n.id);
    }
}

fn update_tray(app: &AppHandle, severity: Severity, title: String) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(tray::icon_for(severity)));
        let _ = tray.set_icon_as_template(false);
        // Always pass Some(_): tray-icon on macOS treats None as no-op, so the
        // previous "73%" sticks even after we reset; an empty string clears it.
        let _ = tray.set_title(Some(title.as_str()));
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
