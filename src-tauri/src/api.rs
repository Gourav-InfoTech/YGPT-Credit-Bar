use crate::models::{Notification, Org, Usage};
use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const BASE: &str = "https://api.yourgpt.ai";

/// The YourGPT dashboard auto-injects this into every POST body via its `network/index.ts` wrapper.
/// The backend validators require it. "1" is the chatbot app.
const APP_ID: &str = "1";

#[derive(Clone)]
pub struct ApiClient {
    http: Client,
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct PlanDetailResult {
    pub plan_name: Option<String>,
    pub subscription_status: Option<String>,
    pub current_period_start: Option<i64>,
    pub current_period_end: Option<i64>,
    /// Unix seconds of when the trial ends. Only Some when the org is on a trial.
    /// Sourced from `subscriptionData.trail_plan.expiry_date` (note: backend's literal
    /// spelling is "trail_plan", not "trial_plan").
    pub trial_expiry: Option<i64>,
    pub usage: Usage,
}

impl ApiClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent(concat!("YGPTCreditBar/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client");
        Self { http }
    }

    async fn post(&self, token: &str, route: &str, mut body: Value) -> Result<Value> {
        // Inject app_id like the dashboard does, otherwise backend validators reject the request.
        if let Some(obj) = body.as_object_mut() {
            obj.entry("app_id".to_string()).or_insert(Value::String(APP_ID.into()));
        }

        let url = format!("{BASE}{route}");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {route}"))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            // Try to extract message from envelope, else return raw status
            if let Ok(env) = serde_json::from_str::<ApiEnvelope>(&text) {
                if let Some(msg) = env.message {
                    return Err(anyhow!("API {status}: {msg}"));
                }
            }
            return Err(anyhow!("API {status}: {text}"));
        }

        let env: ApiEnvelope =
            serde_json::from_str(&text).context("decode YourGPT response envelope")?;

        if let Some(kind) = &env.kind {
            if kind != "RXSUCCESS" {
                let msg = env.message.unwrap_or_else(|| "API returned error".into());
                return Err(anyhow!(msg));
            }
        }
        Ok(env.data.unwrap_or(Value::Null))
    }

    /// POST /api/v1/getMyOrganizations
    pub async fn list_orgs(&self, token: &str) -> Result<Vec<Org>> {
        let data = self
            .post(token, "/api/v1/getMyOrganizations", json!({}))
            .await?;

        // Response is a list, items may have varying field names — try a few.
        let arr = match data {
            Value::Array(a) => a,
            Value::Object(ref m) => {
                if let Some(Value::Array(a)) = m.get("organizations") {
                    a.clone()
                } else if let Some(Value::Array(a)) = m.get("data") {
                    a.clone()
                } else {
                    return Err(anyhow!("unexpected getMyOrganizations response shape"));
                }
            }
            _ => return Err(anyhow!("getMyOrganizations returned non-array data")),
        };

        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            let id = item
                .get("organization_id")
                .or_else(|| item.get("id"))
                .or_else(|| item.get("organization_uid"))
                .or_else(|| item.get("uid"))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                });
            let name = item
                .get("organization_name")
                .or_else(|| item.get("name"))
                .or_else(|| item.get("title"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    id.clone().unwrap_or_else(|| "Unnamed".to_string())
                });
            if let Some(id) = id {
                out.push(Org { id, name });
            }
        }
        Ok(out)
    }

    /// POST /api/v1/getOrgPlanDetail
    pub async fn get_plan_detail(
        &self,
        token: &str,
        organization_id: &str,
    ) -> Result<PlanDetailResult> {
        let data = self
            .post(
                token,
                "/api/v1/getOrgPlanDetail",
                json!({ "organization_id": organization_id }),
            )
            .await?;

        // The response shape varies. Be defensive: locate usage, plan, subscription wherever they live.
        let root = match data {
            Value::Object(m) => m,
            other => return Err(anyhow!("unexpected plan detail shape: {other:?}")),
        };

        // usage can live at root.usage OR root.activePlanDetails.usage etc. Probe a few paths.
        let usage_val = root
            .get("usage")
            .cloned()
            .or_else(|| {
                root.get("activePlanDetails")
                    .and_then(|v| v.get("usage"))
                    .cloned()
            })
            .or_else(|| {
                root.get("plan_detail")
                    .and_then(|v| v.get("usage"))
                    .cloned()
            })
            .unwrap_or(Value::Null);

        let usage: Usage = serde_json::from_value(usage_val).unwrap_or_default();

        // plan_name: top-level `plan` is a string like "chatbot_professional_monthly". Map to a
        // human-readable display name. Mirror the dashboard's logic in usePlan.tsx.
        let plan_key = root.get("plan").and_then(|v| v.as_str()).map(|s| s.to_string());
        let plan_name = plan_key
            .as_deref()
            .map(plan_key_to_display_name)
            .or_else(|| first_string(&root, &["plan_name", "plan_title", "name"]));

        // subscription status — top-level `status` field
        let subscription_status = first_string(&root, &["status", "subscription_status"]);

        // current_period_start / current_period_end live on subscriptionData. The dashboard reads
        // activePlanDetails.subscriptionData.current_period_end.
        let current_period_start = first_i64(&root, &["current_period_start"])
            .or_else(|| {
                root.get("subscriptionData")
                    .and_then(|s| first_i64_obj(s, &["current_period_start"]))
            })
            .or_else(|| {
                root.get("next_billing_cycle")
                    .and_then(|n| first_i64_obj(n, &["period_start"]))
            });

        let current_period_end = first_i64(&root, &["current_period_end"])
            .or_else(|| {
                root.get("subscriptionData")
                    .and_then(|s| first_i64_obj(s, &["current_period_end"]))
            })
            .or_else(|| {
                root.get("next_billing_cycle")
                    .and_then(|n| first_i64_obj(n, &["period_end"]))
            });

        // Trial expiry — `subscriptionData.trail_plan.expiry_date` (yes, the server's spelling
        // is "trail_plan"). Accept either a Unix-seconds number or an ISO 8601 string.
        let trial_expiry = root
            .get("subscriptionData")
            .and_then(|s| s.get("trail_plan"))
            .filter(|v| !v.is_null())
            .and_then(|tp| tp.get("expiry_date"))
            .and_then(parse_unix_seconds);

        Ok(PlanDetailResult {
            plan_name,
            subscription_status,
            current_period_start,
            current_period_end,
            trial_expiry,
            usage,
        })
    }

    /// POST /chatbot/v1/getMyNotification
    ///
    /// Returns the full notification feed (no pagination). The chatbot subtree uses the same
    /// `api.yourgpt.ai` host as the rest of the API and the same JWT auth, so the existing
    /// `post()` helper Just Works.
    pub async fn get_notifications(&self, token: &str) -> Result<Vec<Notification>> {
        let data = self
            .post(token, "/chatbot/v1/getMyNotification", json!({}))
            .await?;

        // The envelope's `data` may be the array directly, or wrapped in `{ notifications: [...] }`
        // or `{ data: [...] }`. Match the same defensive shape we use for orgs.
        let arr = match data {
            Value::Array(a) => a,
            Value::Object(ref m) => {
                if let Some(Value::Array(a)) = m.get("notifications") {
                    a.clone()
                } else if let Some(Value::Array(a)) = m.get("data") {
                    a.clone()
                } else {
                    return Err(anyhow!("unexpected getMyNotification response shape"));
                }
            }
            Value::Null => Vec::new(),
            _ => return Err(anyhow!("getMyNotification returned non-array data")),
        };

        // Deserialize per-item so a single malformed row doesn't tank the whole feed.
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            match serde_json::from_value::<Notification>(item.clone()) {
                Ok(n) => out.push(n),
                Err(e) => log::warn!("skipping malformed notification: {e}"),
            }
        }
        Ok(out)
    }

}

