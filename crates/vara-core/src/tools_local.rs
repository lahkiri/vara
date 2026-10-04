//! Filesystem-backed tool host + the Phase-1 read-only tool set.
//!
//! The registry (`tools_registry.rs`) defines *what a tool is*; this module
//! defines *what the tools can actually see*. Two host implementations share
//! one contract:
//!
//! * [`MockToolHost`] — an in-memory world used by the tests and the harness.
//!   It is what makes the safety rules provable headlessly: no machine, no
//!   clock, no ambient environment.
//! * [`FsToolHost`] — the real thing, used by the shell. It never decides
//!   policy; it only reads what it is handed.
//!
//! The read-only tools here are the direct answer to gap G-02 in the audit
//! ("explain my system" ended in a refused shell command): the entity now has
//! a legal, typed way to look at the machine instead of proposing `systeminfo | findstr`.

use crate::tools_registry::{
    arg_str, arg_str_opt, arg_u64_opt, human_bytes, reject_unknown_args, HostEntry, RiskClass,
    SystemInfo, Tool, ToolCtx, ToolError, ToolHost, ToolResult, ToolSpec,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

// ---------------------------------------------------------------- mock host

/// A deterministic filesystem in memory. Paths are stored exactly as given
/// (normalized to forward slashes) so tests read like the real world.
#[derive(Default)]
pub struct MockToolHost {
    files: Mutex<HashMap<String, String>>,
    info: SystemInfo,
    volumes: Mutex<HashMap<String, (u64, u64)>>,
}

fn key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl MockToolHost {
    pub fn new() -> Self {
        Self {
            info: SystemInfo {
                os: "MockOS 1.0".into(),
                arch: "x86_64".into(),
                cpus: 8,
                total_memory_bytes: 16 * 1024 * 1024 * 1024,
                hostname: "mock-host".into(),
            },
            ..Default::default()
        }
    }

    pub fn with_file(mut self, path: &str, body: &str) -> Self {
        self.files
            .get_mut()
            .unwrap()
            .insert(path.replace('\\', "/"), body.to_string());
        self
    }

    pub fn with_volume(mut self, root: &str, used: u64, total: u64) -> Self {
        self.volumes
            .get_mut()
            .unwrap()
            .insert(root.to_string(), (used, total));
        self
    }

    fn entries_under(&self, dir: &Path) -> Vec<HostEntry> {
        let dir_key = key(dir).trim_end_matches('/').to_string();
        let files = self.files.lock().unwrap();
        let mut names: Vec<(String, u64, bool)> = Vec::new();
        for (path, body) in files.iter() {
            let trimmed = path.trim_end_matches('/');
            let Some((parent, name)) = trimmed.rsplit_once('/') else {
                continue;
            };
            if parent != dir_key {
                continue;
            }
            names.push((name.to_string(), body.len() as u64, false));
        }
        names.sort();
        names
            .into_iter()
            .map(|(name, bytes, is_dir)| HostEntry {
                ext: name
                    .rsplit_once('.')
                    .map(|(_, e)| e.to_lowercase())
                    .unwrap_or_default(),
                name,
                is_dir,
                bytes,
            })
            .collect()
    }
}

impl ToolHost for MockToolHost {
    fn read_dir(&self, path: &Path) -> Result<Vec<HostEntry>, String> {
        let entries = self.entries_under(path);
        if entries.is_empty() {
            return Err(format!("directory not found: {}", key(path)));
        }
        Ok(entries)
    }

    fn read_file(&self, path: &Path, max_bytes: usize) -> Result<String, String> {
        let files = self.files.lock().unwrap();
        let body = files
            .get(&key(path))
            .ok_or_else(|| format!("file not found: {}", key(path)))?;
        let cut = crate::tools_registry::floor_char_boundary(body, max_bytes);
        Ok(body[..cut].to_string())
    }

    fn file_size(&self, path: &Path) -> Result<u64, String> {
        let files = self.files.lock().unwrap();
        files
            .get(&key(path))
            .map(|b| b.len() as u64)
            .ok_or_else(|| format!("file not found: {}", key(path)))
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.lock().unwrap().contains_key(&key(path))
    }

    fn is_dir(&self, path: &Path) -> bool {
        !self.entries_under(path).is_empty()
    }

    fn walk(&self, root: &Path, _max_depth: usize, max_entries: usize) -> Vec<PathBuf> {
        let root_key = key(root).trim_end_matches('/').to_string();
        let files = self.files.lock().unwrap();
        let mut out: Vec<PathBuf> = files
            .keys()
            .filter(|p| p.starts_with(&(root_key.clone() + "/")))
            .map(PathBuf::from)
            .collect();
        out.sort();
        out.truncate(max_entries);
        out
    }

    fn system_info(&self) -> SystemInfo {
        self.info.clone()
    }

    fn volume_usage(&self, path: &Path) -> Option<(u64, u64)> {
        let text = key(path);
        let volumes = self.volumes.lock().unwrap();
        volumes
            .iter()
            .find(|(root, _)| text.starts_with(root.as_str()))
            .map(|(_, v)| *v)
            .or_else(|| volumes.values().next().copied())
    }
}

// ------------------------------------------------------------------ real host

/// The real filesystem. Every method returns a plain `Result` so a tool can
/// turn an I/O failure into a typed, actionable error instead of panicking.
pub struct FsToolHost {
    info: SystemInfo,
}

impl Default for FsToolHost {
    fn default() -> Self {
        Self::new()
    }
}

impl FsToolHost {
    pub fn new() -> Self {
        let info = SystemInfo {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cpus: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            total_memory_bytes: total_memory_bytes(),
            hostname: hostname(),
        };
        Self { info }
    }
}

impl ToolHost for FsToolHost {
    fn read_dir(&self, path: &Path) -> Result<Vec<HostEntry>, String> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let meta = entry.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            out.push(HostEntry {
                ext: name
                    .rsplit_once('.')
                    .map(|(_, e)| e.to_lowercase())
                    .unwrap_or_default(),
                name,
                is_dir,
                bytes: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            });
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(out)
    }

    fn read_file(&self, path: &Path, max_bytes: usize) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let cut = bytes.len().min(max_bytes);
        // Refuse binary rather than feeding mojibake to a model.
        if bytes.iter().take(1024).any(|b| *b == 0) {
            return Err("binary file (refused)".into());
        }
        let text = String::from_utf8_lossy(&bytes[..cut]).to_string();
        Ok(text)
    }

    fn file_size(&self, path: &Path) -> Result<u64, String> {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| e.to_string())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn walk(&self, root: &Path, max_depth: usize, max_entries: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![(root.to_path_buf(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if out.len() >= max_entries || depth > max_depth {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    // Skip the noise dirs that make a walk useless.
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if matches!(
                        name.as_str(),
                        "node_modules" | ".git" | "target" | "$recycle.bin" | "windows"
                    ) {
                        continue;
                    }
                    stack.push((path, depth + 1));
                } else {
                    out.push(path);
                    if out.len() >= max_entries {
                        break;
                    }
                }
            }
        }
        out
    }

    fn system_info(&self) -> SystemInfo {
        self.info.clone()
    }

    fn volume_usage(&self, _path: &Path) -> Option<(u64, u64)> {
        None // the shell fills this in from the OS; see `sysinfo` in the app layer
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "this machine".into())
}

