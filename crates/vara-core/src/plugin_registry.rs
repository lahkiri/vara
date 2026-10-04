//! The plugin registry — what is installed, what is enabled, and what the
//! owner has decided about each one.
//!
//! A manifest says what a plugin *is*. This layer records what the owner
//! *decided*: enabled or disabled, and whether a plugin that asks for dangerous
//! permissions has ever been approved. Keeping the two apart is what makes the
//! decision auditable — the manifest cannot change the answer, and re-enabling a
//! plugin cannot silently reuse an old approval.
//!
//! State lives in one JSON file beside the database, so an interrupted write
//! cannot corrupt the entity's memory, and a corrupt state file degrades to
//! "everything at its default" rather than to "nothing loads".

use crate::plugin::{discover, load_order, Manifest, PluginFolder, Slot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The owner's decision about one plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginState {
    /// `None` means "use the manifest's default"; set it to remember a change.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Hash of the manifest at the moment the owner approved its permissions.
    /// A plugin whose permissions change after approval is un-approved again:
    /// an approval is for what was shown, not for the id.
    #[serde(default)]
    pub approved_hash: Option<String>,
    /// Free-form notes the owner left, shown in the plugin list.
    #[serde(default)]
    pub note: String,
}

impl PluginState {
    pub fn is_approved(&self, manifest: &Manifest) -> bool {
        match &self.approved_hash {
            Some(hash) => {
                hash.eq_ignore_ascii_case(&manifest.sha256) && !manifest.sha256.is_empty()
            }
            None => manifest.permissions.dangerous().is_empty(),
        }
    }

    pub fn approve(&mut self, manifest: &Manifest) {
        if !manifest.sha256.is_empty() {
            self.approved_hash = Some(manifest.sha256.clone());
        }
    }
}

/// The whole registry file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistryState {
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginState>,
}

/// One row in the plugin list the UI renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub slots: Vec<Slot>,
    pub shipped: bool,
    /// Will it load on the next boot?
    pub enabled: bool,
    /// Has the owner seen and accepted what it asks for?
    pub approved: bool,
    /// Empty when the manifest carries a valid hash.
    pub integrity: Integrity,
    /// What it asks for, in the owner's words.
    pub asks: Vec<String>,
    /// Where it lives, for "open folder".
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Integrity {
    /// A declared hash that matches.
    Verified,
    /// No hash at all: a locally authored plugin. Usable, but labelled.
    Unverified,
    /// A hash that does not match. Never loaded.
    Broken { expected: String, actual: String },
}

impl Integrity {
    pub fn label(&self) -> &'static str {
        match self {
            Integrity::Verified => "verified",
            Integrity::Unverified => "unverified",
            Integrity::Broken { .. } => "BROKEN",
        }
    }
    pub fn is_loadable(&self) -> bool {
        !matches!(self, Integrity::Broken { .. })
    }
}

/// The registry: discovered plugins plus the owner's state file.
pub struct PluginRegistry {
    state_path: PathBuf,
    rows: Vec<PluginRow>,
    /// Broken manifests, kept so the UI can explain them instead of hiding them.
    errors: Vec<(PathBuf, String)>,
}

