use crate::models::{Org, Usage};
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

        Ok(PlanDetailResult {
            plan_name,
            subscription_status,
            current_period_start,
            current_period_end,
            usage,
        })
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

fn first_string_obj(v: &Value, keys: &[&str]) -> Option<String> {
    let obj = v.as_object()?;
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|v| v.as_str()) {
            return Some(s.to_string());
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