fn total_memory_bytes() -> u64 {
    // Platform-specific totals are filled in by the shell (it already links the
    // Windows APIs); the core stays dependency-free and reports 0 when unknown
    // rather than inventing a number.
    0
}

/// Unix seconds, read once by the caller — tools never touch the clock.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// --------------------------------------------------------------- read tools

/// `system_info` — the tool whose absence made "explain my system" fail.
pub struct SystemInfoTool;
impl SystemInfoTool {
    fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "system_info".into(),
            description: "Report the operating system, architecture, CPU count and memory of this machine. Use it for any question about 'my system' before searching the web.".into(),
            params: json!({"type":"object","properties":{},"additionalProperties":false}),
            class: RiskClass::Read,
            max_output_bytes: 1200,
            timeout_ms: 5_000,
            example: json!({"tool":"system_info","args":{}}),
        }
    }
}
impl Tool for SystemInfoTool {
    fn spec(&self) -> &ToolSpec {
        static S: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        S.get_or_init(SystemInfoTool::spec_value)
    }
    fn validate(&self, args: &Value, _ctx: &ToolCtx) -> Result<(), ToolError> {
        reject_unknown_args(args, &[])
    }
    fn run(&self, _args: &Value, _ctx: &ToolCtx, host: &dyn ToolHost) -> ToolResult {
        let info = host.system_info();
        let memory = if info.total_memory_bytes > 0 {
            human_bytes(info.total_memory_bytes)
        } else {
            "unknown".into()
        };
        ToolResult::ok_with(
            format!(
                "{} ({}) · {} CPU cores · {} RAM · host {}",
                info.os, info.arch, info.cpus, memory, info.hostname
            ),
            json!({
                "os": info.os,
                "arch": info.arch,
                "cpus": info.cpus,
                "memory_bytes": info.total_memory_bytes,
                "hostname": info.hostname,
            }),
            1,
            false,
        )
    }
}

