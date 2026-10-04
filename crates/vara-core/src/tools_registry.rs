//! The tool registry — how the entity touches this machine, and the only way it does.
//!
//! Vara's promise is that the model *proposes* and the shell *decides*. Until
//! now "propose" meant a fixed three-step plan (`search`/`fetch`/`report`) plus
//! free-text `[[sys]]` markers, which is why a request as ordinary as "explain
//! my system" ended in a refused shell command and a dead conversation (gap
//! G-02 in the audit).
//!
//! This module is the typed contract that replaces that: every capability is a
//! [`Tool`] with a name, a JSON schema, a risk class, output caps and a timeout.
//! The model calls tools by name with arguments; the policy layer decides
//! whether a call may run; the journal records what actually happened.
//!
//! Design rules (mirroring `computer_use`, deliberately):
//!
//! 1. **Validate-then-execute.** Unknown tool, unknown argument, wrong type or
//!    missing field is refused *before* anything touches the machine.
//! 2. **Errors are for the model.** A refusal or a bad argument is a typed
//!    value that renders as one actionable sentence, so the model can repair
//!    its own call instead of guessing. "Something went wrong" is a bug.
//! 3. **Output is summarised, never dumped.** A listing returns a count, a few
//!    entries and how to narrow — the measured lesson from agent-interface
//!    research that raw output actively hurts.
//! 4. **The registry is data.** Adding a capability must not add a branch to
//!    the agent loop.
//! 5. **No tool reaches the forbidden set**: credential stores, browser
//!    password databases, SSH/API key files, Vara's own settings/policy/updater
//!    records, or the workspace integrity hashes. That floor is in Rust.

use crate::exec_policy::{confine_to_root, is_secret_env_key};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Risk classes from `docs/tasks/ALIVE_V2_SPEC.md` §7.2. The class is what the
/// policy middleware reasons about; the model never chooses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    /// R — read-only.
    Read,
    /// Wr — write, reversible (journaled / Recycle Bin).
    WriteReversible,
    /// Wd — write, destructive (permanent delete / overwrite).
    WriteDestructive,
    /// X — run a program.
    Execute,
    /// N — network out / open a URL.
    Network,
    /// H — hand-off: secrets, payments, policy changes. Never automated.
    HandOff,
}

impl RiskClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskClass::Read => "R",
            RiskClass::WriteReversible => "Wr",
            RiskClass::WriteDestructive => "Wd",
            RiskClass::Execute => "X",
            RiskClass::Network => "N",
            RiskClass::HandOff => "H",
        }
    }
}

/// Why a call was refused. Every variant becomes a sentence the model can act
/// on, which is the difference between a dead end and a repaired call.
///
/// Deserialisation is deliberately not derived: a `&'static str` field would
/// force a borrowed lifetime on the whole enum, and these values are produced
/// in-process — the wire format only ever needs the rendering (`message()`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolError {
    /// No such tool. The message lists the closest names.
    UnknownTool { name: String, suggestion: String },
    /// A required field is missing.
    MissingField { field: String },
    /// A field has the wrong shape.
    BadType {
        field: String,
        expected: &'static str,
    },
    /// A value is outside what the tool accepts.
    BadValue { field: String, why: String },
    /// The path is outside the roots the owner allowed.
    OutOfRoots { path: String },
    /// The path is in the hard-deny floor.
    Forbidden { path: String, why: &'static str },
    /// The policy middleware said no (class not allowed at this level).
    Denied { class: RiskClass, why: String },
    /// The tool ran and failed for an environmental reason.
    Failed { why: String },
    /// Output was too large to return and the tool refused to guess.
    TooLarge { bytes: usize, cap: usize },
}

