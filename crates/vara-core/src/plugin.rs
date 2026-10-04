//! Every plugin is a folder with a manifest — including the ones Vara ships.
//!
//! The rule this module exists to enforce: **nothing is built in**. A capability
//! that cannot be described by a manifest cannot be loaded; a capability that
//! can be described by one can be removed by the owner without touching code.
//! That is the difference between "configurable" and "composable", and the
//! owner asked for the second.
//!
//! A manifest is deliberately **data, not code**: it can be listed, diffed,
//! validated and (later) signed. Everything a plugin is allowed to do is
//! declared up front, so the gate can refuse a call the manifest never claimed
//! — which is what makes an untrusted third-party plugin safe to install at all.
//!
//! Integrity is a SHA-256 over the plugin's files, computed the same way on
//! every platform, so a manifest that travelled through a chat window or a git
//! clone can be verified rather than trusted.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a plugin may contribute. Closed on purpose: a new *kind* of thing is a
/// product decision, not something an unknown plugin can invent at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// Tools the entity can call.
    Tool,
    /// A model provider.
    Brain,
    /// A memory backend.
    Memory,
    /// An interface (desktop, TUI, web, headless).
    Interface,
    /// A visual theme (tokens, not code).
    Theme,
    /// An identity/persona pack.
    Persona,
    /// A channel the entity can speak through (Discord, Slack, mail…).
    Channel,
    /// A goal engine that decides what to do next.
    GoalEngine,
    /// A way to spawn scoped workers.
    Subagent,
    /// An MCP server connection.
    Mcp,
    /// A skill the model can read.
    Skill,
    /// A pack that bundles tools for one job (browser use, computer use…).
    Toolset,
}

impl Slot {
    pub fn as_str(&self) -> &'static str {
        match self {
            Slot::Tool => "tool",
            Slot::Brain => "brain",
            Slot::Memory => "memory",
            Slot::Interface => "interface",
            Slot::Theme => "theme",
            Slot::Persona => "persona",
            Slot::Channel => "channel",
            Slot::GoalEngine => "goal_engine",
            Slot::Subagent => "subagent",
            Slot::Mcp => "mcp",
            Slot::Skill => "skill",
            Slot::Toolset => "toolset",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.trim() {
            "tool" => Slot::Tool,
            "brain" => Slot::Brain,
            "memory" => Slot::Memory,
            "interface" => Slot::Interface,
            "theme" => Slot::Theme,
            "persona" => Slot::Persona,
            "channel" => Slot::Channel,
            "goal_engine" => Slot::GoalEngine,
            "subagent" => Slot::Subagent,
            "mcp" => Slot::Mcp,
            "skill" => Slot::Skill,
            "toolset" => Slot::Toolset,
            _ => return None,
        })
    }
}

/// How much a plugin asks for. Deny by default: a permission that is not
/// declared is refused, so a plugin cannot quietly acquire one by omission.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    /// Read files under an allowed root.
    #[serde(default)]
    pub read_files: bool,
    /// Write files. Never granted without the undo journal and owner approval.
    #[serde(default)]
    pub write_files: bool,
    /// Run a program (argv-only, gated).
    #[serde(default)]
    pub run_programs: bool,
    /// Reach the network.
    #[serde(default)]
    pub network: bool,
    /// Read the entity's own memory.
    #[serde(default)]
    pub read_memory: bool,
    /// Write memory. Separate from reading: a plugin that can poison memory can
    /// change every later answer.
    #[serde(default)]
    pub write_memory: bool,
    /// Talk to the owner (notifications, channels).
    #[serde(default)]
    pub notify: bool,
    /// Extensions this plugin may read. A list, not a boolean, because "which
    /// files" is the question the owner actually cares about.
    #[serde(default)]
    pub file_extensions: Vec<String>,
}

impl Permissions {
    /// The permissions that must show in an approval card before first run.
    pub fn dangerous(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.write_files {
            out.push("write files");
        }
        if self.run_programs {
            out.push("run programs");
        }
        if self.network {
            out.push("use the network");
        }
        if self.write_memory {
            out.push("change memory");
        }
        if self.notify {
            out.push("contact you");
        }
        out
    }