/// `list_dir` — one directory level, capped, sorted.
pub struct ListDirTool;
impl ListDirTool {
    fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "list_dir".into(),
            description: "List the entries of one directory inside the owner's allowed folders. Returns a count plus the largest entries; use find_files to search deeper.".into(),
            params: json!({
                "type":"object",
                "properties":{
                    "path":{"type":"string","description":"directory path, absolute or relative to an allowed folder"},
                    "limit":{"type":"integer","description":"maximum entries to show (default 25)"}
                },
                "required":["path"],
                "additionalProperties":false
            }),
            class: RiskClass::Read,
            max_output_bytes: 2500,
            timeout_ms: 10_000,
            example: json!({"tool":"list_dir","args":{"path":"Downloads","limit":25}}),
        }
    }
}
impl Tool for ListDirTool {
    fn spec(&self) -> &ToolSpec {
        static S: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        S.get_or_init(ListDirTool::spec_value)
    }
    fn validate(&self, args: &Value, ctx: &ToolCtx) -> Result<(), ToolError> {
        reject_unknown_args(args, &["path", "limit"])?;
        let path = arg_str(args, "path")?;
        ctx.resolve_real(&path)?;
        Ok(())
    }
    fn run(&self, args: &Value, ctx: &ToolCtx, host: &dyn ToolHost) -> ToolResult {
        let raw = arg_str(args, "path").unwrap_or_default();
        let limit = arg_u64_opt(args, "limit").ok().flatten().unwrap_or(25) as usize;
        let dir = match ctx.resolve_real(&raw) {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(e),
        };
        match host.read_dir(&dir) {
            Err(why) => ToolResult::fail(ToolError::Failed { why }),
            Ok(mut entries) => {
                let total = entries.len();
                entries.sort_by(|a, b| b.bytes.cmp(&a.bytes));
                let shown: Vec<Value> = entries
                    .iter()
                    .take(limit)
                    .map(|e| {
                        json!({
                            "name": e.name,
                            "kind": if e.is_dir { "dir" } else { "file" },
                            "bytes": e.bytes,
                            "ext": e.ext,
                        })
                    })
                    .collect();
                let truncated = total > shown.len();
                let biggest = entries
                    .first()
                    .map(|e| format!(", largest {} ({})", e.name, human_bytes(e.bytes)))
                    .unwrap_or_default();
                ToolResult::ok_with(
                    format!("{total} entries in {}{}", dir.display(), biggest),
                    json!({"entries": shown}),
                    total,
                    truncated,
                )
            }
        }
    }
}

