//! Profiles and bundles — how a run is composed.
//!
//! A host with no plugins does nothing, and a product that ships every plugin
//! loaded does everything badly. dsh solves this with **profiles** (a named
//! composition) stacked from **bundles** (a distribution unit of plugin rows),
//! applied in a fixed order, with per-profile patch files on top. The property
//! that matters for Vara: the *same* host runs a desktop app, a terminal UI or
//! a headless daemon purely by composing a different set — the interface is a
//! layer, never the product.
//!
//! This module is the data model for that, kept deliberately small and free of
//! any product knowledge. It knows plugin *ids*, not what they do.
//!
//! Ordering is the whole semantics, so it is spelled out here:
//!
//! 1. bundles in the order the profile lists them,
//! 2. then the profile's own `patch` (add / drop / reorder),
//! 3. then the home-level patch (the owner's own overrides),
//! 4. then any `--patch` overlay given at launch.
//!
//! Later layers may drop a plugin an earlier layer added, which is how an owner
//! turns a shipped profile into their own without forking it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One plugin row inside a bundle: an id plus opaque configuration.
///
/// `config` is a string on purpose. The host must not parse plugin settings —
/// each plugin owns its own schema, and a host that understood them would have
/// to change every time a plugin did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRow {
    pub id: String,
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    /// A row can be present but disabled: useful for a bundle that ships an
    /// optional capability switched off by default (the pattern Vara already
    /// uses for screenshots and commands).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl PluginRow {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            config: BTreeMap::new(),
            enabled: true,
        }
    }

    pub fn disabled(id: &str) -> Self {
        Self {
            enabled: false,
            ..Self::new(id)
        }
    }

    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.config.insert(key.to_string(), value.to_string());
        self
    }
}

/// A bundle: the distribution format for a set of plugin rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub name: String,
    #[serde(default)]
    pub rows: Vec<PluginRow>,
}

impl Bundle {
    pub fn new(name: &str, rows: Vec<PluginRow>) -> Self {
        Self {
            name: name.to_string(),
            rows,
        }
    }
}

/// A patch layer: add rows, drop rows by id, or re-enable one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    /// Rows to append (or to replace, when the id already exists).
    #[serde(default)]
    pub add: Vec<PluginRow>,
    /// Ids to remove entirely.
    #[serde(default)]
    pub drop: Vec<String>,
}

impl Patch {
    pub fn dropping(ids: &[&str]) -> Self {
        Self {
            add: Vec::new(),
            drop: ids.iter().map(|s| s.to_string()).collect(),
        }
    }
    pub fn adding(rows: Vec<PluginRow>) -> Self {
        Self {
            add: rows,
            drop: Vec::new(),
        }
    }
}

/// A named composition: an ordered list of bundles plus its own patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub bundles: Vec<String>,
    #[serde(default)]
    pub patch: Patch,
    /// One line the UI can show, so "which profile am I running" is answerable.
    #[serde(default)]
    pub description: String,
}

impl Profile {
    pub fn new(name: &str, bundles: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            bundles: bundles.iter().map(|s| s.to_string()).collect(),
            patch: Patch::default(),
            description: String::new(),
        }
    }
    pub fn described(mut self, text: &str) -> Self {
        self.description = text.to_string();
        self
    }
}

/// Why a composition could not be built. Every variant is user-explainable:
/// a missing bundle is a packaging bug the owner should see, not a silent skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    UnknownBundle(String),
    UnknownProfile(String),
    DuplicatePluginId(String),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComposeError::UnknownBundle(name) => write!(f, "unknown bundle '{name}'"),
            ComposeError::UnknownProfile(name) => write!(f, "unknown profile '{name}'"),
            ComposeError::DuplicatePluginId(id) => write!(
                f,
                "plugin '{id}' is listed twice in the composition — a plugin may only appear once"
            ),
        }
    }
}

