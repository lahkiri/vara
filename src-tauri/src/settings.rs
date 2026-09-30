//! Settings persistence: human-readable JSON in the app data dir.

use std::path::Path;
use vara_core::types::Settings;

pub fn load(dir: &Path) -> Settings {
    let p = dir.join("settings.json");
    match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str::<Settings>(&s).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(dir: &Path, s: &Settings) -> Result<(), String> {
    let p = dir.join("settings.json");
    let tmp = dir.join("settings.json.tmp");
    let body = serde_json::to_string_pretty(s).map_err(|e| format!("serialize settings: {e}"))?;
    std::fs::write(&tmp, body).map_err(|e| format!("write settings: {e}"))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("rename settings: {e}"))?;
    Ok(())
}