/// Maps a YourGPT plan_key (e.g. "chatbot_professional_monthly") to a display title.
/// Mirrors the .includes() chain in yourgpt-chatbot's usePlan.tsx.
fn plan_key_to_display_name(plan: &str) -> String {
    let p = plan.to_lowercase();
    let yearly = p.contains("yearly") || p.contains("annual");
    let suffix = if yearly { " (Yearly)" } else { "" };
    let base = if p == "chatbot_trial" || p.contains("trial") {
        "Trial"
    } else if p.contains("agency_starter") {
        "Agency Starter"
    } else if p.contains("agency_growth") {
        "Agency Growth"
    } else if p.contains("starter") {
        "Starter"
    } else if p.contains("essential") {
        "Essential"
    } else if p.contains("professional") {
        "Professional"
    } else if p.contains("elite") {
        "Elite"
    } else if p.contains("prime_standard") {
        "Prime Standard"
    } else if p.contains("prime_exclusive") {
        "Prime Exclusive"
    } else if p.contains("growth") {
        "Growth"
    } else if p.contains("advanced") {
        "Advanced"
    } else if p.contains("agency") {
        "Agency"
    } else {
        return plan.to_string(); // unknown — return raw key
    };
    if base == "Trial" { base.to_string() } else { format!("{base}{suffix}") }
}

fn first_string(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = obj.get(*k).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
    }
    None
}

/// Coerces a JSON value to Unix seconds. Accepts either a number (timestamp seconds OR
/// milliseconds — auto-detected by magnitude) or an ISO-8601 string. Returns None on
/// anything else.
fn parse_unix_seconds(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        // Treat anything that looks like a JS-style millisecond timestamp as ms.
        return Some(if n > 1_000_000_000_000 { n / 1000 } else { n });
    }
    if let Some(s) = v.as_str() {
        if let Ok(n) = s.parse::<i64>() {
            return Some(if n > 1_000_000_000_000 { n / 1000 } else { n });
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Some(dt.timestamp());
        }
    }
    None
}

fn first_i64(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<i64> {
    for k in keys {
        if let Some(v) = obj.get(*k) {
            if let Some(n) = v.as_i64() {
                return Some(n);
            }
            if let Some(s) = v.as_str().and_then(|s| s.parse::<i64>().ok()) {
                return Some(s);
            }
        }
    }
    None
}

fn first_i64_obj(v: &Value, keys: &[&str]) -> Option<i64> {
    let obj = v.as_object()?;
    for k in keys {
        if let Some(val) = obj.get(*k) {
            if let Some(n) = val.as_i64() {
                return Some(n);
            }
            if let Some(s) = val.as_str().and_then(|s| s.parse::<i64>().ok()) {
                return Some(s);
            }
        }
    }
    None
}
