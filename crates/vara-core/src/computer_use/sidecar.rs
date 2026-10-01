//! SidecarComputerUse — drives the owner's Python computer-use MCP server
//! (frozen with PyInstaller into a Tauri sidecar exe) over MCP stdio.
//!
//! Contract mapping: Vara's `CuOp` language → the server's 38 tools. The
//! server's response conventions (ok/error/active/path/verify_path/check/
//! dry_run/next/hint) pass through into `CuResult` unchanged — response-as-
//! guidance survives the transport.
//!
//! Enable by setting `VARA_CU_SIDECAR` to the sidecar command (e.g. the
//! bundled `vara-cu.exe` or `python path/to/server.py`). Absent or failing
//! spawn yields a clear error — Vara stays honest about missing hands.

use super::ops::{CuOp, CuResult};
use super::ComputerUseAdapter;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub struct SidecarComputerUse {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
    next_id: u64,
    timeout: Duration,
}

impl SidecarComputerUse {
    /// Spawn the sidecar and run the MCP initialize handshake.
    pub fn spawn(command: &str) -> std::result::Result<Self, String> {
        let mut parts = split_command(command);
        let (program, args) = (parts.remove(0), parts);
        let mut child = Command::new(program)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("sidecar spawn '{command}': {e}"))?;

        let stdout = child.stdout.take().ok_or("sidecar stdout missing")?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let stdin = child.stdin.take().ok_or("sidecar stdin missing")?;
        let mut me = Self {
            child,
            stdin,
            rx,
            next_id: 1,
            timeout: Duration::from_secs(30),
        };
        me.initialize()?;
        Ok(me)
    }

    pub fn set_timeout(&mut self, secs: u64) {
        self.timeout = Duration::from_secs(secs);
    }

    fn initialize(&mut self) -> std::result::Result<(), String> {
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "vara", "version": "0.5.0"}
            }
        });
        self.send(&req)?;
        let _resp = self.recv_response(id)?;
        // initialized notification — no response expected
        let note = serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        self.send(&note)
    }

    fn send(&mut self, v: &serde_json::Value) -> std::result::Result<(), String> {
        let line = serde_json::to_string(v).map_err(|e| e.to_string())?;
        writeln!(self.stdin, "{line}").map_err(|e| format!("sidecar write: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("sidecar flush: {e}"))
    }

    fn recv_response(&mut self, id: u64) -> std::result::Result<serde_json::Value, String> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let remain = deadline.saturating_duration_since(Instant::now());
            if remain.is_zero() {
                return Err("sidecar timeout".into());
            }
            match self.rx.recv_timeout(remain) {
                Ok(line) => {
                    let v: serde_json::Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(_) => continue, // non-JSON noise
                    };
                    if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                        if let Some(err) = v.get("error") {
                            return Err(format!("sidecar rpc error: {err}"));
                        }
                        return Ok(v);
                    }
                    // responses for other ids or notifications: skip
                }
                Err(RecvTimeoutError::Timeout) => return Err("sidecar timeout".into()),
                Err(RecvTimeoutError::Disconnected) => return Err("sidecar died".into()),
            }
        }
    }

    fn call_tool(&mut self, name: &str, args: serde_json::Value) -> CuResult {
        let op = name.to_string();
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": args}
        });
        if let Err(e) = self.send(&req) {
            return CuResult::fail(&op, e);
        }
        match self.recv_response(id) {
            Err(e) => CuResult::fail(&op, e),
            Ok(resp) => parse_tool_result(&op, &resp),
        }
    }
}

impl Drop for SidecarComputerUse {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// MCP result shape: {content: [{type:"text", text:"{...}"}], isError?}.
fn parse_tool_result(op: &str, resp: &serde_json::Value) -> CuResult {
    let text = resp
        .pointer("/result/content/0/text")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let is_err = resp
        .pointer("/result/isError")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    if text.is_empty() {
        return CuResult::fail(op, "sidecar returned no content");
    }
    let mut r: CuResult = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            // Non-JSON text: wrap it as a minimal honest result.
            return CuResult {
                ok: !is_err,
                op: op.into(),
                error: if is_err { Some(text.into()) } else { None },
                active: None,
                path: None,
                before_path: None,
                check: None,
                dry_run: None,
                focus: None,
                ms: 0,
                next: None,
                hint: Some(text.chars().take(300).collect()),
            };
        }
    };
    r.op = op.into();
    if is_err && r.ok {
        r.ok = false;
    }
    r
}