/// `find_files` — glob-ish search under the allowed roots.
pub struct FindFilesTool;
impl FindFilesTool {
    fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "find_files".into(),
            description: "Find files under an allowed folder by name substring or extension. Returns the biggest matches, not every path.".into(),
            params: json!({
                "type":"object",
                "properties":{
                    "query":{"type":"string","description":"name substring to match (case-insensitive); empty matches everything"},
                    "ext":{"type":"string","description":"extension filter without the dot, e.g. pdf"},
                    "root":{"type":"string","description":"folder to search in; defaults to the first allowed folder"},
                    "limit":{"type":"integer","description":"maximum matches (default 20, hard cap 100)"}
                },
                "required":[],
                "additionalProperties":false
            }),
            class: RiskClass::Read,
            max_output_bytes: 2500,
            timeout_ms: 20_000,
            example: json!({"tool":"find_files","args":{"query":"invoice","ext":"pdf","limit":10}}),
        }
    }
}
impl Tool for FindFilesTool {
    fn spec(&self) -> &ToolSpec {
        static S: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        S.get_or_init(FindFilesTool::spec_value)
    }
    fn validate(&self, args: &Value, ctx: &ToolCtx) -> Result<(), ToolError> {
        reject_unknown_args(args, &["query", "ext", "root", "limit"])?;
        if let Some(root) = arg_str_opt(args, "root")? {
            ctx.resolve_real(&root)?;
        } else if ctx.roots.is_empty() {
            return Err(ToolError::OutOfRoots {
                path: "(no allowed folder configured)".into(),
            });
        }
        Ok(())
    }
    fn run(&self, args: &Value, ctx: &ToolCtx, host: &dyn ToolHost) -> ToolResult {
        let query = arg_str_opt(args, "query")
            .ok()
            .flatten()
            .unwrap_or_default();
        let ext = arg_str_opt(args, "ext").ok().flatten().unwrap_or_default();
        let limit = arg_u64_opt(args, "limit")
            .ok()
            .flatten()
            .unwrap_or(20)
            .min(100) as usize;
        let roots: Vec<PathBuf> = match arg_str_opt(args, "root").ok().flatten() {
            Some(raw) => match ctx.resolve_real(&raw) {
                Ok(p) => vec![p],
                Err(e) => return ToolResult::fail(e),
            },
            None => ctx.roots.paths().to_vec(),
        };
        let needle = query.to_lowercase();
        let mut hits: Vec<(PathBuf, u64)> = Vec::new();
        for root in roots {
            for path in host.walk(&root, 4, 4_000) {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if !needle.is_empty() && !name.contains(&needle) {
                    continue;
                }
                if !ext.is_empty()
                    && !name.ends_with(&format!(".{}", ext.trim_start_matches('.').to_lowercase()))
                {
                    continue;
                }
                let bytes = host.file_size(&path).unwrap_or(0);
                hits.push((path, bytes));
            }
        }
        let total = hits.len();
        hits.sort_by(|a, b| b.1.cmp(&a.1));
        let shown: Vec<Value> = hits
            .iter()
            .take(limit)
            .map(|(p, b)| json!({"path": p.display().to_string(), "bytes": b}))
            .collect();
        if total == 0 {
            return ToolResult::ok_with(
                if needle.is_empty() && ext.is_empty() {
                    "no files found under the allowed folders".to_string()
                } else {
                    format!("no file matching query='{query}' ext='{ext}'")
                },
                json!({"matches": []}),
                0,
                false,
            );
        }
        ToolResult::ok_with(
            format!(
                "{total} matching files; largest is {}",
                human_bytes(hits[0].1)
            ),
            json!({"matches": shown}),
            total,
            total > shown.len(),
        )
    }
}

