//! Settings persistence: human-readable JSON in the app data dir.
//!
//! Environment overrides (development & testing discipline): the provider
//! identity can be injected without touching settings.json — the key never
//! has to touch disk to be usable:
//!
//! - `VARA_PROVIDER_API_KEY`  → provider.api_key
//! - `VARA_PROVIDER_BASE_URL` → provider.base_url
//! - `VARA_PROVIDER_MODEL`    → provider.model
//!
//! Precedence: environment > settings.json. Overrides are re-applied after
//! every UI save, so an env-injected key can never be wiped (or leaked into
//! the persisted file) by a Settings-window save.

use std::path::Path;
use vara_core::types::Settings;

/// Apply the `VARA_PROVIDER_*` environment overrides in place. Empty or
/// unset variables are ignored — only non-empty values override.
pub fn apply_env_overrides(s: &mut Settings) {
    if let Ok(v) = std::env::var("VARA_PROVIDER_API_KEY") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            s.provider.api_key = v;
        }
    }
    if let Ok(v) = std::env::var("VARA_PROVIDER_BASE_URL") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            s.provider.base_url = v;
        }
    }
    if let Ok(v) = std::env::var("VARA_PROVIDER_MODEL") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            s.provider.model = v;
        }
    }
}

pub fn load(dir: &Path) -> Settings {
    let p = dir.join("settings.json");
    let mut s = match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str::<Settings>(&s).unwrap_or_default(),
        Err(_) => Settings::default(),
    };
    apply_env_overrides(&mut s);
    s
}

pub fn save(dir: &Path, s: &Settings) -> Result<(), String> {
    let p = dir.join("settings.json");
    let tmp = dir.join("settings.json.tmp");
    let body = serde_json::to_string_pretty(s).map_err(|e| format!("serialize settings: {e}"))?;
    std::fs::write(&tmp, body).map_err(|e| format!("write settings: {e}"))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("rename settings: {e}"))?;
    Ok(())
}
