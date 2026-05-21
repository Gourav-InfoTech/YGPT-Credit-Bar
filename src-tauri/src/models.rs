use serde::{Deserialize, Serialize};

/// A single quota bucket (credits, voice_credits, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bucket {
    #[serde(deserialize_with = "de_string_or_number")]
    pub usage: f64,
    #[serde(deserialize_with = "de_string_or_number")]
    pub limit: f64,
}

impl Bucket {
    pub fn percent(&self) -> f64 {
        if self.limit <= 0.0 {
            0.0
        } else {
            (self.usage / self.limit * 100.0).clamp(0.0, 100.0)
        }
    }
}

/// All quota buckets returned by getOrgPlanDetail. All fields optional because some plans don't
/// have voice_lite_credits, campaign_credits, etc.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub chatbot: Option<Bucket>,
    pub document: Option<Bucket>,
    pub webpages: Option<Bucket>,
    pub credits: Option<Bucket>,
    pub voice_credits: Option<Bucket>,
    pub voice_lite_credits: Option<Bucket>,
    pub campaign_credits: Option<Bucket>,
    pub members: Option<Bucket>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Idle,
    Ok,
    Warn,
    Alert,
}

impl Usage {
    /// Returns the worst severity across all populated buckets, plus the worst pct.
    pub fn worst_severity(&self) -> (Severity, f64) {
        let buckets: Vec<&Bucket> = [
            &self.chatbot,
            &self.document,
            &self.webpages,
            &self.credits,
            &self.voice_credits,
            &self.voice_lite_credits,
            &self.campaign_credits,
            &self.members,
        ]
        .iter()
        .filter_map(|b| b.as_ref())
        .collect();

        if buckets.is_empty() {
            return (Severity::Idle, 0.0);
        }

        let worst_pct = buckets
            .iter()
            .map(|b| b.percent())
            .fold(0.0_f64, f64::max);

        let sev = if worst_pct >= 90.0 {
            Severity::Alert
        } else if worst_pct >= 70.0 {
            Severity::Warn
        } else {
            Severity::Ok
        };
        (sev, worst_pct)
    }
}

/// Snapshot returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub org_name: String,
    pub plan_name: Option<String>,
    pub subscription_status: Option<String>,
    pub current_period_start: Option<i64>,
    pub current_period_end: Option<i64>,
    pub usage: Usage,
    pub fetched_at_ms: i64,
    pub severity: Severity,
    pub worst_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Org {
    pub id: String,
    pub name: String,
}

/// YourGPT API tolerantly returns numbers as either strings or numbers. Coerce to f64.
fn de_string_or_number<'de, D>(d: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;
    let v = Value::deserialize(d)?;
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| Error::custom("invalid number")),
        Value::String(s) => s.parse::<f64>().map_err(Error::custom),
        Value::Null => Ok(0.0),
        other => Err(Error::custom(format!("unexpected type: {other:?}"))),
    }
}