impl ToolError {
    /// The single sentence the model sees. Kept short on purpose: the loop
    /// budget is small and the model reads better without a stack trace.
    pub fn message(&self) -> String {
        match self {
            ToolError::UnknownTool { name, suggestion } => format!(
                "no tool named '{name}'.{}",
                if suggestion.is_empty() {
                    String::new()
                } else {
                    format!(" Did you mean: {suggestion}?")
                }
            ),
            ToolError::MissingField { field } => format!("missing required argument '{field}'"),
            ToolError::BadType { field, expected } => {
                format!("argument '{field}' must be {expected}")
            }
            ToolError::BadValue { field, why } => format!("argument '{field}' is invalid: {why}"),
            ToolError::OutOfRoots { path } => format!(
                "'{path}' is outside the folders the owner allowed. Ask the owner to add it in Settings."
            ),
            ToolError::Forbidden { path, why } => format!("'{path}' is never accessible: {why}"),
            ToolError::Denied { class, why } => {
                format!("this action ({}) needs approval: {why}", class.as_str())
            }
            ToolError::Failed { why } => format!("the tool failed: {why}"),
            ToolError::TooLarge { bytes, cap } => format!(
                "the result is {bytes} bytes (limit {cap}). Narrow the request (a smaller folder, a pattern, fewer entries)."
            ),
        }
    }

    /// A refusal the owner should see on the approval card / activity tree.
    pub fn is_policy(&self) -> bool {
        matches!(
            self,
            ToolError::Denied { .. } | ToolError::OutOfRoots { .. } | ToolError::Forbidden { .. }
        )
    }
}

/// What a tool produced.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolResult {
    pub ok: bool,
    /// Bounded, model-readable summary. Never a raw dump.
    pub summary: String,
    /// Structured payload for the UI/receipts (already capped).
    #[serde(default)]
    pub data: Value,
    /// Rows/entries the tool saw before summarising, when meaningful.
    #[serde(default)]
    pub total: Option<usize>,
    #[serde(default)]
    pub truncated: bool,
    /// Set when the tool failed, so the caller can render one sentence.
    #[serde(default)]
    pub error: Option<ToolError>,
}

impl ToolResult {
    pub fn ok(summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            summary: summary.into(),
            data: Value::Null,
            total: None,
            truncated: false,
            error: None,
        }
    }
    pub fn ok_with(summary: impl Into<String>, data: Value, total: usize, truncated: bool) -> Self {
        Self {
            ok: true,
            summary: summary.into(),
            data,
            total: Some(total),
            truncated,
            error: None,
        }
    }
    pub fn fail(error: ToolError) -> Self {
        Self {
            ok: false,
            summary: error.message(),
            data: Value::Null,
            total: None,
            truncated: false,
            error: Some(error),
        }
    }
}

/// Where a tool may look. Resolved by the caller once, from the owner's
/// settings — a tool never decides its own scope.
#[derive(Debug, Clone, Default)]
pub struct Roots {
    allowed: Vec<PathBuf>,
}

impl Roots {
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            allowed: paths
                .into_iter()
                .filter(|p| p.is_absolute())
                .collect::<Vec<_>>(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.allowed
    }

    /// The root that contains `path`, if any.
    pub fn containing(&self, path: &Path) -> Option<&PathBuf> {
        self.allowed
            .iter()
            .find(|root| confine_to_root(root, &path.to_string_lossy()).is_ok())
    }
}

/// Everything a tool is allowed to know about the world.
///
/// Note what is *absent*: no ambient `std::env`, no global clock, no
/// filesystem handle. A tool that needs the network or the clock is handed it
/// explicitly, which is what makes the harness deterministic.
#[derive(Clone)]
pub struct ToolCtx {
    pub roots: Roots,
    /// Unix seconds, supplied by the caller (never read inside a tool).
    pub now_unix: i64,
    /// Names that must never be read, regardless of roots.
    pub denied_paths: Vec<PathBuf>,
    /// Vara's own memory, when the caller can offer it. A trait object keeps
    /// the tool layer free of any SQLite dependency.
    pub memory: Option<std::sync::Arc<dyn crate::tools_local::MemoryProvider>>,
}

impl Default for ToolCtx {
    fn default() -> Self {
        Self {
            roots: Roots::default(),
            now_unix: 0,
            denied_paths: Vec::new(),
            memory: None,
        }
    }
}

/// Manual `Debug`: the memory handle is a trait object, and a tool context
/// should never dump memory contents into a log line anyway.
impl std::fmt::Debug for ToolCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCtx")
            .field("roots", &self.roots)
            .field("now_unix", &self.now_unix)
            .field("denied_paths", &self.denied_paths)
            .field("memory", &self.memory.as_ref().map(|_| "<provider>"))
            .finish()
    }
}