/// The registry of everything a host could compose.
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    bundles: BTreeMap<String, Bundle>,
    profiles: BTreeMap<String, Profile>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_bundle(&mut self, bundle: Bundle) -> &mut Self {
        self.bundles.insert(bundle.name.clone(), bundle);
        self
    }

    pub fn add_profile(&mut self, profile: Profile) -> &mut Self {
        self.profiles.insert(profile.name.clone(), profile);
        self
    }

    pub fn profiles(&self) -> Vec<&str> {
        self.profiles.keys().map(|s| s.as_str()).collect()
    }

    pub fn bundles(&self) -> Vec<&str> {
        self.bundles.keys().map(|s| s.as_str()).collect()
    }

    /// Compose a profile with any number of overlay patches, in the documented
    /// order: bundles → profile patch → each overlay.
    pub fn compose(
        &self,
        profile: &str,
        overlays: &[Patch],
    ) -> Result<Vec<PluginRow>, ComposeError> {
        let profile = self
            .profiles
            .get(profile)
            .ok_or_else(|| ComposeError::UnknownProfile(profile.to_string()))?;

        let mut rows: Vec<PluginRow> = Vec::new();
        for bundle_name in &profile.bundles {
            let bundle = self
                .bundles
                .get(bundle_name)
                .ok_or_else(|| ComposeError::UnknownBundle(bundle_name.clone()))?;
            rows.extend(bundle.rows.iter().cloned());
        }

        apply(&mut rows, &profile.patch);
        for overlay in overlays {
            apply(&mut rows, overlay);
        }

        // A plugin listed twice is a composition bug: which config wins would
        // otherwise depend on layer order in a way nobody can reason about.
        for (i, row) in rows.iter().enumerate() {
            if rows.iter().skip(i + 1).any(|other| other.id == row.id) {
                return Err(ComposeError::DuplicatePluginId(row.id.clone()));
            }
        }
        Ok(rows)
    }

    /// The ids a profile would load, enabled ones only — what `host.load_all`
    /// will actually be handed.
    pub fn load_order(
        &self,
        profile: &str,
        overlays: &[Patch],
    ) -> Result<Vec<String>, ComposeError> {
        Ok(self
            .compose(profile, overlays)?
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| r.id)
            .collect())
    }
}

fn apply(rows: &mut Vec<PluginRow>, patch: &Patch) {
    for id in &patch.drop {
        rows.retain(|row| &row.id != id);
    }
    for row in &patch.add {
        // A patch that re-adds an id replaces the earlier row wholesale, which
        // is what "patch replaces the row's whole config" means.
        if let Some(existing) = rows.iter_mut().find(|r| r.id == row.id) {
            *existing = row.clone();
        } else {
            rows.push(row.clone());
        }
    }
}