impl ComputerUseAdapter for SidecarComputerUse {
    fn execute(&mut self, op: &CuOp) -> CuResult {
        match op {
            CuOp::Screenshot { region, settle } | CuOp::Shot { region, settle } => {
                let mut args = serde_json::json!({"settle": settle});
                if let Some([x, y, w, h]) = region {
                    args["region"] = serde_json::json!([x, y, w, h]);
                }
                self.call_tool("screenshot", args)
            }
            CuOp::Verify => self.call_tool("screenshot", serde_json::json!({"settle": 0.1})),
            CuOp::Focus { title } => self.call_tool(
                "focus_window",
                serde_json::json!({"title_substring": title}),
            ),
            CuOp::Click {
                x,
                y,
                button,
                clicks,
                window,
            } => {
                let mut args = serde_json::json!({"button": button, "clicks": clicks});
                if let (Some(x), Some(y)) = (x, y) {
                    args["x"] = serde_json::json!(x);
                    args["y"] = serde_json::json!(y);
                }
                if let Some(w) = window {
                    args["window"] = serde_json::json!(w);
                }
                self.call_tool("click", args)
            }
            CuOp::ClickWin {
                title,
                rel_x,
                rel_y,
                button,
                clicks,
            } => self.call_tool(
                "click_window",
                serde_json::json!({
                    "title_substring": title, "rel_x": rel_x, "rel_y": rel_y,
                    "button": button, "clicks": clicks
                }),
            ),
            CuOp::Move { x, y } => {
                self.call_tool("move_mouse", serde_json::json!({"x": x, "y": y}))
            }
            CuOp::Type { text, window } => {
                let mut args = serde_json::json!({"text": text});
                if let Some(w) = window {
                    args["window"] = serde_json::json!(w);
                    return self.call_tool("type_text", args);
                }
                self.call_tool("type_into_window", args)
            }
            CuOp::Hotkey { keys, window } => {
                let mut args = serde_json::json!({"keys": keys});
                if let Some(w) = window {
                    args["window"] = serde_json::json!(w);
                }
                self.call_tool("hotkey", args)
            }
            CuOp::Key { key, window } => {
                let mut args = serde_json::json!({"key": key});
                if let Some(w) = window {
                    args["window"] = serde_json::json!(w);
                }
                self.call_tool("press_key", args)
            }
            CuOp::Keys { sequence, window } => {
                let mut args = serde_json::json!({"keys": sequence});
                if let Some(w) = window {
                    args["window"] = serde_json::json!(w);
                }
                self.call_tool("key_sequence", args)
            }
            CuOp::Scroll { clicks, x, y } => {
                let mut args = serde_json::json!({"clicks": clicks});
                if let (Some(x), Some(y)) = (x, y) {
                    args["x"] = serde_json::json!(x);
                    args["y"] = serde_json::json!(y);
                }
                self.call_tool("scroll", args)
            }
            CuOp::Wait { seconds } => {
                self.call_tool("wait", serde_json::json!({"seconds": seconds}))
            }
            CuOp::IgnoreErrors => CuResult {
                ok: true,
                op: "ignore_errors".into(),
                error: None,
                active: None,
                path: None,
                before_path: None,
                check: None,
                dry_run: None,
                focus: None,
                ms: 0,
                next: None,
                hint: None,
            },
            CuOp::CloseWindow { title, confirm } => self.call_tool(
                "close_window",
                serde_json::json!({"title_substring": title, "confirm": confirm}),
            ),
            CuOp::CloseApp { process, confirm } => self.call_tool(
                "close_app",
                serde_json::json!({"process_name": process, "confirm": confirm}),
            ),
        }
    }
}

fn split_command(command: &str) -> Vec<String> {
    // Space-split with quote tolerance — enough for sidecar paths.
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in command.chars() {
        match c {
            '"' => in_q = !in_q,
            ' ' if !in_q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_paths() {
        let v = split_command(r#""C:\Program Files\vara\vara-cu.exe" --flag"#);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], r"C:\Program Files\vara\vara-cu.exe");
    }

    #[test]
    fn parse_tool_result_maps_conventions() {
        let resp = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {"content": [{"type": "text", "text": r#"{"ok":true,"op":"screenshot","active":"Notepad","path":"C:\\s\\a.png","check":"confirm it","ms":12}"#}]}
        });
        let r = parse_tool_result("screenshot", &resp);
        assert!(r.ok);
        assert_eq!(r.active.as_deref(), Some("Notepad"));
        assert!(r.path.is_some());
        assert_eq!(r.check.as_deref(), Some("confirm it"));
    }
}