impl PluginRegistry {
    /// Scan `plugin_dirs` and merge the owner's decisions from `state_path`.
    pub fn scan(plugin_dirs: &[PathBuf], state_path: &Path) -> Self {
        let state = load_state(state_path);
        let mut rows = Vec::new();
        let mut errors = Vec::new();

        for found in discover(plugin_dirs, false) {
            match found {
                Ok(folder) => {
                    let manifest = &folder.manifest;
                    let integrity = match manifest.verify_hash() {
                        Ok(true) => Integrity::Verified,
                        Ok(false) => Integrity::Unverified,
                        Err(crate::plugin::ManifestError::HashMismatch { expected, actual }) => {
                            Integrity::Broken { expected, actual }
                        }
                        Err(_) => Integrity::Unverified,
                    };
                    let saved = state.plugins.get(&manifest.id).cloned().unwrap_or_default();
                    rows.push(PluginRow {
                        id: manifest.id.clone(),
                        name: manifest.name.clone(),
                        version: manifest.version.clone(),
                        summary: manifest.summary.clone(),
                        slots: manifest.slots.clone(),
                        shipped: manifest.shipped,
                        enabled: saved.enabled.unwrap_or(manifest.default_enabled),
                        approved: saved.is_approved(manifest),
                        integrity,
                        asks: manifest
                            .permissions
                            .dangerous()
                            .iter()
                            .map(|s| s.to_string())
                            .collect(),
                        path: folder.root.clone(),
                    });
                }
                Err(why) => errors.push((plugin_dirs.first().cloned().unwrap_or_default(), why)),
            }
        }

        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            state_path: state_path.to_path_buf(),
            rows,
            errors,
        }
    }

    pub fn rows(&self) -> &[PluginRow] {
        &self.rows
    }

    pub fn errors(&self) -> &[(PathBuf, String)] {
        &self.errors
    }

    pub fn get(&self, id: &str) -> Option<&PluginRow> {
        self.rows.iter().find(|r| r.id == id)
    }

    /// Turn a plugin on or off. Refuses to enable a plugin whose integrity check
    /// failed, and refuses to enable one that asks for permissions the owner has
    /// not approved — both are policy, not UI convenience.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        let row = self
            .rows
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("no plugin '{id}'"))?;
        if enabled {
            if let Integrity::Broken { expected, actual } = &row.integrity {
                return Err(format!(
                    "'{id}' failed its integrity check (expected {expected}, the files hash to {actual}) — refusing to load it"
                ));
            }
            if !row.asks.is_empty() && !row.approved {
                return Err(format!(
                    "'{id}' asks to {} — approve it first",
                    row.asks.join(", ")
                ));
            }
        }
        row.enabled = enabled;

        let mut state = load_state(&self.state_path);
        state.plugins.entry(id.to_string()).or_default().enabled = Some(enabled);
        save_state(&self.state_path, &state)
    }

    /// Record that the owner accepted what this plugin asks for.
    pub fn approve(&mut self, id: &str) -> Result<(), String> {
        let row = self
            .rows
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("no plugin '{id}'"))?;
        // Re-read the manifest so the approval is bound to the *current* claims.
        let folder = PluginFolder::load(&row.path, false).map_err(|e| e.to_string())?;
        let mut state = load_state(&self.state_path);
        state
            .plugins
            .entry(id.to_string())
            .or_default()
            .approve(&folder.manifest);
        row.approved = true;
        save_state(&self.state_path, &state)
    }

    /// The ids that should load, in dependency order.
    ///
    /// Disabled plugins are withheld, and a dependency on a withheld plugin is
    /// reported: a plugin that cannot work must not be loaded and then fail
    /// mysteriously at first use.
    pub fn load_plan(&self) -> Result<Vec<String>, String> {
        let enabled: Vec<(String, Vec<String>)> = self
            .rows
            .iter()
            .filter(|r| r.enabled && r.integrity.is_loadable())
            .map(|r| {
                // The requires list comes from the manifest on disk; a row only
                // caches what the UI needs, not what loading needs.
                let requires = PluginFolder::load(&r.path, false)
                    .map(|f| f.manifest.requires.clone())
                    .unwrap_or_default();
                (r.id.clone(), requires)
            })
            .collect();

        // Withheld dependencies: named, because "it just did not load" is the
        // least debuggable failure a plugin system can have.
        let enabled_ids: Vec<&String> = enabled.iter().map(|(id, _)| id).collect();
        let mut withheld = Vec::new();
        for (id, requires) in &enabled {
            for dep in requires {
                let dep_enabled = enabled_ids.contains(&dep);
                let dep_exists = self.rows.iter().any(|r| &r.id == dep);
                if !dep_enabled {
                    withheld.push(format!(
                        "'{id}' needs '{dep}', which is {}{}",
                        if dep_exists {
                            "disabled"
                        } else {
                            "not installed"
                        },
                        if dep_exists {
                            " — enable it or disable the dependent plugin"
                        } else {
                            ""
                        }
                    ));
                }
            }
        }
        if !withheld.is_empty() {
            return Err(withheld.join("; "));
        }

        load_order(&enabled)
    }

    /// Everything the owner can turn on, grouped by slot — the shape a settings
    /// screen needs without re-deriving it from the rows.
    pub fn by_slot(&self) -> BTreeMap<&'static str, Vec<&PluginRow>> {
        let mut map: BTreeMap<&'static str, Vec<&PluginRow>> = BTreeMap::new();
        for row in &self.rows {
            for slot in &row.slots {
                map.entry(slot.as_str()).or_default().push(row);
            }
        }
        map
    }
}