/// Sentence-fragment names that are always off-limits.
pub const FORBIDDEN_PATH_MARKERS: &[&str] = &[
    ".ssh",
    ".aws",
    "id_rsa",
    "id_ed25519",
    "credentials.json",
    "login data",
    "keychain",
    ".netrc",
    ".npmrc",
    ".git-credentials",
    "settings.json",
    "vara.db",
    "integrity.json",
];

impl ToolCtx {
    /// The hard-deny floor. Runs before any tool logic, on the raw argument
    /// text, so no tool can forget it.
    pub fn guard_path(&self, raw: &str) -> Result<(), ToolError> {
        let lowered = raw.to_lowercase();
        for marker in FORBIDDEN_PATH_MARKERS {
            if lowered.contains(marker) {
                return Err(ToolError::Forbidden {
                    path: raw.to_string(),
                    why: "it is a credential or Vara's own state",
                });
            }
        }
        if is_secret_env_key(&lowered) {
            return Err(ToolError::Forbidden {
                path: raw.to_string(),
                why: "it names a secret",
            });
        }
        Ok(())
    }

    /// Resolve a caller-supplied path inside the allowed roots.
    pub fn resolve(&self, raw: &str) -> Result<PathBuf, ToolError> {
        self.guard_path(raw)?;
        if self.roots.is_empty() {
            return Err(ToolError::OutOfRoots {
                path: raw.to_string(),
            });
        }
        // A bare name is interpreted relative to each root in turn; the first
        // root that contains it wins. Absolute paths must already be inside one.
        let candidate = Path::new(raw);
        if candidate.is_absolute() {
            let root = self
                .roots
                .containing(candidate)
                .ok_or_else(|| ToolError::OutOfRoots {
                    path: raw.to_string(),
                })?;
            return confine_to_root(root, raw).map_err(|_| ToolError::OutOfRoots {
                path: raw.to_string(),
            });
        }
        for root in self.roots.paths() {
            let joined = root.join(candidate);
            if let Ok(resolved) = confine_to_root(root, &joined.to_string_lossy()) {
                return Ok(resolved);
            }
        }
        Err(ToolError::OutOfRoots {
            path: raw.to_string(),
        })
    }
}

/// The description of a tool: what the model sees, and what the policy reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments object.
    pub params: Value,
    pub class: RiskClass,
    pub max_output_bytes: usize,
    pub timeout_ms: u64,
    /// A valid example call, shown to the model (a demo is worth measurable
    /// accuracy in agent benchmarks).
    pub example: Value,
}

/// A capability. Implementations must be **pure with respect to the outside
/// world except through [`ToolHost`]**, so the harness can run them headless.
///
/// `Send + Sync` is required rather than incidental: the registry is built
/// inside async Tauri command handlers, whose futures must be `Send`. A tool
/// holding thread-affine state would make the whole shell uncompilable.
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;

    /// Structural validation only — no I/O, no clock, no environment.
    fn validate(&self, args: &Value, ctx: &ToolCtx) -> Result<(), ToolError>;

    /// The effectful half. Only ever called after `validate` (and, for
    /// non-read classes, after the policy layer allowed it).
    fn run(&self, args: &Value, ctx: &ToolCtx, host: &dyn ToolHost) -> ToolResult;
}

