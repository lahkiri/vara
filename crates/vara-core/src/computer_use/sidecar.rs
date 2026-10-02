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

/// Map one `CuOp` to the sidecar tool call it becomes.
///
/// Pure and total on purpose: this is the seam where Vara's operation language
/// meets the server's tool names, and a wrong mapping here is *invisible* at
/// runtime — the server simply does something other than what the loop
/// validated. Every op is pinned by a unit test below, including the one that
/// was inverted (`type` into a named window must not lose the title).
///
/// Window discipline: an op that names a window must carry it to the tool that
/// acts on windows. `click_win` → `click_window`, `close_window` → the same,
/// `focus` → `focus_window`, and `type` with a window → `type_into_window`;
/// plain `type_text` is reserved for typing into whatever is already focused.
/// Window titles ride the `title_substring`/`window` arguments this adapter
/// already used, never dropped, never widened to "the active window".
///
/// `None` is returned only for `IgnoreErrors`, the loop's control marker: it is
/// not a tool, and must never be sent as one.
fn tool_call_for(op: &CuOp) -> Option<(String, serde_json::Value)> {
    let call = match op {
        CuOp::Screenshot { region, settle } | CuOp::Shot { region, settle } => {
            let mut args = serde_json::json!({"settle": settle});
            if let Some([x, y, w, h]) = region {
                args["region"] = serde_json::json!([x, y, w, h]);
            }
            ("screenshot", args)
        }
        CuOp::Verify => ("screenshot", serde_json::json!({"settle": 0.1})),
        CuOp::Focus { title } => (
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
            ("click", args)
        }
        CuOp::ClickWin {
            title,
            rel_x,
            rel_y,
            button,
            clicks,
        } => (
            "click_window",
            serde_json::json!({
                "title_substring": title, "rel_x": rel_x, "rel_y": rel_y,
                "button": button, "clicks": clicks
            }),
        ),
        CuOp::Move { x, y } => ("move_mouse", serde_json::json!({"x": x, "y": y})),
        // The window title decides which tool is honest here: `type_text`
        // types into the focused window and has no window argument, so an op
        // that names a window must go to `type_into_window` with that title.
        // The old mapping did the opposite (named window → `type_text`, no
        // window → `type_into_window`), which silently dropped the title the
        // loop had validated.
        CuOp::Type { text, window } => match window {
            Some(w) => (
                "type_into_window",
                serde_json::json!({"text": text, "window": w}),
            ),
            None => ("type_text", serde_json::json!({"text": text})),
        },
        CuOp::Hotkey { keys, window } => {
            let mut args = serde_json::json!({"keys": keys});
            if let Some(w) = window {
                args["window"] = serde_json::json!(w);
            }
            ("hotkey", args)
        }
        CuOp::Key { key, window } => {
            let mut args = serde_json::json!({"key": key});
            if let Some(w) = window {
                args["window"] = serde_json::json!(w);
            }
            ("press_key", args)
        }
        CuOp::Keys { sequence, window } => {
            let mut args = serde_json::json!({"keys": sequence});
            if let Some(w) = window {
                args["window"] = serde_json::json!(w);
            }
            ("key_sequence", args)
        }
        CuOp::Scroll { clicks, x, y } => {
            let mut args = serde_json::json!({"clicks": clicks});
            if let (Some(x), Some(y)) = (x, y) {
                args["x"] = serde_json::json!(x);
                args["y"] = serde_json::json!(y);
            }
            ("scroll", args)
        }
        CuOp::Wait { seconds } => ("wait", serde_json::json!({"seconds": seconds})),
        CuOp::IgnoreErrors => return None,
        CuOp::CloseWindow { title, confirm } => (
            "close_window",
            serde_json::json!({"title_substring": title, "confirm": confirm}),
        ),
        CuOp::CloseApp { process, confirm } => (
            "close_app",
            serde_json::json!({"process_name": process, "confirm": confirm}),
        ),
    };
    Some((call.0.to_string(), call.1))
}

