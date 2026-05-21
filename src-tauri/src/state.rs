use crate::keychain;
use crate::models::Snapshot;
use serde::{Deserialize, Serialize};
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
                if s.interval_secs == 0 {
                    s.interval_secs = 30;
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
    pub last_notified_pct: RwLock<f64>, // tracks the last threshold we notified for, to avoid duplicate alerts
    pub last_error: RwLock<Option<String>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            snapshot: RwLock::new(None),
            settings: RwLock::new(UserSettings::load()),
            last_notified_pct: RwLock::new(0.0),
            last_error: RwLock::new(None),
        }
    }

    pub fn has_token(&self) -> bool {
        keychain::load_token().is_some()
    }

    pub fn token(&self) -> Option<String> {
        keychain::load_token()
    }
}