/// `read_file` — allowed roots only, capped, binary refused.
pub struct ReadFileTool;
impl ReadFileTool {
    fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "Read a text file inside an allowed folder (capped). Use it before answering anything about a document's contents.".into(),
            params: json!({
                "type":"object",
                "properties":{
                    "path":{"type":"string","description":"file path"},
                    "max_bytes":{"type":"integer","description":"read at most this many bytes (default 8000, hard cap 32000)"}
                },
                "required":["path"],
                "additionalProperties":false
            }),
            class: RiskClass::Read,
            max_output_bytes: 9000,
            timeout_ms: 10_000,
            example: json!({"tool":"read_file","args":{"path":"notes/todo.md"}}),
        }
    }
}
impl Tool for ReadFileTool {
    fn spec(&self) -> &ToolSpec {
        static S: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        S.get_or_init(ReadFileTool::spec_value)
    }
    fn validate(&self, args: &Value, ctx: &ToolCtx) -> Result<(), ToolError> {
        reject_unknown_args(args, &["path", "max_bytes"])?;
        let path = arg_str(args, "path")?;
        ctx.resolve_real(&path)?;
        arg_u64_opt(args, "max_bytes")?;
        Ok(())
    }
    fn run(&self, args: &Value, ctx: &ToolCtx, host: &dyn ToolHost) -> ToolResult {
        let raw = arg_str(args, "path").unwrap_or_default();
        let cap = arg_u64_opt(args, "max_bytes")
            .ok()
            .flatten()
            .unwrap_or(8_000)
            .min(32_000) as usize;
        let path = match ctx.resolve_real(&raw) {
            Ok(p) => p,
            Err(e) => return ToolResult::fail(e),
        };
        if !host.exists(&path) {
            return ToolResult::fail(ToolError::Failed {
                why: format!("no such file: {}", path.display()),
            });
        }
        match host.read_file(&path, cap) {
            Err(why) => ToolResult::fail(ToolError::Failed { why }),
            Ok(text) => {
                let bytes = host.file_size(&path).unwrap_or(0);
                ToolResult::ok_with(
                    text.clone(),
                    json!({"path": path.display().to_string(), "bytes": bytes, "text": text}),
                    1,
                    bytes > cap as u64,
                )
            }
        }
    }
}

/// `disk_usage` — free/total space, and optionally the size of a folder.
pub struct DiskUsageTool;
impl DiskUsageTool {
    fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "disk_usage".into(),
            description: "Report free and total space of the drive holding a folder. Answers 'how much space do I have left' without a shell.".into(),
            params: json!({
                "type":"object",
                "properties":{"path":{"type":"string","description":"a folder inside the allowed roots; defaults to the first allowed folder"}},
                "required":[],
                "additionalProperties":false
            }),
            class: RiskClass::Read,
            max_output_bytes: 800,
            timeout_ms: 10_000,
            example: json!({"tool":"disk_usage","args":{"path":"Downloads"}}),
        }
    }
}
impl Tool for DiskUsageTool {
    fn spec(&self) -> &ToolSpec {
        static S: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        S.get_or_init(DiskUsageTool::spec_value)
    }
    fn validate(&self, args: &Value, ctx: &ToolCtx) -> Result<(), ToolError> {
        reject_unknown_args(args, &["path"])?;
        match arg_str_opt(args, "path")? {
            Some(p) => {
                ctx.resolve_real(&p)?;
                Ok(())
            }
            None if !ctx.roots.is_empty() => Ok(()),
            None => Err(ToolError::OutOfRoots {
                path: "(no allowed folder configured)".into(),
            }),
        }
    }
    fn run(&self, args: &Value, ctx: &ToolCtx, host: &dyn ToolHost) -> ToolResult {
        let path = match arg_str_opt(args, "path").ok().flatten() {
            Some(raw) => match ctx.resolve_real(&raw) {
                Ok(p) => p,
                Err(e) => return ToolResult::fail(e),
            },
            None => match ctx.roots.paths().first() {
                Some(p) => p.clone(),
                None => {
                    return ToolResult::fail(ToolError::OutOfRoots {
                        path: "(no allowed folder configured)".into(),
                    })
                }
            },
        };
        match host.volume_usage(&path) {
            Some((used, total)) if total > 0 => {
                let free = total.saturating_sub(used);
                ToolResult::ok_with(
                    format!(
                        "{} free of {} ({:.0}% used)",
                        human_bytes(free),
                        human_bytes(total),
                        (used as f64 / total as f64) * 100.0
                    ),
                    json!({"free_bytes": free, "total_bytes": total, "used_bytes": used}),
                    1,
                    false,
                )
            }
            _ => ToolResult::ok_with(
                "drive totals are not available on this platform yet",
                json!({}),
                1,
                false,
            ),
        }
    }
}

