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

    let (token, selected_org_id, selected_org_name) = {
        let settings = state.settings.read().unwrap();
        let token = state.token();
        (token, settings.organization_id.clone(), settings.organization_name.clone())
    };

    let (Some(token), Some(selected_org_id)) = (token, selected_org_id) else {
        // Nothing to fetch yet — leave snapshot as-is and tray as Idle.
        update_tray(app, Severity::Idle, String::new());
        return;
    };

    log::info!(
        "fetch_once: selected_org_id={selected_org_id}, token_len={}",
        token.len()
    );
    let _ = app.emit("fetch-started", ());

    // Fetch the user's full org list so we can run threshold checks across ALL of them,
    // not just the currently-selected one. If listing fails we still process the selected
    // org so the popover continues to work.
    let mut orgs = match api.list_orgs(&token).await {
        Ok(list) if !list.is_empty() => list
            .into_iter()
            .map(|o| (o.id, o.name))
            .collect::<Vec<_>>(),
        Ok(_) | Err(_) => {
            // Fall back to the selected org only — better than nothing.
            vec![(
                selected_org_id.clone(),
                selected_org_name.clone().unwrap_or_else(|| "Organization".into()),
            )]
        }
    };

    // Make sure the selected org is in the list (it always should be, but defensive).
    if !orgs.iter().any(|(id, _)| id == &selected_org_id) {
        orgs.push((
            selected_org_id.clone(),
            selected_org_name.clone().unwrap_or_else(|| "Organization".into()),
        ));
    }

    // Iterate every org. For each: fetch plan detail, run threshold checks (per-org dedup),
    // and if it's the SELECTED org, write its snapshot to state + update the tray + popover.
    for (org_id, org_name) in &orgs {
        let plan = match api.get_plan_detail(&token, org_id).await {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("{e:#}");
                log::warn!("plan detail fetch failed for org={org_id}: {msg}");
                if org_id == &selected_org_id {
                    *state.last_error.write().unwrap() = Some(msg);
                    let _ = app.emit("snapshot-updated", ());
                }
                continue;
            }
        };

        let (severity, worst_pct) = plan.usage.worst_severity();
        let snapshot = Snapshot {
            org_name: org_name.clone(),
            plan_name: plan.plan_name.clone(),
            subscription_status: plan.subscription_status.clone(),
            current_period_start: plan.current_period_start,
            current_period_end: plan.current_period_end,
            trial_expiry: plan.trial_expiry,
            usage: plan.usage,
            fetched_at_ms: chrono::Utc::now().timestamp_millis(),
            severity,
            worst_pct,
        };

        log::info!(
            "fetch_once OK: org={}({}), plan_name={:?}, status={:?}, buckets=[{}], worst_pct={worst_pct:.1}",
            snapshot.org_name,
            org_id,
            snapshot.plan_name,
            snapshot.subscription_status,
            bucket_summary(&snapshot.usage),
        );

        // Fire native banners for any newly-crossed thresholds, scoped to this org so we
        // dedup independently per org.
        check_thresholds(app, &state, org_id, &snapshot);
        check_subscription_status(app, &state, org_id, &snapshot);

        // Selected org also drives the popover + tray.
        if org_id == &selected_org_id {
            *state.snapshot.write().unwrap() = Some(snapshot.clone());
            *state.last_error.write().unwrap() = None;

            let tray_title = if worst_pct >= 70.0 {
                format!("{}%", worst_pct.round() as i64)
            } else {
                String::new()
            };
            update_tray(app, severity, tray_title);
            let _ = app.emit("snapshot-updated", ());
        }
    }

    // Server-side billing notifications (account-wide, not per-org).
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