/// The effectful surface the tools are built on. `RealToolHost` touches the
/// filesystem; `MockToolHost` is an in-memory world for tests. This is the same
/// split that made the computer-use ActLoop testable with a mock desktop.
pub trait ToolHost: Send + Sync {
    fn read_dir(&self, path: &Path) -> Result<Vec<HostEntry>, String>;
    fn read_file(&self, path: &Path, max_bytes: usize) -> Result<String, String>;
    fn file_size(&self, path: &Path) -> Result<u64, String>;
    fn exists(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    /// Folders to walk under a root, capped by the caller.
    fn walk(&self, root: &Path, max_depth: usize, max_entries: usize) -> Vec<PathBuf>;
    /// A short, human-readable description of the machine.
    fn system_info(&self) -> SystemInfo;
    /// Free/total bytes for the volume holding `path`.
    fn volume_usage(&self, path: &Path) -> Option<(u64, u64)>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostEntry {
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
    /// Extension without the dot, lowercased.
    pub ext: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub cpus: usize,
    pub total_memory_bytes: u64,
    pub hostname: String,
}

/// The registry: names → tools. Cheap to clone-share, immutable after build.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) -> &mut Self {
        self.tools.insert(tool.spec().name.clone(), tool);
        self
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// The tool list as the model sees it (for a tool-calling request body).
    pub fn schemas(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| {
                let s = t.spec();
                json!({
                    "name": s.name,
                    "description": s.description,
                    "parameters": s.params,
                    "example": s.example,
                    "risk": s.class.as_str(),
                })
            })
            .collect()
    }

    /// A compact textual catalogue — the fallback for providers without native
    /// tool calling, and the thing that keeps the prompt small.
    pub fn catalogue(&self) -> String {
        let mut out = String::new();
        for tool in self.tools.values() {
            let s = tool.spec();
            out.push_str(&format!(
                "- {} ({}) — {}\n",
                s.name,
                s.class.as_str(),
                s.description
            ));
            out.push_str(&format!("  example: {}\n", s.example));
        }
        out
    }

    /// Nearest names for a typo, so `UnknownTool` can be repaired in one turn.
    ///
    /// Token similarity alone is too blunt for short identifiers (`echoo` vs
    /// `echo` shares no 3+ character token with itself once lowercased), so
    /// this also accepts a prefix/substring relationship — which is what a
    /// dropped or doubled letter actually looks like.
    pub fn suggest(&self, name: &str) -> String {
        let needle = name.to_lowercase();
        let mut best: Option<(f64, &str)> = None;
        for candidate in self.tools.keys() {
            let lower = candidate.to_lowercase();
            let mut score = crate::dedup::jaccard(&needle, &lower);
            if lower.starts_with(&needle) || needle.starts_with(&lower) {
                score = score.max(0.5);
            }
            if lower.contains(&needle) || needle.contains(&lower) {
                score = score.max(0.3);
            }
            if best.map(|(s, _)| score > s).unwrap_or(true) {
                best = Some((score, candidate.as_str()));
            }
        }
        match best {
            Some((score, name)) if score > 0.0 => name.to_string(),
            _ => String::new(),
        }
    }

    /// Validate + run one call. The caller is responsible for the policy
    /// decision; this only enforces structure and the hard floor.
    pub fn call(&self, name: &str, args: &Value, ctx: &ToolCtx, host: &dyn ToolHost) -> ToolResult {
        let Some(tool) = self.get(name) else {
            return ToolResult::fail(ToolError::UnknownTool {
                name: name.to_string(),
                suggestion: self.suggest(name),
            });
        };
        if let Err(e) = tool.validate(args, ctx) {
            return ToolResult::fail(e);
        }
        let mut result = tool.run(args, ctx, host);
        // Belt and braces: a tool that overreaches gets trimmed here, whatever
        // it did internally. The model never receives an unbounded blob.
        let cap = tool.spec().max_output_bytes;
        if result.summary.len() > cap {
            result
                .summary
                .truncate(floor_char_boundary(&result.summary, cap));
            result.summary.push_str("\n… (truncated)");
            result.truncated = true;
        }
        result
    }
}

/// Char-boundary-safe truncation (slicing UTF-8 at a byte offset panics).
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ---------- argument helpers (small, typed, and used by every tool) ----------

pub fn arg_str(args: &Value, field: &str) -> Result<String, ToolError> {
    match args.get(field) {
        None => Err(ToolError::MissingField {
            field: field.to_string(),
        }),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(ToolError::BadType {
            field: field.to_string(),
            expected: "a string",
        }),
    }
}

pub fn arg_str_opt(args: &Value, field: &str) -> Result<Option<String>, ToolError> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ToolError::BadType {
            field: field.to_string(),
            expected: "a string",
        }),
    }
}