/// `memory_search` — ask the entity what it already knows, without a mission.
pub struct MemorySearchTool;
impl MemorySearchTool {
    fn spec_value() -> ToolSpec {
        ToolSpec {
            name: "memory_search".into(),
            description: "Search Vara's own memory (research notes and facts learned earlier). Use it instead of starting a mission when the answer may already be known.".into(),
            params: json!({
                "type":"object",
                "properties":{
                    "query":{"type":"string","description":"what to look for"},
                    "limit":{"type":"integer","description":"maximum notes (default 5)"}
                },
                "required":["query"],
                "additionalProperties":false
            }),
            class: RiskClass::Read,
            max_output_bytes: 3000,
            timeout_ms: 10_000,
            example: json!({"tool":"memory_search","args":{"query":"inference servers"}}),
        }
    }
}
impl Tool for MemorySearchTool {
    fn spec(&self) -> &ToolSpec {
        static S: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        S.get_or_init(MemorySearchTool::spec_value)
    }
    fn validate(&self, args: &Value, _ctx: &ToolCtx) -> Result<(), ToolError> {
        reject_unknown_args(args, &["query", "limit"])?;
        let q = arg_str(args, "query")?;
        if q.trim().is_empty() {
            return Err(ToolError::BadValue {
                field: "query".into(),
                why: "must not be empty".into(),
            });
        }
        Ok(())
    }
    /// The database is not part of `ToolHost` on purpose: memory search is a
    /// pure read of Vara's own store, supplied by the caller through
    /// [`MemoryProvider`] so the tool stays testable without SQLite.
    fn run(&self, args: &Value, ctx: &ToolCtx, _host: &dyn ToolHost) -> ToolResult {
        let query = arg_str(args, "query").unwrap_or_default();
        let limit = arg_u64_opt(args, "limit").ok().flatten().unwrap_or(5) as usize;
        let Some(provider) = &ctx.memory else {
            return ToolResult::ok("memory search is not available in this context");
        };
        match provider.search(&query, limit) {
            Err(why) => ToolResult::fail(ToolError::Failed { why }),
            Ok(hits) if hits.is_empty() => {
                ToolResult::ok_with("nothing in memory matches that yet", json!([]), 0, false)
            }
            Ok(hits) => {
                let total = hits.len();
                let lines: Vec<String> = hits
                    .iter()
                    .map(|h| format!("- {}: {}", h.title, crate::tools::truncate(&h.body, 200)))
                    .collect();
                ToolResult::ok_with(
                    lines.join("\n"),
                    json!({"hits": hits.iter().map(|h| json!({"title": h.title, "body": h.body})).collect::<Vec<_>>()}),
                    total,
                    false,
                )
            }
        }
    }
}

/// One memory hit.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryHit {
    pub title: String,
    pub body: String,
}

/// How the core asks for memory without linking SQLite into the tool layer.
pub trait MemoryProvider: Send + Sync {
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryHit>, String>;
}