fn load_state(path: &Path) -> RegistryState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str::<RegistryState>(&body).ok())
        // A corrupt or absent file means "everything at its default", which is
        // strictly better than a product that refuses to start.
        .unwrap_or_default()
}

fn save_state(path: &Path, state: &RegistryState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    // Write through a temporary file: an interrupted save must not leave a
    // half-written state that changes what loads next boot.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Where the shipped plugins live, relative to the workspace or an install.
pub fn default_plugin_dirs(install_root: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(root) = install_root {
        dirs.push(root.join("plugins"));
    }
    // A user-level folder for plugins the owner adds, kept separate from the
    // shipped ones so an update never overwrites their work.
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("app.vara.entity").join("plugins"));
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{Manifest, Permissions, Slot};

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vara-reg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_plugin(
        root: &Path,
        folder: &str,
        id: &str,
        slots: Vec<Slot>,
        requires: Vec<&str>,
        dangerous: bool,
        default_enabled: bool,
    ) -> PathBuf {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = Manifest {
            id: id.into(),
            name: format!("Plugin {id}"),
            version: "1.0.0".into(),
            summary: "test".into(),
            slots,
            permissions: Permissions {
                write_files: dangerous,
                ..Default::default()
            },
            requires: requires.into_iter().map(|s| s.to_string()).collect(),
            replaces: Vec::new(),
            shipped: true,
            default_enabled,
            config: Default::default(),
            files: Vec::new(),
            sha256: String::new(),
        };
        m.apply_hash();
        std::fs::write(dir.join("plugin.toml"), toml::to_string(&m).unwrap()).unwrap();
        dir
    }

    #[test]
    fn scan_reads_the_folder_and_honours_manifest_defaults() {
        let root = tmp("scan");
        write_plugin(&root, "a", "vara.a", vec![Slot::Tool], vec![], false, true);
        write_plugin(
            &root,
            "b",
            "vara.b",
            vec![Slot::Theme],
            vec![],
            false,
            false,
        );
        let state = root.join("state.json");

        let reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        assert_eq!(reg.rows().len(), 2);
        assert!(reg.get("vara.a").unwrap().enabled);
        assert!(!reg.get("vara.b").unwrap().enabled);
        assert_eq!(reg.get("vara.a").unwrap().integrity, Integrity::Verified);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_permission_asking_plugin_is_not_approved_by_default_and_cannot_start() {
        let root = tmp("approve");
        write_plugin(&root, "w", "vara.w", vec![Slot::Tool], vec![], true, true);
        let state = root.join("state.json");

        let mut reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        let row = reg.get("vara.w").unwrap();
        assert!(!row.approved, "dangerous permissions start unapproved");
        assert_eq!(row.asks, vec!["write files".to_string()]);

        // Enabling first must fail…
        let err = reg.set_enabled("vara.w", true).unwrap_err();
        assert!(err.contains("approve it first"), "{err}");
        // …approving then enabling must work.
        reg.approve("vara.w").unwrap();
        assert!(reg.get("vara.w").unwrap().approved);
        reg.set_enabled("vara.w", true).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn approval_is_bound_to_the_claims_not_to_the_id() {
        let root = tmp("rebind");
        let dir = write_plugin(&root, "w", "vara.w", vec![Slot::Tool], vec![], true, true);
        let state = root.join("state.json");
        let mut reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        reg.approve("vara.w").unwrap();

        // The plugin now asks for MORE than what was approved.
        let mut m = Manifest {
            id: "vara.w".into(),
            name: "Plugin vara.w".into(),
            version: "1.0.0".into(),
            summary: "test".into(),
            slots: vec![Slot::Tool],
            permissions: Permissions {
                write_files: true,
                run_programs: true,
                ..Default::default()
            },
            requires: vec![],
            replaces: vec![],
            shipped: true,
            default_enabled: true,
            config: Default::default(),
            files: vec![],
            sha256: String::new(),
        };
        m.apply_hash();
        std::fs::write(dir.join("plugin.toml"), toml::to_string(&m).unwrap()).unwrap();

        let reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        assert!(
            !reg.get("vara.w").unwrap().approved,
            "a changed permission set must require a new approval"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn state_survives_a_restart() {
        let root = tmp("persist");
        write_plugin(&root, "a", "vara.a", vec![Slot::Tool], vec![], false, true);
        let state = root.join("state.json");

        let mut reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        reg.set_enabled("vara.a", false).unwrap();

        let again = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        assert!(
            !again.get("vara.a").unwrap().enabled,
            "the owner's decision must persist"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_corrupt_state_file_degrades_to_defaults_instead_of_failing() {
        let root = tmp("corrupt");
        write_plugin(&root, "a", "vara.a", vec![Slot::Tool], vec![], false, true);
        let state = root.join("state.json");
        std::fs::write(&state, "{ this is not json").unwrap();

        let reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        assert!(reg.get("vara.a").unwrap().enabled, "defaults apply");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tampered_manifest_is_broken_and_refuses_to_enable() {
        let root = tmp("tamper");
        let dir = write_plugin(&root, "a", "vara.a", vec![Slot::Tool], vec![], false, true);
        let state = root.join("state.json");
        // Change the claims without updating the hash.
        let body = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
        let body = body.replace("version = \"1.0.0\"", "version = \"9.9.9\"");
        std::fs::write(dir.join("plugin.toml"), body).unwrap();

        let mut reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        assert!(matches!(
            reg.get("vara.a").unwrap().integrity,
            Integrity::Broken { .. }
        ));
        let err = reg.set_enabled("vara.a", true).unwrap_err();
        assert!(err.contains("integrity"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_load_plan_orders_dependencies_and_withholds_the_disabled() {
        let root = tmp("plan");
        write_plugin(
            &root,
            "a",
            "vara.a",
            vec![Slot::Tool],
            vec!["vara.b"],
            false,
            true,
        );
        write_plugin(&root, "b", "vara.b", vec![Slot::Tool], vec![], false, true);
        write_plugin(
            &root,
            "off",
            "vara.off",
            vec![Slot::Tool],
            vec![],
            false,
            false,
        );
        let state = root.join("state.json");

        let reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        let plan = reg.load_plan().unwrap();
        assert!(!plan.contains(&"vara.off".to_string()));
        let pos = |id: &str| plan.iter().position(|p| p == id).unwrap();
        assert!(pos("vara.b") < pos("vara.a"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_disabled_dependency_is_reported_by_name() {
        let root = tmp("withheld");
        write_plugin(
            &root,
            "a",
            "vara.a",
            vec![Slot::Tool],
            vec!["vara.b"],
            false,
            true,
        );
        write_plugin(&root, "b", "vara.b", vec![Slot::Tool], vec![], false, false);
        let state = root.join("state.json");

        let reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        let err = reg.load_plan().unwrap_err();
        assert!(err.contains("vara.b"), "{err}");
        assert!(err.contains("disabled"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_dependency_is_distinguished_from_a_disabled_one() {
        let root = tmp("missing");
        write_plugin(
            &root,
            "a",
            "vara.a",
            vec![Slot::Tool],
            vec!["vara.ghost"],
            false,
            true,
        );
        let state = root.join("state.json");
        let reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        let err = reg.load_plan().unwrap_err();
        assert!(err.contains("not installed"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rows_group_by_slot_for_a_settings_screen() {
        let root = tmp("slots");
        write_plugin(
            &root,
            "a",
            "vara.a",
            vec![Slot::Tool, Slot::Toolset],
            vec![],
            false,
            true,
        );
        write_plugin(&root, "t", "vara.t", vec![Slot::Theme], vec![], false, true);
        let state = root.join("state.json");
        let reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        let grouped = reg.by_slot();
        assert_eq!(grouped.get("tool").unwrap().len(), 1);
        assert_eq!(grouped.get("toolset").unwrap().len(), 1);
        assert_eq!(grouped.get("theme").unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_invalid_manifest_is_reported_not_hidden() {
        let root = tmp("invalid");
        let dir = root.join("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.toml"), "id = \"vara.bad\"\nslots = []\n").unwrap();
        let state = root.join("state.json");
        let reg = PluginRegistry::scan(std::slice::from_ref(&root), &state);
        assert!(reg.rows().is_empty());
        assert_eq!(
            reg.errors().len(),
            1,
            "a plugin that exists but is invalid must be visible"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_plugin_dir_is_not_an_error() {
        let root = tmp("nodir");
        let state = root.join("state.json");
        let reg = PluginRegistry::scan(&[root.join("does-not-exist")], &state);
        assert!(reg.rows().is_empty());
        assert!(reg.errors().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