/// Receipt for a loop control marker that is not a tool call.
fn no_tool_result(op: &CuOp) -> CuResult {
    CuResult {
        ok: true,
        op: op.tag().into(),
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
    }
}

impl ComputerUseAdapter for SidecarComputerUse {
    fn execute(&mut self, op: &CuOp) -> CuResult {
        match tool_call_for(op) {
            Some((name, args)) => self.call_tool(&name, args),
            // `ignore_errors` is the loop's control marker, handled before
            // dispatch; reaching here means "nothing to do", not "call a tool".
            None => no_tool_result(op),
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

    /// Table row helper: keeps the big mapping table readable.
    fn case(
        op: CuOp,
        tool: &'static str,
        args: serde_json::Value,
    ) -> (CuOp, &'static str, serde_json::Value) {
        (op, tool, args)
    }

    /// Every `CuOp` maps to exactly one tool call with the expected arguments.
    /// A wrong mapping is silent at runtime (the server just does something
    /// else), so this table is the only place the contract is checkable.
    #[test]
    fn tool_call_for_maps_every_op() {
        let cases: Vec<(CuOp, &'static str, serde_json::Value)> = vec![
            case(
                CuOp::Screenshot {
                    region: None,
                    settle: 0.1,
                },
                "screenshot",
                serde_json::json!({"settle": 0.1}),
            ),
            case(
                CuOp::Shot {
                    region: Some([1, 2, 30, 40]),
                    settle: 0.5,
                },
                "screenshot",
                serde_json::json!({"settle": 0.5, "region": [1, 2, 30, 40]}),
            ),
            case(
                CuOp::Verify,
                "screenshot",
                serde_json::json!({"settle": 0.1}),
            ),
            case(
                CuOp::Focus {
                    title: "Notepad".into(),
                },
                "focus_window",
                serde_json::json!({"title_substring": "Notepad"}),
            ),
            case(
                CuOp::Click {
                    x: Some(10),
                    y: Some(20),
                    button: "left".into(),
                    clicks: 1,
                    window: None,
                },
                "click",
                serde_json::json!({"button": "left", "clicks": 1, "x": 10, "y": 20}),
            ),
            case(
                CuOp::Click {
                    x: None,
                    y: None,
                    button: "right".into(),
                    clicks: 2,
                    window: Some("Notepad".into()),
                },
                "click",
                serde_json::json!({"button": "right", "clicks": 2, "window": "Notepad"}),
            ),
            case(
                CuOp::ClickWin {
                    title: "Notepad".into(),
                    rel_x: 5,
                    rel_y: 6,
                    button: "left".into(),
                    clicks: 1,
                },
                "click_window",
                serde_json::json!({
                    "title_substring": "Notepad", "rel_x": 5, "rel_y": 6,
                    "button": "left", "clicks": 1
                }),
            ),
            case(
                CuOp::Move { x: 3, y: 4 },
                "move_mouse",
                serde_json::json!({"x": 3, "y": 4}),
            ),
            case(
                CuOp::Type {
                    text: "hi".into(),
                    window: None,
                },
                "type_text",
                serde_json::json!({"text": "hi"}),
            ),
            case(
                CuOp::Type {
                    text: "hi".into(),
                    window: Some("Notepad".into()),
                },
                "type_into_window",
                serde_json::json!({"text": "hi", "window": "Notepad"}),
            ),
            case(
                CuOp::Hotkey {
                    keys: "ctrl+s".into(),
                    window: None,
                },
                "hotkey",
                serde_json::json!({"keys": "ctrl+s"}),
            ),
            case(
                CuOp::Hotkey {
                    keys: "alt+f4".into(),
                    window: Some("Notepad".into()),
                },
                "hotkey",
                serde_json::json!({"keys": "alt+f4", "window": "Notepad"}),
            ),
            case(
                CuOp::Key {
                    key: "enter".into(),
                    window: None,
                },
                "press_key",
                serde_json::json!({"key": "enter"}),
            ),
            case(
                CuOp::Keys {
                    sequence: "ctrl+a ctrl+c".into(),
                    window: None,
                },
                "key_sequence",
                serde_json::json!({"keys": "ctrl+a ctrl+c"}),
            ),
            case(
                CuOp::Scroll {
                    clicks: -3,
                    x: Some(9),
                    y: Some(8),
                },
                "scroll",
                serde_json::json!({"clicks": -3, "x": 9, "y": 8}),
            ),
            case(
                CuOp::Scroll {
                    clicks: 2,
                    x: None,
                    y: None,
                },
                "scroll",
                serde_json::json!({"clicks": 2}),
            ),
            case(
                CuOp::Wait { seconds: 0.5 },
                "wait",
                serde_json::json!({"seconds": 0.5}),
            ),
            case(
                CuOp::CloseWindow {
                    title: "Notepad".into(),
                    confirm: false,
                },
                "close_window",
                serde_json::json!({"title_substring": "Notepad", "confirm": false}),
            ),
            case(
                CuOp::CloseApp {
                    process: "notepad.exe".into(),
                    confirm: true,
                },
                "close_app",
                serde_json::json!({"process_name": "notepad.exe", "confirm": true}),
            ),
        ];

        for (op, tool, args) in cases {
            let (name, got) = tool_call_for(&op)
                .unwrap_or_else(|| panic!("{} must map to a tool call", op.tag()));
            assert_eq!(name, tool, "tool for op {}", op.tag());
            assert_json_eq(&got, &args, op.tag());
        }

        // The loop's control marker is not a tool: it must never be sent.
        assert!(tool_call_for(&CuOp::IgnoreErrors).is_none());
    }

    /// Compare mapped arguments without tripping over `f32`→`f64` noise:
    /// `settle: 0.1` is serialized as the f32's exact f64 value, so numeric
    /// leaves get a tolerance and everything else is compared exactly.
    fn assert_json_eq(got: &serde_json::Value, want: &serde_json::Value, ctx: &str) {
        match (got, want) {
            (serde_json::Value::Number(g), serde_json::Value::Number(w)) => {
                let g = g.as_f64().unwrap_or(f64::NAN);
                let w = w.as_f64().unwrap_or(f64::NAN);
                assert!((g - w).abs() < 1e-6, "{ctx}: number {g} != {w}");
            }
            (serde_json::Value::Object(g), serde_json::Value::Object(w)) => {
                assert_eq!(g.len(), w.len(), "{ctx}: key count");
                for (k, wv) in w {
                    let gv = g.get(k).unwrap_or_else(|| panic!("{ctx}: missing key {k}"));
                    assert_json_eq(gv, wv, &format!("{ctx}.{k}"));
                }
            }
            (serde_json::Value::Array(g), serde_json::Value::Array(w)) => {
                assert_eq!(g.len(), w.len(), "{ctx}: array length");
                for (i, (gv, wv)) in g.iter().zip(w).enumerate() {
                    assert_json_eq(gv, wv, &format!("{ctx}[{i}]"));
                }
            }
            _ => assert_eq!(got, want, "{ctx}"),
        }
    }

    /// Regression for the inverted mapping: `type_into_window` used to be
    /// called precisely when there was *no* window, and `type_text` received a
    /// window title it would ignore — the window discipline was silently
    /// dropped on exactly the op where focus-first matters most.
    #[test]
    fn type_mapping_never_drops_the_window_title() {
        let (name, args) = tool_call_for(&CuOp::Type {
            text: "hello".into(),
            window: Some("Notepad".into()),
        })
        .unwrap();
        assert_eq!(name, "type_into_window");
        assert_eq!(
            args.get("window").and_then(|w| w.as_str()),
            Some("Notepad"),
            "the validated window title must ride the call"
        );

        let (name, args) = tool_call_for(&CuOp::Type {
            text: "hello".into(),
            window: None,
        })
        .unwrap();
        assert_eq!(name, "type_text");
        assert!(
            args.get("window").is_none(),
            "no window argument when the op named none"
        );
    }
}
