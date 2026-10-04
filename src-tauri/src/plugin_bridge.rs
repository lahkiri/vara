//! The plugin surface: what is installed, and the owner's decisions about it.
//!
//! The entity's capabilities must be inspectable *and changeable from the app*,
//! not only from a terminal — otherwise "everything is a plugin" is true of the
//! code and false of the product. This module is the thin Tauri layer over
//! `vara_core::plugin_registry`; all policy lives in the core, so the desktop
//! app, the CLI and any future interface answer the same questions identically.

use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// One plugin as the UI needs it. Flat and string-typed on purpose: the webview
/// receives what it can render, and never the manifest's internal shape.
#[derive(Debug, Clone, Serialize)]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    /// Slot names, already lowercased for grouping in the UI.
    pub slots: Vec<String>,
    pub shipped: bool,
    pub enabled: bool,
    pub approved: bool,
    /// "verified" | "unverified" | "BROKEN"
    pub integrity: String,
    /// What it asks for, in the owner's words: ["write files", "use the network"]
    pub asks: Vec<String>,
    /// Absolute path, so the UI can offer "open folder".
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginReport {
    pub plugins: Vec<PluginView>,
    /// Manifests that exist but could not be read. Shown, never hidden —
    /// a plugin the owner believes is installed and is not is the worst state.
    pub errors: Vec<String>,
    /// The plugins that would load on the next start, in dependency order.
    pub plan: Vec<String>,
    /// Why the plan is not usable, when it is not.
    pub plan_error: Option<String>,
    /// Where user-added plugins are looked for.
    pub user_dir: String,
    pub enabled_count: usize,
    pub installed_count: usize,
}

/// Where plugins live: the shipped folder inside the install, plus a user folder
/// that an app update never overwrites.
fn plugin_dirs(app: &AppHandle) -> (Vec<PathBuf>, PathBuf) {
    let mut dirs = Vec::new();

    // 1) shipped plugins, beside the executable's resources
    if let Ok(resource) = app.path().resource_dir() {
        let shipped = resource.join("plugins");
        if shipped.is_dir() {
            dirs.push(shipped);
        }
    }
    // 2) the checkout, when running in development
    if cfg!(debug_assertions) {
        if let Ok(cwd) = std::env::current_dir() {
            for candidate in [cwd.join("plugins"), cwd.join("..").join("plugins")] {
                if candidate.is_dir() {
                    let resolved = candidate
                        .canonicalize()
                        .unwrap_or_else(|_| candidate.clone());
                    if !dirs.contains(&resolved) {
                        dirs.push(resolved);
                    }
                }
            }
        }
    }
    // 3) the owner's own folder
    let user_dir = app
        .path()
        .app_data_dir()
        .map(|d| d.join("plugins"))
        .unwrap_or_else(|_| PathBuf::from(".").join("plugins"));
    dirs.push(user_dir.clone());

    (dirs, user_dir)
}

fn state_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("plugins.json"))
        .map_err(|e| format!("cannot resolve the data directory: {e}"))
}

fn registry(app: &AppHandle) -> Result<vara_core::plugin_registry::PluginRegistry, String> {
    let (dirs, _) = plugin_dirs(app);
    let state = state_path(app)?;
    Ok(vara_core::plugin_registry::PluginRegistry::scan(
        &dirs, &state,
    ))
}

fn report_from(reg: &vara_core::plugin_registry::PluginRegistry, user_dir: &str) -> PluginReport {
    let plan = reg.load_plan();
    let (plan_ids, plan_error) = match plan {
        Ok(ids) => (ids, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    PluginReport {
        enabled_count: reg.rows().iter().filter(|r| r.enabled).count(),
        installed_count: reg.rows().len(),
        plugins: reg
            .rows()
            .iter()
            .map(|r| PluginView {
                id: r.id.clone(),
                name: r.name.clone(),
                version: r.version.clone(),
                summary: r.summary.clone(),
                slots: r.slots.iter().map(|s| s.as_str().to_string()).collect(),
                shipped: r.shipped,
                enabled: r.enabled,
                approved: r.approved,
                integrity: r.integrity.label().to_string(),
                asks: r.asks.clone(),
                path: r.path.display().to_string(),
            })
            .collect(),
        errors: reg.errors().iter().map(|(_, e)| e.clone()).collect(),
        plan: plan_ids,
        plan_error,
        user_dir: user_dir.to_string(),
    }
}

/// The full plugin list plus the composed plan — what a settings screen renders.
#[tauri::command]
pub fn list_plugins(app: AppHandle) -> Result<PluginReport, String> {
    let reg = registry(&app)?;
    let (_, user_dir) = plugin_dirs(&app);
    Ok(report_from(&reg, &user_dir.display().to_string()))
}

/// Turn one plugin on or off. Refusals (broken integrity, unapproved
/// permissions) come straight from the core, so the UI shows the same reason the
/// CLI prints instead of inventing its own rule.
#[tauri::command]
pub fn set_plugin_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<PluginReport, String> {
    let mut reg = registry(&app)?;
    reg.set_enabled(&id, enabled)?;
    let (_, user_dir) = plugin_dirs(&app);
    Ok(report_from(&reg, &user_dir.display().to_string()))
}

/// Accept what a plugin asks for. Bound to the current claims: if the manifest
/// changes afterwards, the approval no longer applies.
#[tauri::command]
pub fn approve_plugin(app: AppHandle, id: String) -> Result<PluginReport, String> {
    let mut reg = registry(&app)?;
    reg.approve(&id)?;
    let (_, user_dir) = plugin_dirs(&app);
    Ok(report_from(&reg, &user_dir.display().to_string()))
}

/// Open the folder a plugin lives in, so "where is this?" has an answer.
#[tauri::command]
pub fn reveal_plugin(app: AppHandle, id: String) -> Result<(), String> {
    let reg = registry(&app)?;
    let row = reg.get(&id).ok_or_else(|| format!("no plugin '{id}'"))?;
    let path = row.path.clone();
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(opener)
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// The user plugin folder, created on demand so "add a plugin" has somewhere to
/// go. Returns the path the UI should tell the owner about.
#[tauri::command]
pub fn open_user_plugin_dir(app: AppHandle) -> Result<String, String> {
    let (_, user_dir) = plugin_dirs(&app);
    std::fs::create_dir_all(&user_dir).map_err(|e| e.to_string())?;
    // A README in the folder so a non-technical owner who opens it sees what it
    // is for rather than an empty directory they cannot interpret.
    let readme = user_dir.join("README.md");
    if !readme.exists() {
        let _ = std::fs::write(
            &readme,
            "# إضافاتك\n\nضع كل إضافة في مجلد خاص بها، وداخله ملف `plugin.toml`.\n\
             الدليل الكامل في `docs/PLUGIN_GUIDE.md`.\n\n\
             ### Your plugins\n\nEach plugin is a folder containing a `plugin.toml`.\n\
             See `docs/PLUGIN_GUIDE.md` for the format.\n",
        );
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&user_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(user_dir.display().to_string())
}