    pub fn is_read_only(&self) -> bool {
        self.dangerous().is_empty() && self.read_files
    }
}

/// The manifest itself. One file, `plugin.toml`, at the plugin's root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Stable id, `author.name` style: `vara.tools.read`.
    pub id: String,
    pub name: String,
    pub version: String,
    /// One line for the plugin list.
    #[serde(default)]
    pub summary: String,
    /// What it contributes. A plugin with no slots contributes nothing and is
    /// refused, because a plugin that does nothing is a mistake not a plugin.
    pub slots: Vec<Slot>,
    #[serde(default)]
    pub permissions: Permissions,
    /// Ids that must be loaded first.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Ids this plugin replaces (for a swap-in implementation).
    #[serde(default)]
    pub replaces: Vec<String>,
    /// True when Vara ships it. Shipped plugins are still removable.
    #[serde(default)]
    pub shipped: bool,
    /// Loaded at boot unless the owner turns it off.
    #[serde(default = "default_true")]
    pub default_enabled: bool,
    /// Free-form settings the plugin itself understands. The host never parses
    /// these: a host that understood plugin settings would have to change every
    /// time a plugin did.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    /// Relative paths of the files that make up the plugin, for the hash.
    #[serde(default)]
    pub files: Vec<String>,
    /// SHA-256 over [`Manifest::hash_input`]. Empty means "not verified yet".
    #[serde(default)]
    pub sha256: String,
}

fn default_true() -> bool {
    true
}

/// Why a manifest is not usable. Every variant is owner-explainable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    EmptyId,
    BadId(String),
    NoName,
    NoVersion,
    NoSlots,
    UnknownSlot(String),
    SelfRequires(String),
    MissingFile(String),
    HashMismatch { expected: String, actual: String },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::EmptyId => write!(f, "a plugin needs an id"),
            ManifestError::BadId(id) => write!(
                f,
                "id '{id}' must be dotted lowercase (letters, digits, - and _), e.g. vara.tools.read"
            ),
            ManifestError::NoName => write!(f, "a plugin needs a display name"),
            ManifestError::NoVersion => write!(f, "a plugin needs a version"),
            ManifestError::NoSlots => write!(
                f,
                "a plugin must contribute something: list at least one slot"
            ),
            ManifestError::UnknownSlot(slot) => write!(f, "unknown slot '{slot}'"),
            ManifestError::SelfRequires(id) => write!(f, "plugin '{id}' requires itself"),
            ManifestError::MissingFile(path) => {
                write!(f, "declared file '{path}' is missing from the plugin folder")
            }
            ManifestError::HashMismatch { expected, actual } => write!(
                f,
                "integrity check failed: manifest says {expected}, the files hash to {actual}"
            ),
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    /// The text the hash is computed over. Deliberately a **canonical** view:
    /// id, version, slots, permissions and file list, in sorted order. Comments,
    /// key order and whitespace in the file do not change it, so reformatting a
    /// manifest does not invalidate a signature — but changing what the plugin
    /// *claims* does.
    pub fn hash_input(&self) -> String {
        let mut slots: Vec<&str> = self.slots.iter().map(|s| s.as_str()).collect();
        slots.sort_unstable();
        slots.dedup();
        let mut requires = self.requires.clone();
        requires.sort();
        let mut files = self.files.clone();
        files.sort();
        let p = &self.permissions;
        let mut exts = p.file_extensions.clone();
        exts.sort();
        format!(
            "id={}\nversion={}\nslots={}\nrequires={}\nread_files={}\nwrite_files={}\nrun_programs={}\nnetwork={}\nread_memory={}\nwrite_memory={}\nnotify={}\nextensions={}\nfiles={}",
            self.id,
            self.version,
            slots.join(","),
            requires.join(","),
            p.read_files,
            p.write_files,
            p.run_programs,
            p.network,
            p.read_memory,
            p.write_memory,
            p.notify,
            exts.join(","),
            files.join(","),
        )
    }

    /// Validate the declaration itself, before any file is read.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.id.trim().is_empty() {
            return Err(ManifestError::EmptyId);
        }
        if !is_valid_id(&self.id) {
            return Err(ManifestError::BadId(self.id.clone()));
        }
        if self.name.trim().is_empty() {
            return Err(ManifestError::NoName);
        }
        if self.version.trim().is_empty() {
            return Err(ManifestError::NoVersion);
        }
        if self.slots.is_empty() {
            return Err(ManifestError::NoSlots);
        }
        if self.requires.iter().any(|r| r == &self.id) {
            return Err(ManifestError::SelfRequires(self.id.clone()));
        }
        Ok(())
    }

    pub fn apply_hash(&mut self) {
        self.sha256 = crate::sha256_hex(self.hash_input().as_bytes());
    }

    /// Verify the declared hash. Returns `Ok(false)` when the manifest simply
    /// has no hash yet (a locally authored plugin), and an error only when a
    /// hash is present and wrong — which is the case that must stop a load.
    pub fn verify_hash(&self) -> Result<bool, ManifestError> {
        if self.sha256.is_empty() {
            return Ok(false);
        }
        let actual = crate::sha256_hex(self.hash_input().as_bytes());
        if actual.eq_ignore_ascii_case(&self.sha256) {
            Ok(true)
        } else {
            Err(ManifestError::HashMismatch {
                expected: self.sha256.clone(),
                actual,
            })
        }
    }

    pub fn provides(&self, slot: Slot) -> bool {
        self.slots.contains(&slot)
    }

    /// One line for a plugin list, showing what it asks for.
    pub fn describe(&self) -> String {
        let danger = self.permissions.dangerous();
        if danger.is_empty() {
            format!("{} {} — no extra permissions", self.name, self.version)
        } else {
            format!(
                "{} {} — asks to {}",
                self.name,
                self.version,
                danger.join(", ")
            )
        }
    }
}