/// Build the Phase-1 read-only registry.
pub fn read_only_registry() -> crate::tools_registry::ToolRegistry {
    let mut registry = crate::tools_registry::ToolRegistry::new();
    registry.register(Box::new(SystemInfoTool));
    registry.register(Box::new(ListDirTool));
    registry.register(Box::new(FindFilesTool));
    registry.register(Box::new(ReadFileTool));
    registry.register(Box::new(DiskUsageTool));
    registry.register(Box::new(MemorySearchTool));
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools_registry::{Roots, ToolCtx};

    /// A root that is absolute on **every** platform.
    ///
    /// These tests used to hardcode `C:/Users/me`, which Windows treats as
    /// absolute and Unix treats as a single relative component — so
    /// `Path::is_absolute()` disagreed between CI's Linux runner and a Windows
    /// dev box, and five tests that passed locally failed there. Building the
    /// root from the platform's own temp directory tests the behaviour (roots,
    /// confinement, mock file lookup) instead of the spelling of a path.
    fn demo_root() -> PathBuf {
        let dir = std::env::temp_dir().join("vara-tools-demo");
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    /// A path inside [demo_root], spelled the way the tools resolve it.
    fn under(path: &str) -> String {
        demo_root().join(path).to_string_lossy().replace('\\', "/")
    }

    fn ctx(root: &std::path::Path) -> ToolCtx {
        ToolCtx {
            roots: Roots::new(vec![root.to_path_buf()]),
            now_unix: 1_700_000_000,
            denied_paths: Vec::new(),
            memory: None,
        }
    }

    fn demo_host() -> MockToolHost {
        MockToolHost::new()
            .with_file(&under("Downloads/report.pdf"), "pdf-bytes")
            .with_file(&under("Downloads/photo.jpg"), "jpg")
            .with_file(&under("Downloads/notes/todo.md"), "# todo\nbuy milk")
            .with_volume(&demo_root().to_string_lossy(), 300, 1000)
    }

    #[test]
    fn system_info_answers_the_question_that_used_to_fail() {
        let registry = read_only_registry();
        let result = registry.call("system_info", &json!({}), &ctx(&demo_root()), &demo_host());
        assert!(result.ok, "{}", result.summary);
        assert!(result.summary.contains("MockOS 1.0"));
        assert!(result.summary.contains("8 CPU cores"));
    }

    #[test]
    fn list_dir_counts_and_shows_the_largest() {
        let registry = read_only_registry();
        let result = registry.call(
            "list_dir",
            &json!({"path":"Downloads","limit":2}),
            &ctx(&demo_root()),
            &demo_host(),
        );
        assert!(result.ok, "{}", result.summary);
        assert!(
            result.summary.starts_with("2 entries"),
            "{}",
            result.summary
        );
        let listed = result.data["entries"].as_array().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["name"], "report.pdf");
    }

    #[test]
    fn find_files_filters_by_name_and_extension() {
        let registry = read_only_registry();
        let host = demo_host();
        let ctx = ctx(&demo_root());

        let by_ext = registry.call("find_files", &json!({"ext":"pdf"}), &ctx, &host);
        assert!(by_ext.ok);
        assert_eq!(by_ext.total, Some(1));

        let by_name = registry.call("find_files", &json!({"query":"todo"}), &ctx, &host);
        assert!(by_name.ok);
        assert_eq!(by_name.total, Some(1));

        let nothing = registry.call("find_files", &json!({"query":"zzz"}), &ctx, &host);
        assert!(nothing.ok);
        assert!(nothing.summary.contains("no file matching"));
    }

    #[test]
    fn read_file_returns_content_and_refuses_binaries_or_missing_files() {
        let registry = read_only_registry();
        let host = demo_host();
        let ctx = ctx(&demo_root());

        let ok = registry.call(
            "read_file",
            &json!({"path":"Downloads/notes/todo.md"}),
            &ctx,
            &host,
        );
        assert!(ok.ok, "{}", ok.summary);
        assert!(ok.summary.contains("buy milk"));

        let missing = registry.call(
            "read_file",
            &json!({"path":"Downloads/nope.md"}),
            &ctx,
            &host,
        );
        assert!(!missing.ok);
        assert!(matches!(missing.error, Some(ToolError::Failed { .. })));

        // FsToolHost refuses binaries; the mock has no binary marker, so check
        // the real host's contract with a NUL-containing temp file.
        let dir = std::env::temp_dir().join("vara-tool-test");
        std::fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("blob.bin");
        std::fs::write(&binary, [0u8, 1, 2, 3]).unwrap();
        let real = FsToolHost::new();
        let err = real.read_file(&binary, 100).unwrap_err();
        assert!(err.contains("binary"), "{err}");
    }

    #[test]
    fn disk_usage_reports_free_and_total() {
        let registry = read_only_registry();
        let result = registry.call("disk_usage", &json!({}), &ctx(&demo_root()), &demo_host());
        assert!(result.ok, "{}", result.summary);
        assert!(result.summary.contains("free of"));
        assert_eq!(result.data["free_bytes"], 700);
    }

    #[test]
    fn memory_search_says_when_it_knows_nothing() {
        struct Empty;
        impl MemoryProvider for Empty {
            fn search(&self, _q: &str, _l: usize) -> Result<Vec<MemoryHit>, String> {
                Ok(Vec::new())
            }
        }
        let registry = read_only_registry();
        let mut ctx = ctx(&demo_root());
        ctx.memory = Some(std::sync::Arc::new(Empty));
        let result = registry.call(
            "memory_search",
            &json!({"query":"anything"}),
            &ctx,
            &demo_host(),
        );
        assert!(result.ok);
        assert!(result.summary.contains("nothing in memory"));
    }

    #[test]
    fn every_read_tool_respects_the_roots_and_the_forbidden_floor() {
        let registry = read_only_registry();
        let host = demo_host();
        let ctx = ctx(&demo_root().join("Downloads"));

        // Outside the root.
        let outside = registry.call("list_dir", &json!({"path":"C:/Windows"}), &ctx, &host);
        assert!(!outside.ok);
        assert!(matches!(outside.error, Some(ToolError::OutOfRoots { .. })));

        // Credentials, even inside a plausible path.
        let secrets = registry.call(
            "read_file",
            &json!({"path": under("Downloads/.ssh/id_rsa")}),
            &ctx,
            &host,
        );
        assert!(!secrets.ok);
        assert!(matches!(secrets.error, Some(ToolError::Forbidden { .. })));

        // Vara's own state.
        let own = registry.call("read_file", &json!({"path":"settings.json"}), &ctx, &host);
        assert!(!own.ok);
        assert!(matches!(own.error, Some(ToolError::Forbidden { .. })));
    }

    #[test]
    fn the_registry_catalogue_lists_every_read_tool_with_a_risk_class() {
        let registry = read_only_registry();
        let catalogue = registry.catalogue();
        for name in [
            "system_info",
            "list_dir",
            "find_files",
            "read_file",
            "disk_usage",
            "memory_search",
        ] {
            assert!(catalogue.contains(name), "catalogue is missing {name}");
        }
        assert_eq!(registry.len(), 6);
        for spec in registry.schemas() {
            assert_eq!(spec["risk"], "R", "read tools must be class R");
        }
    }

    #[test]
    fn a_bad_argument_never_reaches_the_host() {
        struct PanickingHost;
        impl ToolHost for PanickingHost {
            fn read_dir(&self, _p: &Path) -> Result<Vec<HostEntry>, String> {
                panic!("the host must not be touched when validation fails")
            }
            fn read_file(&self, _p: &Path, _m: usize) -> Result<String, String> {
                panic!("the host must not be touched when validation fails")
            }
            fn file_size(&self, _p: &Path) -> Result<u64, String> {
                panic!("the host must not be touched when validation fails")
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
        let registry = read_only_registry();
        let ctx = ctx(&demo_root());
        let host = PanickingHost;
        // Missing required argument.
        let r = registry.call("list_dir", &json!({}), &ctx, &host);
        assert!(!r.ok);
        // Unknown argument.
        let r = registry.call("list_dir", &json!({"path":"x","nope":1}), &ctx, &host);
        assert!(!r.ok);
        // Path outside the roots.
        let r = registry.call(
            "read_file",
            &json!({"path":"C:/Windows/win.ini"}),
            &ctx,
            &host,
        );
        assert!(!r.ok);
    }
}