/// The shipped catalog. Kept as data so it can be dumped, diffed and patched —
/// `catalog()` is the equivalent of `dsh --dump-config`.
pub fn shipped_catalog() -> Catalog {
    let mut catalog = Catalog::new();

    catalog.add_bundle(Bundle::new(
        "core",
        vec![
            PluginRow::new("store.sqlite"),
            PluginRow::new("policy.gate"),
            PluginRow::new("approvals.queue"),
            PluginRow::new("brain.openai-compatible"),
            PluginRow::new("tools.read"),
            PluginRow::new("memory.notes"),
        ],
    ));
    catalog.add_bundle(Bundle::new(
        "research",
        vec![
            PluginRow::new("tools.web"),
            PluginRow::new("provenance.gate"),
        ],
    ));
    catalog.add_bundle(Bundle::new(
        "desktop",
        vec![
            PluginRow::new("interface.tauri"),
            PluginRow::new("tray.native"),
            PluginRow::new("notify.system"),
        ],
    ));
    catalog.add_bundle(Bundle::new(
        "terminal",
        vec![PluginRow::new("interface.tui")],
    ));
    catalog.add_bundle(Bundle::new(
        "headless",
        vec![PluginRow::new("interface.rpc")],
    ));
    catalog.add_bundle(Bundle::new(
        "initiative",
        vec![PluginRow::new("heartbeat.checklist")],
    ));
    // Everything dangerous ships present but disabled — the product's existing
    // discipline (commands, screenshots and computer use ship OFF).
    catalog.add_bundle(Bundle::new(
        "actions",
        vec![
            PluginRow::disabled("tools.write"),
            PluginRow::disabled("tools.run"),
            PluginRow::disabled("computer.use"),
        ],
    ));

    catalog.add_profile(
        Profile::new(
            "desktop",
            &["core", "research", "initiative", "actions", "desktop"],
        )
        .described("The full resident app: chat, research, tray, approvals"),
    );
    catalog.add_profile(
        Profile::new(
            "tui",
            &["core", "research", "initiative", "actions", "terminal"],
        )
        .described("The same entity in a terminal, for servers and SSH"),
    );
    catalog.add_profile(
        Profile::new(
            "headless",
            &["core", "research", "initiative", "actions", "headless"],
        )
        .described("No interface at all: an RPC surface for a VPS or another program"),
    );
    catalog.add_profile(
        Profile::new("minimal", &["core"])
            .described("Only the store, the gate and a model — the smallest useful entity"),
    );
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_entity_runs_behind_three_interfaces() {
        let catalog = shipped_catalog();
        // The interface differs; the core is identical in all three.
        for profile in ["desktop", "tui", "headless"] {
            let ids = catalog.load_order(profile, &[]).unwrap();
            for core in [
                "store.sqlite",
                "policy.gate",
                "approvals.queue",
                "brain.openai-compatible",
            ] {
                assert!(ids.iter().any(|i| i == core), "{profile} is missing {core}");
            }
        }
        let desktop = catalog.load_order("desktop", &[]).unwrap();
        let tui = catalog.load_order("tui", &[]).unwrap();
        assert!(desktop.contains(&"interface.tauri".to_string()));
        assert!(!desktop.contains(&"interface.tui".to_string()));
        assert!(tui.contains(&"interface.tui".to_string()));
        assert!(!tui.contains(&"interface.tauri".to_string()));
    }

    #[test]
    fn dangerous_capabilities_are_present_but_disabled_by_default() {
        let catalog = shipped_catalog();
        let rows = catalog.compose("desktop", &[]).unwrap();
        for id in ["tools.write", "tools.run", "computer.use"] {
            let row = rows.iter().find(|r| r.id == id).expect(id);
            assert!(!row.enabled, "{id} must ship disabled");
        }
        // …and they are therefore absent from the load order.
        let ids = catalog.load_order("desktop", &[]).unwrap();
        assert!(!ids.iter().any(|i| i == "tools.run"));
    }

    #[test]
    fn a_minimal_profile_is_actually_minimal() {
        let catalog = shipped_catalog();
        let ids = catalog.load_order("minimal", &[]).unwrap();
        assert_eq!(ids.len(), 6);
        assert!(!ids.iter().any(|i| i.starts_with("interface.")));
    }

    #[test]
    fn a_patch_can_drop_a_bundle_row_without_forking_it() {
        let catalog = shipped_catalog();
        let overlay = Patch::dropping(&["provenance.gate"]);
        let ids = catalog.load_order("desktop", &[overlay]).unwrap();
        assert!(!ids.iter().any(|i| i == "provenance.gate"));
        // The shipped profile is untouched: patches are not mutations.
        let fresh = catalog.load_order("desktop", &[]).unwrap();
        assert!(fresh.iter().any(|i| i == "provenance.gate"));
    }

    #[test]
    fn a_patch_can_enable_a_disabled_row() {
        let catalog = shipped_catalog();
        let mut row = PluginRow::new("tools.run");
        row.config.insert("allow".into(), "cargo".into());
        let overlay = Patch::adding(vec![row]);
        let ids = catalog.load_order("desktop", &[overlay]).unwrap();
        assert!(ids.iter().any(|i| i == "tools.run"));
    }

    #[test]
    fn later_layers_win_over_earlier_ones() {
        let catalog = shipped_catalog();
        let first = Patch::adding(vec![PluginRow::disabled("tools.run")]);
        let second = Patch::adding(vec![PluginRow::new("tools.run")]);
        let ids = catalog.load_order("desktop", &[first, second]).unwrap();
        assert!(
            ids.iter().any(|i| i == "tools.run"),
            "the last layer decides"
        );
    }

    #[test]
    fn a_missing_bundle_is_reported_not_skipped() {
        let mut catalog = shipped_catalog();
        catalog.add_profile(Profile::new("broken", &["core", "does-not-exist"]));
        match catalog.compose("broken", &[]) {
            Err(ComposeError::UnknownBundle(name)) => assert_eq!(name, "does-not-exist"),
            other => panic!("expected UnknownBundle, got {other:?}"),
        }
    }

    #[test]
    fn a_duplicate_plugin_row_is_refused() {
        let mut catalog = Catalog::new();
        catalog.add_bundle(Bundle::new(
            "one",
            vec![
                PluginRow::new("store.sqlite"),
                PluginRow::new("store.sqlite"),
            ],
        ));
        catalog.add_profile(Profile::new("p", &["one"]));
        assert_eq!(
            catalog.compose("p", &[]),
            Err(ComposeError::DuplicatePluginId("store.sqlite".into()))
        );
    }

    #[test]
    fn an_unknown_profile_is_reported() {
        let catalog = shipped_catalog();
        assert_eq!(
            catalog.compose("nope", &[]),
            Err(ComposeError::UnknownProfile("nope".into()))
        );
    }

    #[test]
    fn composition_order_follows_the_bundle_list() {
        let mut catalog = Catalog::new();
        catalog.add_bundle(Bundle::new("a", vec![PluginRow::new("first")]));
        catalog.add_bundle(Bundle::new("b", vec![PluginRow::new("second")]));
        catalog.add_profile(Profile::new("p", &["b", "a"]));
        let rows = catalog.compose("p", &[]).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["second", "first"]
        );
    }

    #[test]
    fn the_catalog_is_data_and_can_be_dumped() {
        let catalog = shipped_catalog();
        let json = serde_json::to_string_pretty(&catalog.profiles).unwrap();
        assert!(json.contains("desktop"));
        assert!(json.contains("headless"));
        // The names a UI would show are all present.
        assert!(catalog.profiles().contains(&"desktop"));
        assert!(catalog.profiles().contains(&"tui"));
        assert!(catalog.profiles().contains(&"headless"));
        assert!(catalog.profiles().contains(&"minimal"));
    }
}