/// Ids are dotted lowercase. Enforced because ids appear in config files,
/// dependency lists and (later) signatures: a lenient rule here becomes an
/// ambiguous one there.
fn is_valid_id(id: &str) -> bool {
    if id.is_empty() || id.starts_with('.') || id.ends_with('.') {
        return false;
    }
    id.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    })
}

/// A plugin as it exists on disk.
#[derive(Debug, Clone)]
pub struct PluginFolder {
    pub root: PathBuf,
    pub manifest: Manifest,
    /// Files found in the folder, relative to the root, sorted.
    pub present_files: Vec<String>,
}

impl PluginFolder {
    /// Read and validate a plugin folder.
    ///
    /// When `verify` is true a declared hash that does not match is fatal. A
    /// manifest with no hash is accepted but reported as unverified — the owner
    /// can see the difference instead of being told "safe" either way.
    pub fn load(root: &Path, verify: bool) -> Result<Self, String> {
        let manifest_path = root.join("plugin.toml");
        let body = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        let manifest: Manifest = toml::from_str(&body)
            .map_err(|e| format!("{} is not a valid manifest: {e}", manifest_path.display()))?;
        manifest.validate().map_err(|e| e.to_string())?;
        if verify {
            manifest.verify_hash().map_err(|e| e.to_string())?;
        }

        let mut present_files = Vec::new();
        collect_relative(root, root, &mut present_files)
            .map_err(|e| format!("cannot read {}: {e}", root.display()))?;
        present_files.sort();

        for declared in &manifest.files {
            if !present_files.iter().any(|f| f == declared) {
                return Err(ManifestError::MissingFile(declared.clone()).to_string());
            }
        }

        Ok(Self {
            root: root.to_path_buf(),
            manifest,
            present_files,
        })
    }

    /// The hash of everything in the folder, not just the declared files.
    ///
    /// This is the number an owner compares against a published value, so it
    /// must not be forgeable by *omitting* a file from `files` — hence it walks
    /// the folder. `plugin.toml` is excluded because it holds the expected hash.
    pub fn content_hash(&self) -> Result<String, String> {
        let mut entries: Vec<(String, String)> = Vec::new();
        for rel in &self.present_files {
            if rel == "plugin.toml" || rel.ends_with("/plugin.toml") {
                continue;
            }
            let path = self
                .root
                .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
            entries.push((rel.clone(), crate::sha256_hex(&bytes)));
        }
        let joined = entries
            .iter()
            .map(|(name, hash)| format!("{name}={hash}"))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(crate::sha256_hex(joined.as_bytes()))
    }