/// Fires a native macOS banner for the highest threshold a bucket has newly crossed.
/// At most ONE banner per (org, bucket) per poll cycle — so a bucket that's already at 100%
/// on cold start doesn't fire 80% + 95% + 100% banners simultaneously. As the bucket
/// climbs across new (higher) thresholds later, each one fires independently.
///
/// Dedup is per-(org_id, bucket_label, threshold) and in-memory only, so:
///   - the same (org, bucket, threshold) doesn't re-fire across poll cycles
///   - different orgs fire independently (Acme + Beta both at 100% Credits → two banners)
///   - cold-restarting the app re-fires for genuinely unacknowledged thresholds
fn check_thresholds(app: &AppHandle, state: &AppState, org_id: &str, snap: &Snapshot) {
    for (label, bucket_opt) in named_buckets(&snap.usage) {
        let Some(bucket) = bucket_opt else { continue };
        let pct = bucket.percent();

        // Walk thresholds high → low. The first one we haven't fired yet is what we banner.
        // Any *lower* thresholds get marked as fired implicitly so we never retroactively
        // fire them (e.g. bucket drops to 70 then climbs back to 85 — we already alerted at
        // 85+ once, no point re-alerting at 80).
        for &t in THRESHOLDS.iter().rev() {
            if pct < t {
                continue;
            }
            let key = (org_id.to_string(), label.to_string(), t as u8);
            if state.fired_thresholds.read().unwrap().contains(&key) {
                // Highest crossed threshold is already-fired — and any lower ones we'd
                // consider must have been already-fired or implicitly-marked too. Done.
                break;
            }

            // Record THIS threshold + all lower ones in one shot so concurrent polls
            // and future "rebound" scenarios can't double-fire.
            {
                let mut fired = state.fired_thresholds.write().unwrap();
                fired.insert(key);
                for &lower in THRESHOLDS.iter().filter(|&&x| x < t) {
                    fired.insert((org_id.to_string(), label.to_string(), lower as u8));
                }
            }

            let body = banner_body(label, t as u8);

            log::info!(
                "firing banner: org={} bucket={} t={} pct={pct:.1}",
                snap.org_name, label, t
            );
            if let Err(e) = app
                .notification()
                .builder()
                .title(&snap.org_name)
                .body(&body)
                .show()
            {
                log::warn!("native notification show failed: {e}");
            }
            break;
        }
    }
}

/// (display_label, &Option<Bucket>) tuples for all 8 buckets in a fixed order. Single source
/// of truth used by both `check_thresholds` and `bucket_summary` so they can't drift.
/// Fires a synthesised native banner when an org's subscription transitions into a
/// payment-required state (`past_due`). Reads the polled `subscription_status` directly
/// from `getOrgPlanDetail` so we react faster than waiting for the server's
/// PAST_DUE_SUBSCRIPTION notification. When the status recovers, we clear the dedup key
/// so a future past-due event can re-fire.
fn check_subscription_status(app: &AppHandle, state: &AppState, org_id: &str, snap: &Snapshot) {
    let status = snap.subscription_status.as_deref().unwrap_or("");
    // Reuse the existing dedup set with a synthetic threshold key. 0 sorts before any real
    // bucket-threshold (which start at 80), keeping the namespace clean.
    let key = (org_id.to_string(), "__subscription_past_due".to_string(), 0u8);

    let is_past_due = status == "past_due";
    let already_fired = state.fired_thresholds.read().unwrap().contains(&key);

    if is_past_due && !already_fired {
        log::info!("firing past-due banner: org={}", snap.org_name);
        if let Err(e) = app
            .notification()
            .builder()
            .title(&snap.org_name)
            .body("Subscription past due. Update payment to keep service running.")
            .show()
        {
            log::warn!("native notification show failed: {e}");
        }
        state.fired_thresholds.write().unwrap().insert(key);
    } else if !is_past_due && already_fired {
        // Recovered — clear the dedup so a future past_due transition can fire again.
        state.fired_thresholds.write().unwrap().remove(&key);
    }
}

