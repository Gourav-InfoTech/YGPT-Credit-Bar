use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE: &str = "ai.yourgpt.creditbar";
const TOKEN_ACCOUNT: &str = "token";
const SETTINGS_ACCOUNT: &str = "settings";

fn entry(account: &str) -> Result<Entry> {
    Entry::new(SERVICE, account).with_context(|| format!("open keychain entry: {account}"))
}

pub fn load_token() -> Option<String> {
    let e = entry(TOKEN_ACCOUNT).ok()?;
    e.get_password().ok()
}

pub fn save_token(token: &str) -> Result<()> {
    let e = entry(TOKEN_ACCOUNT)?;
    e.set_password(token).context("save token to keychain")?;
    Ok(())
}

pub fn delete_token() -> Result<()> {
    if let Ok(e) = entry(TOKEN_ACCOUNT) {
        let _ = e.delete_credential();
    }
    Ok(())
}

pub fn load_settings_json() -> Option<String> {
    let e = entry(SETTINGS_ACCOUNT).ok()?;
    e.get_password().ok()
}

pub fn save_settings_json(json: &str) -> Result<()> {
    let e = entry(SETTINGS_ACCOUNT)?;
    e.set_password(json).context("save settings to keychain")?;
    Ok(())
}

pub fn delete_settings() -> Result<()> {
    if let Ok(e) = entry(SETTINGS_ACCOUNT) {
        let _ = e.delete_credential();
    }
    Ok(())
}