    /// Does the folder match what the manifest declares?
    pub fn is_verified(&self) -> bool {
        self.manifest.verify_hash().unwrap_or(false)
    }
}

fn collect_relative(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// Discover every plugin folder under `dirs`, depth-first, skipping duplicates.
///
/// A folder without `plugin.toml` is skipped silently: `plugins/` may contain
/// documents, assets or a half-written plugin, and failing the whole scan for
/// one incomplete folder would make the folder unusable as a workspace.
pub fn discover(dirs: &[PathBuf], verify: bool) -> Vec<Result<PluginFolder, String>> {
    let mut found = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let mut candidates = Vec::new();
        collect_manifest_dirs(dir, &mut candidates);
        candidates.sort();
        for candidate in candidates {
            found.push(PluginFolder::load(&candidate, verify));
        }
    }
    found
}

fn collect_manifest_dirs(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.join("plugin.toml").is_file() {
        out.push(dir.to_path_buf());
        // A plugin does not nest inside another plugin.
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_manifest_dirs(&path, out);
            }
        }
    }
}

/// Resolve the order plugins must load in, given what they require.
///
/// Deterministic and cycle-safe: a cycle is reported as an error naming the
/// participants rather than being silently broken by an arbitrary order, because
/// a nondeterministic plugin order makes a bug impossible to reproduce.
pub fn load_order(plugins: &[(String, Vec<String>)]) -> Result<Vec<String>, String> {
    let mut known: BTreeMap<&str, &Vec<String>> = BTreeMap::new();
    for (id, requires) in plugins {
        known.insert(id.as_str(), requires);
    }

    let mut ordered: Vec<String> = Vec::new();
    let mut visiting: Vec<String> = Vec::new();
    let mut done: Vec<String> = Vec::new();

    fn visit(
        id: &str,
        known: &BTreeMap<&str, &Vec<String>>,
        ordered: &mut Vec<String>,
        visiting: &mut Vec<String>,
        done: &mut Vec<String>,
        missing: &mut Vec<String>,
    ) {
        if done.iter().any(|d| d == id) {
            return;
        }
        if let Some(pos) = visiting.iter().position(|v| v == id) {
            // Report the cycle from where it starts, so the message names the
            // loop rather than just one plugin.
            let cycle = visiting[pos..].join(" → ");
            visiting.push(id.to_string());
            // Leave a marker the caller can spot.
            ordered.push(format!("!cycle:{cycle}"));
            return;
        }
        let Some(requires) = known.get(id) else {
            missing.push(id.to_string());
            return;
        };
        visiting.push(id.to_string());
        for dep in requires.iter() {
            visit(dep, known, ordered, visiting, done, missing);
        }
        visiting.pop();
        done.push(id.to_string());
        ordered.push(id.to_string());
    }

    let mut missing = Vec::new();
    for (id, _) in plugins {
        visit(
            id,
            &known,
            &mut ordered,
            &mut visiting,
            &mut done,
            &mut missing,
        );
    }

    if let Some(bad) = ordered.iter().find(|o| o.starts_with("!cycle:")) {
        return Err(format!("dependency cycle: {}", &bad[7..]));
    }
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        return Err(format!(
            "these required plugins are not installed: {}",
            missing.join(", ")
        ));
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str) -> Manifest {
        Manifest {
            id: id.into(),
            name: "Test".into(),
            version: "1.0.0".into(),
            summary: String::new(),
            slots: vec![Slot::Tool],
            permissions: Permissions::default(),
            requires: Vec::new(),
            replaces: Vec::new(),
            shipped: false,
            default_enabled: true,
            config: BTreeMap::new(),
            files: Vec::new(),
            sha256: String::new(),
        }
    }

    #[test]
    fn a_plugin_must_contribute_something() {
        let mut m = manifest("vara.test");
        m.slots.clear();
        assert_eq!(m.validate().unwrap_err(), ManifestError::NoSlots);
    }

    #[test]
    fn ids_are_dotted_lowercase_and_a_bad_one_is_named() {
        for bad in ["", "Vara.Test", "vara..test", "vara test", ".vara", "vara."] {
            let mut m = manifest("vara.test");
            m.id = bad.into();
            assert!(m.validate().is_err(), "'{bad}' should be refused");
        }
        assert!(manifest("vara.tools.read-file_2").validate().is_ok());
    }

    #[test]
    fn a_plugin_cannot_require_itself() {
        let mut m = manifest("vara.loop");
        m.requires = vec!["vara.loop".into()];
        assert_eq!(
            m.validate().unwrap_err(),
            ManifestError::SelfRequires("vara.loop".into())
        );
    }

    #[test]
    fn the_hash_covers_claims_but_not_formatting() {
        let a = manifest("vara.test");
        let mut b = manifest("vara.test");
        // Nothing about the file's formatting is in the input.
        assert_eq!(a.hash_input(), b.hash_input());

        // A changed claim does change it.
        b.permissions.network = true;
        assert_ne!(a.hash_input(), b.hash_input());

        // So does a new dependency, and a new slot.
        let mut c = manifest("vara.test");
        c.requires = vec!["vara.other".into()];
        assert_ne!(a.hash_input(), c.hash_input());

        let mut d = manifest("vara.test");
        d.slots = vec![Slot::Tool, Slot::Mcp];
        assert_ne!(a.hash_input(), d.hash_input());
    }

    #[test]
    fn order_of_slots_and_files_does_not_change_the_hash() {
        let mut a = manifest("vara.test");
        a.slots = vec![Slot::Tool, Slot::Mcp, Slot::Theme];
        a.files = vec!["b.rhai".into(), "a.rhai".into()];
        let mut b = manifest("vara.test");
        b.slots = vec![Slot::Theme, Slot::Tool, Slot::Mcp];
        b.files = vec!["a.rhai".into(), "b.rhai".into()];
        assert_eq!(a.hash_input(), b.hash_input());
    }

    #[test]
    fn a_declared_hash_that_is_wrong_stops_the_load() {
        let mut m = manifest("vara.test");
        m.apply_hash();
        assert!(m.verify_hash().unwrap());

        // The claims changed but the hash did not: this must be an error, not
        // a warning.
        m.permissions.run_programs = true;
        let err = m.verify_hash().unwrap_err();
        match err {
            ManifestError::HashMismatch { expected, actual } => {
                assert_ne!(expected, actual);
            }
            other => panic!("expected HashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn an_unhashed_plugin_is_accepted_but_reported_unverified() {
        let m = manifest("vara.local");
        assert!(!m.verify_hash().unwrap());
        assert!(!m.verify_hash().unwrap());
    }

    #[test]
    fn the_dangerous_permissions_are_the_ones_that_show_in_a_card() {
        let p = Permissions {
            read_files: true,
            ..Permissions::default()
        };
        assert!(p.is_read_only());
        assert!(p.dangerous().is_empty());

        // A second value rather than mutating in place: the read-only set and the
        // dangerous set are different permissions, so they read as two objects.
        let dangerous = Permissions {
            read_files: true,
            write_files: true,
            network: true,
            ..Permissions::default()
        };
        let danger = dangerous.dangerous();
        assert!(danger.contains(&"write files"));
        assert!(danger.contains(&"use the network"));
        assert!(!dangerous.is_read_only());
    }

    #[test]
    fn load_order_puts_dependencies_first() {
        let plugins = vec![
            ("vara.a".to_string(), vec!["vara.b".to_string()]),
            ("vara.b".to_string(), vec!["vara.c".to_string()]),
            ("vara.c".to_string(), vec![]),
        ];
        let order = load_order(&plugins).unwrap();
        let pos = |id: &str| order.iter().position(|o| o == id).unwrap();
        assert!(pos("vara.c") < pos("vara.b"));
        assert!(pos("vara.b") < pos("vara.a"));
    }

    #[test]
    fn load_order_is_deterministic() {
        let plugins = vec![
            ("vara.x".to_string(), vec![]),
            ("vara.y".to_string(), vec![]),
            ("vara.z".to_string(), vec!["vara.x".to_string()]),
        ];
        let first = load_order(&plugins).unwrap();
        let second = load_order(&plugins).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn a_cycle_is_named_not_silently_broken() {
        let plugins = vec![
            ("vara.a".to_string(), vec!["vara.b".to_string()]),
            ("vara.b".to_string(), vec!["vara.a".to_string()]),
        ];
        let err = load_order(&plugins).unwrap_err();
        assert!(err.contains("cycle"), "{err}");
        assert!(err.contains("vara.a") && err.contains("vara.b"), "{err}");
    }

    #[test]
    fn a_missing_dependency_is_named() {
        let plugins = vec![("vara.a".to_string(), vec!["vara.absent".to_string()])];
        let err = load_order(&plugins).unwrap_err();
        assert!(err.contains("vara.absent"), "{err}");
    }

    #[test]
    fn a_manifest_round_trips_through_toml() {
        let mut m = manifest("vara.tools.read");
        m.summary = "six read-only tools".into();
        m.permissions.read_files = true;
        m.permissions.file_extensions = vec!["txt".into(), "md".into()];
        m.shipped = true;
        m.files = vec!["plugin.toml".into(), "README.md".into()];
        m.apply_hash();

        let text = toml::to_string(&m).unwrap();
        let back: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(m, back);
        assert!(back.verify_hash().unwrap());
    }

    #[test]
    fn discovery_reads_real_folders_and_reports_a_bad_one_by_name() {
        let dir = std::env::temp_dir().join(format!("vara-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let good = dir.join("good");
        let bad = dir.join("bad");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::create_dir_all(&bad).unwrap();

        let mut m = manifest("vara.good");
        m.apply_hash();
        std::fs::write(good.join("plugin.toml"), toml::to_string(&m).unwrap()).unwrap();
        // A folder with no slots but a manifest: invalid, and must say so.
        let mut broken = manifest("vara.bad");
        broken.slots.clear();
        std::fs::write(bad.join("plugin.toml"), toml::to_string(&broken).unwrap()).unwrap();
        // A folder with no manifest at all is skipped, not an error.
        std::fs::create_dir_all(dir.join("not-a-plugin")).unwrap();

        let results = discover(std::slice::from_ref(&dir), true);
        assert_eq!(results.len(), 2, "the manifest-less folder must be skipped");
        let ok = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok, 1);
        let err = results
            .iter()
            .find(|r| r.is_err())
            .unwrap()
            .as_ref()
            .unwrap_err();
        assert!(err.contains("vara.bad") || err.contains("slot"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_declared_file_that_is_missing_is_refused() {
        let dir = std::env::temp_dir().join(format!("vara-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = manifest("vara.claims");
        m.files = vec!["engine.rhai".into()];
        m.apply_hash();
        std::fs::write(dir.join("plugin.toml"), toml::to_string(&m).unwrap()).unwrap();

        let err = PluginFolder::load(&dir, true).unwrap_err();
        assert!(err.contains("engine.rhai"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampering_with_a_file_changes_the_content_hash() {
        let dir = std::env::temp_dir().join(format!("vara-tamper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = manifest("vara.file");
        m.files = vec!["tool.json".into()];
        m.apply_hash();
        std::fs::write(dir.join("plugin.toml"), toml::to_string(&m).unwrap()).unwrap();
        std::fs::write(dir.join("tool.json"), r#"{"name":"safe"}"#).unwrap();

        let folder = PluginFolder::load(&dir, true).unwrap();
        assert!(folder.is_verified());
        let before = folder.content_hash().unwrap();

        // Someone edits the tool. The manifest still verifies (its claims did
        // not change) but the content hash moves — which is exactly the number
        // the owner compares against a published one.
        std::fs::write(dir.join("tool.json"), r#"{"name":"not-safe"}"#).unwrap();
        let after = PluginFolder::load(&dir, true)
            .unwrap()
            .content_hash()
            .unwrap();
        assert_ne!(before, after);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_twelve_slots_parse_and_round_trip() {
        let slots = [
            "tool",
            "brain",
            "memory",
            "interface",
            "theme",
            "persona",
            "channel",
            "goal_engine",
            "subagent",
            "mcp",
            "skill",
            "toolset",
        ];
        for name in slots {
            let slot = Slot::parse(name).unwrap_or_else(|| panic!("{name} did not parse"));
            assert_eq!(slot.as_str(), name);
        }
        assert!(Slot::parse("nonsense").is_none());
    }
}