fn named_buckets(u: &crate::models::Usage) -> [(&'static str, &Option<crate::models::Bucket>); 8] {
    [
        ("Credits", &u.credits),
        ("Voice Credits", &u.voice_credits),
        ("Voice Lite Credits", &u.voice_lite_credits),
        ("Campaign Credits", &u.campaign_credits),
        ("Chatbots", &u.chatbot),
        ("Team Members", &u.members),
        ("Documents", &u.document),
        ("Webpages", &u.webpages),
    ]
}

/// Per-bucket, per-threshold consequence copy.
///
/// Two flavors of bucket need different framing:
///   - Credit-burn (Credits / Voice / Voice Lite / Campaign): running out → feature stops.
///     The banner explains *which feature* stops and prompts a top-up.
///   - Count-cap (Chatbots / Documents / Webpages / Team Members): the limit is a hard
///     ceiling. The banner explains "you can't add more" and points to upgrade.
///
/// Bodies are kept under ~60 chars so the macOS banner's collapsed view doesn't truncate
/// the consequence. The label + "at N%" leads so the body is self-describing in
/// Notification Center where multiple banners for the same org stack together.
fn banner_body(label: &str, threshold: u8) -> String {
    let copy: &str = match (label, threshold) {
        // Credit-burn buckets
        ("Credits", 80) => "Credits at 80%. Top up before your chatbot pauses.",
        ("Credits", 95) => "Credits at 95%. Chatbot will pause soon. Top up.",
        ("Credits", 100) => "Credits at 100%. Your chatbot can't answer messages.",

        ("Voice Credits", 80) => "Voice Credits at 80%. Top up before voice pauses.",
        ("Voice Credits", 95) => "Voice Credits at 95%. Voice features pausing. Top up.",
        ("Voice Credits", 100) => "Voice Credits at 100%. Voice features have stopped.",

        ("Voice Lite Credits", 80) => "Voice Lite at 80%. Top up before voice pauses.",
        ("Voice Lite Credits", 95) => "Voice Lite at 95%. Voice features pausing. Top up.",
        ("Voice Lite Credits", 100) => "Voice Lite at 100%. Voice features have stopped.",

        ("Campaign Credits", 80) => "Campaign Credits at 80%. Top up before campaigns pause.",
        ("Campaign Credits", 95) => "Campaign Credits at 95%. Sends pausing soon. Top up.",
        ("Campaign Credits", 100) => "Campaign Credits at 100%. Campaign sends have stopped.",

        // Count-cap buckets
        ("Chatbots", 80) => "Chatbots at 80%. Approaching chatbot limit. Upgrade to add more.",
        ("Chatbots", 95) => "Chatbots at 95%. Almost at chatbot limit. Upgrade to add more.",
        ("Chatbots", 100) => "Chatbots at 100%. Chatbot limit reached. Upgrade to add more.",

        ("Team Members", 80) => "Team Members at 80%. Approaching seat limit. Upgrade.",
        ("Team Members", 95) => "Team Members at 95%. Almost at seat limit. Upgrade.",
        ("Team Members", 100) => "Team Members at 100%. Seat limit reached. Upgrade.",

        ("Documents", 80) => "Documents at 80%. Approaching document limit. Upgrade.",
        ("Documents", 95) => "Documents at 95%. Almost at document limit. Upgrade.",
        ("Documents", 100) => "Documents at 100%. Document limit reached. Upgrade.",

        ("Webpages", 80) => "Webpages at 80%. Approaching webpage limit. Upgrade.",
        ("Webpages", 95) => "Webpages at 95%. Almost at webpage limit. Upgrade.",
        ("Webpages", 100) => "Webpages at 100%. Webpage limit reached. Upgrade.",

        // Defensive: unknown label or threshold. Should be unreachable in practice.
        _ => return format!("{label} at {threshold}%."),
    };
    copy.to_string()
}

fn bucket_summary(u: &crate::models::Usage) -> String {
    named_buckets(u)
        .iter()
        .filter_map(|(name, opt)| opt.as_ref().map(|b| format!("{name}={}/{}", b.usage, b.limit)))
        .collect::<Vec<_>>()
        .join(", ")
}
