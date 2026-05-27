use crate::keychain;
use crate::models::Snapshot;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserSettings {
    pub organization_id: Option<String>,
    pub organization_name: Option<String>,
    pub interval_secs: u64,
}

impl UserSettings {
    pub fn defaults() -> Self {
        Self {
            organization_id: None,
            organization_name: None,
            interval_secs: 30,
        }
    }

    pub fn load() -> Self {
        keychain::load_settings_json()
            .and_then(|json| serde_json::from_str::<UserSettings>(&json).ok())
            .map(|mut s| {
                // Guard against corrupt or stale Keychain values driving extreme poll
                // cadences. `save_settings` already clamps before write, but a build that
                // wrote out-of-range values previously (or hand-edited Keychain entries)
                // would still be loaded as-is.
                if s.interval_secs == 0 {
                    s.interval_secs = 30;
                } else {
                    s.interval_secs = s.interval_secs.clamp(15, 300);
                }
                s
            })
            .unwrap_or_else(Self::defaults)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let json = serde_json::to_string(self)?;
        keychain::save_settings_json(&json)?;
        Ok(())
    }
}

pub struct AppState {
    pub snapshot: RwLock<Option<Snapshot>>,
    pub settings: RwLock<UserSettings>,
    pub last_error: RwLock<Option<String>>,
    /// Per-(org, bucket, threshold) dedup so we fire each "Org X — Chatbots reached 100%"
    /// banner at most once per session, even though the poller re-checks every cycle.
    /// Tuple = (organization_id, bucket_label, threshold_pct).
    pub fired_thresholds: RwLock<HashSet<(String, String, u8)>>,
    /// IDs of YourGPT-server billing notifications we've already fired a native banner for
    /// this session. In-memory only — a cold start may re-announce truly-unread items, which
    /// is desired (user gets a fresh ping for anything they haven't acknowledged yet).
    pub announced_notification_ids: RwLock<HashSet<i64>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            snapshot: RwLock::new(None),
            settings: RwLock::new(UserSettings::load()),
            last_error: RwLock::new(None),
            fired_thresholds: RwLock::new(HashSet::new()),
            announced_notification_ids: RwLock::new(HashSet::new()),
        }
    }

    pub fn has_token(&self) -> bool {
        keychain::load_token().is_some()
    }

    pub fn token(&self) -> Option<String> {
        keychain::load_token()
    }
}