pub fn arg_u64_opt(args: &Value, field: &str) -> Result<Option<u64>, ToolError> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or(ToolError::BadType {
            field: field.to_string(),
            expected: "a positive integer",
        }),
        Some(_) => Err(ToolError::BadType {
            field: field.to_string(),
            expected: "a positive integer",
        }),
    }
}

/// Reject arguments the tool does not know: a typo like `foldr` must not be
/// silently ignored, which is how a model ends up with a wrong-but-quiet answer.
pub fn reject_unknown_args(args: &Value, allowed: &[&str]) -> Result<(), ToolError> {
    let Some(obj) = args.as_object() else {
        return Err(ToolError::BadType {
            field: "(arguments)".into(),
            expected: "an object",
        });
    };
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ToolError::BadValue {
                field: key.clone(),
                why: format!("unknown argument; this tool accepts {}", allowed.join(", ")),
            });
        }
    }
    Ok(())
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    static ECHO: ToolSpec = ToolSpec {
        name: String::new(), // replaced in the unit test below
        description: String::new(),
        params: Value::Null,
        class: RiskClass::Read,
        max_output_bytes: 64,
        timeout_ms: 1000,
        example: Value::Null,
    };

    fn echo_spec() -> ToolSpec {
        ToolSpec {
            name: "echo".into(),
            description: "returns the text you pass".into(),
            params: json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
            class: RiskClass::Read,
            max_output_bytes: 64,
            timeout_ms: 1000,
            example: json!({"tool":"echo","args":{"text":"hi"}}),
        }
    }

    impl Tool for Echo {
        fn spec(&self) -> &ToolSpec {
            let _ = &ECHO;
            // A leaked spec is fine in tests; the real tools use statics.
            Box::leak(Box::new(echo_spec()))
        }
        fn validate(&self, args: &Value, _ctx: &ToolCtx) -> Result<(), ToolError> {
            reject_unknown_args(args, &["text"])?;
            arg_str(args, "text")?;
            Ok(())
        }
        fn run(&self, args: &Value, _ctx: &ToolCtx, _host: &dyn ToolHost) -> ToolResult {
            ToolResult::ok(arg_str(args, "text").unwrap_or_default())
        }
    }

    struct NullHost;
    impl ToolHost for NullHost {
        fn read_dir(&self, _p: &Path) -> Result<Vec<HostEntry>, String> {
            Ok(Vec::new())
        }
        fn read_file(&self, _p: &Path, _m: usize) -> Result<String, String> {
            Err("no filesystem".into())
        }
        fn file_size(&self, _p: &Path) -> Result<u64, String> {
            Err("no filesystem".into())
        }
        fn exists(&self, _p: &Path) -> bool {
            false
        }
        fn is_dir(&self, _p: &Path) -> bool {
            false
        }
        fn walk(&self, _r: &Path, _d: usize, _m: usize) -> Vec<PathBuf> {
            Vec::new()
        }
        fn system_info(&self) -> SystemInfo {
            SystemInfo::default()
        }
        fn volume_usage(&self, _p: &Path) -> Option<(u64, u64)> {
            None
        }
    }

    #[test]
    fn registry_calls_a_known_tool_and_refuses_an_unknown_one() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(Echo));
        assert_eq!(registry.len(), 1);
        let ctx = ToolCtx::default();
        let host = NullHost;

        let ok = registry.call("echo", &json!({"text":"hello"}), &ctx, &host);
        assert!(ok.ok);
        assert_eq!(ok.summary, "hello");

        let unknown = registry.call("echoo", &json!({}), &ctx, &host);
        assert!(!unknown.ok);
        match unknown.error.unwrap() {
            ToolError::UnknownTool { suggestion, .. } => assert_eq!(suggestion, "echo"),
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    #[test]
    fn validation_happens_before_anything_runs() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(Echo));
        let ctx = ToolCtx::default();
        let host = NullHost;

        // Missing field.
        let missing = registry.call("echo", &json!({}), &ctx, &host);
        assert!(matches!(
            missing.error,
            Some(ToolError::MissingField { .. })
        ));
        // Wrong type.
        let wrong = registry.call("echo", &json!({"text": 12}), &ctx, &host);
        assert!(matches!(wrong.error, Some(ToolError::BadType { .. })));
        // Unknown argument (a typo must never be silently ignored).
        let typo = registry.call("echo", &json!({"txt": "hi"}), &ctx, &host);
        assert!(matches!(typo.error, Some(ToolError::BadValue { .. })));
    }

    #[test]
    fn every_error_renders_one_actionable_sentence() {
        let errors = vec![
            ToolError::UnknownTool {
                name: "run_shll".into(),
                suggestion: "run_shell".into(),
            },
            ToolError::MissingField {
                field: "path".into(),
            },
            ToolError::BadType {
                field: "limit".into(),
                expected: "a positive integer",
            },
            ToolError::OutOfRoots {
                path: "C:/Windows".into(),
            },
            ToolError::Forbidden {
                path: "~/.ssh/id_rsa".into(),
                why: "it is a credential",
            },
            ToolError::Denied {
                class: RiskClass::Execute,
                why: "commands are off".into(),
            },
            ToolError::TooLarge {
                bytes: 900_000,
                cap: 8_192,
            },
        ];
        for error in errors {
            let message = error.message();
            assert!(!message.is_empty());
            assert!(
                !message.contains("error:") && !message.contains("Err("),
                "messages are for the model, not the compiler: {message}"
            );
        }
    }

    #[test]
    fn the_forbidden_floor_runs_before_any_root_logic() {
        let ctx = ToolCtx {
            roots: Roots::new(vec![PathBuf::from("C:/Users/me")]),
            now_unix: 0,
            denied_paths: Vec::new(),
            memory: None,
        };
        // Even inside an allowed root, credentials are off limits.
        for bad in [
            "C:/Users/me/.ssh/id_rsa",
            "C:/Users/me/.aws/credentials.json",
            "C:/Users/me/AppData/settings.json",
            "C:/Users/me/integrity.json",
            "C:/Users/me/login data",
        ] {
            let err = ctx
                .guard_path(bad)
                .expect_err(&format!("{bad} must be refused"));
            assert!(matches!(err, ToolError::Forbidden { .. }));
            assert!(err.is_policy());
        }
        assert!(ctx.guard_path("C:/Users/me/notes/todo.md").is_ok());
    }

    #[test]
    fn paths_outside_the_roots_are_refused() {
        let ctx = ToolCtx {
            roots: Roots::new(vec![PathBuf::from("C:/Users/me/Documents")]),
            now_unix: 0,
            denied_paths: Vec::new(),
            memory: None,
        };
        assert!(ctx.resolve("notes.md").is_ok(), "relative joins the root");
        assert!(ctx.resolve("C:/Users/me/Documents/a/b.md").is_ok());
        for bad in ["C:/Windows/system32", "C:/Users/me/Desktop/x"] {
            let err = ctx.resolve(bad).expect_err(bad);
            assert!(matches!(err, ToolError::OutOfRoots { .. }));
        }
    }

    #[test]
    fn an_empty_root_set_refuses_everything() {
        let ctx = ToolCtx::default();
        let err = ctx.resolve("notes.md").unwrap_err();
        assert!(matches!(err, ToolError::OutOfRoots { .. }));
    }

    #[test]
    fn output_is_capped_at_a_char_boundary() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(Echo));
        let ctx = ToolCtx::default();
        let host = NullHost;
        // 64-byte cap from the spec; multibyte text must not panic the truncation.
        let long = "مرحبا بالعالم ".repeat(20);
        let result = registry.call("echo", &json!({ "text": long }), &ctx, &host);
        assert!(result.ok);
        assert!(result.truncated);
        assert!(result.summary.len() <= 64 + "\n… (truncated)".len());
        assert!(result.summary.ends_with("(truncated)"));
    }

    #[test]
    fn the_catalogue_and_schemas_describe_every_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(Echo));
        let catalogue = registry.catalogue();
        assert!(catalogue.contains("echo (R)"));
        assert!(catalogue.contains("example:"));
        let schemas = registry.schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0]["name"], "echo");
        assert_eq!(schemas[0]["risk"], "R");
    }

    #[test]
    fn human_bytes_is_readable() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
