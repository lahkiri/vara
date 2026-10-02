//! MockComputerUse — a deterministic virtual Windows desktop for headless
//! tests and harness scenarios. It reproduces the *real failure modes* the
//! owner's MCP learned to defeat, so the ActLoop's discipline is earned
//! against them, not asserted:
//!
//! - **Stale frames**: after `open_panel`, captures with `settle < 0.6`
//!   return the pre-animation frame until a full-settle (or a second capture
//!   after a wait) — exactly the transient-UI trap.
//! - **Focus mismatch**: `steal_focus()` simulates a dialog grabbing focus;
//!   results carry `active` so the loop can diagnose it.
//! - **Blind clicks refused**: coordinates never revealed by a SEE op in
//!   this world make the adapter fail the action — including window-relative
//!   `click_win` points, which are pixels like any other; the world never
//!   grounds an action from the action itself.
//! - **Focus-first typing**: typing into a window that isn't focused fails
//!   with guidance, mirroring the real #1 GUI-agent failure.

use super::ops::{is_destructive_combo, normalize_combo, CuOp, CuResult};
use super::ComputerUseAdapter;
use std::collections::HashSet;

pub const SCREEN_W: i32 = 1280;
pub const SCREEN_H: i32 = 800;

#[derive(Debug, Clone)]
pub struct MockWidget {
    /// Absolute screen center of the widget (clickable point).
    pub center: (i32, i32),
    pub label: String,
    /// Text field content / toggle state / button role.
    pub kind: WidgetKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WidgetKind {
    Button,
    TextField(String),
    Toggle(bool),
    StaticText,
}

#[derive(Debug, Clone)]
pub struct MockWindow {
    pub title: String,
    pub bounds: (i32, i32, i32, i32), // x, y, w, h
    pub focused: bool,
    pub minimized: bool,
    pub open: bool,
    pub widgets: Vec<MockWidget>,
    /// The process name this window belongs to (for close_app dry runs).
    pub process: String,
}

#[derive(Debug, Default)]
pub struct MockWorld {
    pub windows: Vec<MockWindow>,
    pub clipboard: String,
    /// Frames captured — count of SEE ops (evidence trail).
    pub captures: usize,
}

/// The adapter + its world; the loop drives this in tests and the harness.
pub struct MockComputerUse {
    pub world: MockWorld,
    /// Coordinates revealed by SEE ops since the last reset — the grounding
    /// set. Clicks outside it are refused (never blind-click).
    seen: HashSet<(i32, i32)>,
    /// Panel currently animating in; captures before `settle` are stale.
    animating: Option<String>,
    /// Pending stale-frame count for the animating panel.
    stale_left: u32,
}

impl Default for MockComputerUse {
    fn default() -> Self {
        Self::new()
    }
}

impl MockComputerUse {
    pub fn new() -> Self {
        let mut world = MockWorld::default();
        // A deterministic starting desktop: Notepad with a text field,
        // Settings with toggles, and a static desktop label.
        world.windows.push(MockWindow {
            title: "Untitled - Notepad".into(),
            bounds: (100, 100, 600, 400),
            focused: true,
            minimized: false,
            open: true,
            process: "notepad.exe".into(),
            widgets: vec![MockWidget {
                center: (400, 300),
                label: "edit".into(),
                kind: WidgetKind::TextField(String::new()),
            }],
        });
        world.windows.push(MockWindow {
            title: "Settings".into(),
            bounds: (720, 120, 480, 360),
            focused: false,
            minimized: false,
            open: true,
            process: "systemsettings.exe".into(),
            widgets: vec![
                MockWidget {
                    center: (960, 200),
                    label: "Dark mode".into(),
                    kind: WidgetKind::Toggle(false),
                },
                MockWidget {
                    center: (960, 260),
                    label: "Notifications".into(),
                    kind: WidgetKind::Toggle(true),
                },
            ],
        });
        Self {
            world,
            seen: HashSet::new(),
            animating: None,
            stale_left: 0,
        }
    }

    /// Simulate another app stealing focus (a dialog, a popup).
    pub fn steal_focus(&mut self, title: &str) {
        for w in &mut self.world.windows {
            w.focused = w.title == title;
        }
    }

    /// A transient panel starts animating: next `stale_for` captures with a
    /// too-small settle return the old frame.
    pub fn open_transient_panel(&mut self, title: &str, stale_for: u32) {
        self.animating = Some(title.into());
        self.stale_left = stale_for;
    }

    pub fn active_title(&self) -> String {
        self.world
            .windows
            .iter()
            .find(|w| w.focused && w.open && !w.minimized)
            .map(|w| w.title.clone())
            .unwrap_or_else(|| "Desktop".into())
    }

    fn find_window(&self, title_sub: &str) -> Option<&MockWindow> {
        let t = title_sub.to_lowercase();
        self.world
            .windows
            .iter()
            .find(|w| w.open && !w.minimized && w.title.to_lowercase().contains(&t))
    }

    fn sees_point(&self, x: i32, y: i32) -> bool {
        self.seen.contains(&(x, y))
    }

    pub fn screenshot(&mut self, region: Option<[i32; 4]>, settle: f32) -> CuResult {
        self.world.captures += 1;
        // Grounding: a full-screen SEE reveals the whole desktop.
        if region.is_none() {
            for w in &self.world.windows {
                if w.open && !w.minimized {
                    let (x, y, wd, ht) = w.bounds;
                    for wx in (x..x + wd).step_by(10) {
                        for wy in (y..y + ht).step_by(10) {
                            self.seen.insert((wx, wy));
                        }
                    }
                }
            }
        }
        let stale = if self.animating.is_some() {
            if settle < 0.6 && self.stale_left > 0 {
                self.stale_left -= 1;
                true
            } else {
                self.animating = None;
                false
            }
        } else {
            false
        };
        if stale {
            return CuResult {
                ok: true,
                op: "screenshot".into(),
                error: None,
                active: Some(self.active_title()),
                path: Some(format!("mock://stale-frame-{}", self.world.captures)),
                before_path: None,
                check: Some(
                    "STALE FRAME risk: a panel was animating; if this contradicts the screen, \
                     wait(0.6-1.0) and re-shoot — never click the opener again"
                        .to_string(),
                ),
                dry_run: None,
                focus: None,
                ms: 40,
                next: Some("wait(1.0) then screenshot() full-screen".into()),
                hint: Some("transient UI: edge-anchored panels clip in narrow regions".into()),
            };
        }
        let region_note = region
            .map(|[x, y, w, h]| format!("region {x},{y} {w}x{h}"))
            .unwrap_or_else(|| "full screen".into());
        CuResult {
            ok: true,
            op: "screenshot".into(),
            error: None,
            active: Some(self.active_title()),
            path: Some(format!("mock://frame-{}", self.world.captures)),
            before_path: None,
            check: Some("confirm the expected UI state in this capture".into()),
            dry_run: None,
            focus: None,
            ms: 35,
            next: Some("grounded: coordinates from this frame are clickable".into()),
            hint: Some(region_note),
        }
    }

    pub fn execute(&mut self, op: &CuOp) -> CuResult {
        let t0 = std::time::Instant::now();
        let r = self.step(op);
        let mut r = r;
        r.ms = t0.elapsed().as_millis() as u64;
        r
    }

    fn step(&mut self, op: &CuOp) -> CuResult {
        match op {
            CuOp::Screenshot { region, settle } => self.screenshot(*region, *settle),
            CuOp::Shot { region, settle } => self.screenshot(*region, *settle),
            CuOp::Verify => CuResult {
                ok: true,
                op: "verify".into(),
                error: None,
                active: Some(self.active_title()),
                path: Some(format!("mock://verify-{}", self.world.captures + 1)),
                before_path: None,
                check: Some("confirm the post-action state matches the plan".into()),
                dry_run: None,
                focus: None,
                ms: 5,
                next: None,
                hint: None,
            },
            CuOp::Focus { title } => match self.find_window(title) {
                Some(w) => {
                    let found = w.title.clone();
                    for win in &mut self.world.windows {
                        win.focused = win.title == found;
                    }
                    CuResult {
                        ok: true,
                        op: "focus".into(),
                        error: None,
                        active: Some(found.clone()),
                        path: None,
                        before_path: None,
                        check: None,
                        dry_run: None,
                        focus: Some(format!("focused: true, title: {found}")),
                        ms: 8,
                        next: None,
                        hint: None,
                    }
                }
                None => CuResult::fail(
                    "focus",
                    format!("no open window matching '{title}' — call list_windows equivalent / screenshot"),
                ),
            },
            CuOp::Click { x, y, .. } => {
                let (Some(x), Some(y)) = (*x, *y) else {
                    return CuResult::fail("click", "no coordinates given and cursor position unknown — take a screenshot first");
                };
                if !self.sees_point(x, y) {
                    return CuResult::fail(
                        "click",
                        format!("blind click blocked at ({x},{y}) — coordinates must come from a capture you actually observed"),
                    );
                }
                let hit = self
                    .world
                    .windows
                    .iter_mut()
                    .filter(|w| w.open && !w.minimized)
                    .flat_map(|w| w.widgets.iter_mut())
                    .find(|wd| (wd.center.0 - x).abs() <= 8 && (wd.center.1 - y).abs() <= 8);
                match hit {
                    Some(w) => {
                        let msg = match &mut w.kind {
                            WidgetKind::Toggle(on) => {
                                *on = !*on;
                                format!("toggled '{}' -> {}", w.label, if *on { "ON" } else { "OFF" })
                            }
                            WidgetKind::TextField(s) => {
                                s.push(' '); // focus lands the caret
                                format!("focused text field '{}'", w.label)
                            }
                            WidgetKind::Button => format!("pressed button '{}'", w.label),
                            WidgetKind::StaticText => format!("selected text '{}'", w.label),
                        };
                        CuResult {
                            ok: true,
                            op: "click".into(),
                            error: None,
                            active: Some(self.active_title()),
                            path: None,
                            before_path: None,
                            check: Some("open verify_path and confirm the expected change".into()),
                            dry_run: None,
                            focus: None,
                            ms: 6,
                            next: Some("verify".into()),
                            hint: Some(msg),
                        }
                    }
                    None => CuResult {
                        ok: true,
                        op: "click".into(),
                        error: None,
                        active: Some(self.active_title()),
                        path: None,
                        before_path: None,
                        check: Some("empty area — confirm nothing unexpected changed".into()),
                        dry_run: None,
                        focus: None,
                        ms: 4,
                        next: None,
                        hint: None,
                    },
                }
            }
            CuOp::ClickWin {
                title,
                rel_x,
                rel_y,
                ..
            } => {
                let (wtitle, wbounds) = match self.find_window(title) {
                    Some(w) => (w.title.clone(), w.bounds),
                    None => {
                        return CuResult::fail(
                            "click_win",
                            format!("no open window matching '{title}'"),
                        )
                    }
                };
                let (wx, wy, ww, wh) = wbounds;
                if *rel_x < 0 || *rel_y < 0 || *rel_x > ww || *rel_y > wh {
                    return CuResult::fail(
                        "click_win",
                        format!("relative coords ({rel_x},{rel_y}) outside window bounds {ww}x{wh}"),
                    );
                }
                let abs = (wx + *rel_x, wy + *rel_y);
                // Window-relative is still a pixel target: a point the model
                // never saw is a blind hit, whether it wrote screen or window
                // coordinates. The old world inserted `abs` into the grounding
                // set here, i.e. it grounded itself from the action it was
                // validating — the exact hole the loop's gate exists to close.
                if !self.sees_point(abs.0, abs.1) {
                    return CuResult::fail(
                        "click_win",
                        format!(
                            "blind click_win blocked: window-relative ({rel_x},{rel_y}) is \
                             absolute ({},{}) and no capture has shown that point",
                            abs.0, abs.1
                        ),
                    );
                }
                self.seen.insert(abs);
                let focused_title = wtitle.clone();
                self.world.windows.iter_mut().for_each(|win| {
                    win.focused = win.title == focused_title;
                });
                let active_after = focused_title.clone();
                CuResult {
                    ok: true,
                    op: "click_win".into(),
                    error: None,
                    active: Some(active_after),
                    path: None,
                    before_path: None,
                    check: Some("confirm the click landed inside the window".into()),
                    dry_run: None,
                    focus: None,
                    ms: 7,
                    next: None,
                    hint: Some(format!("absolute ({},{})", abs.0, abs.1)),
                }
            }
            CuOp::Move { .. } => CuResult {
                ok: true,
                op: "move".into(),
                error: None,
                active: Some(self.active_title()),
                path: None,
                before_path: None,
                check: None,
                dry_run: None,
                focus: None,
                ms: 2,
                next: None,
                hint: None,
            },
            CuOp::Type { text, window } => self.type_text(text, window.as_deref()),
            CuOp::Hotkey { keys, window } => {
                // The chord is canonicalized the same way the loop classifies
                // it: the world must react to the chord the policy judged, not
                // to its spelling ("ALT + F4" closes here, as it does on
                // Windows).
                let combo = normalize_combo(keys);
                if is_destructive_combo(&combo) {
                    // destructive: forced before_path + verify contract
                    if combo == "alt+f4" {
                        let active = self.active_title();
                        self.world.windows.iter_mut().for_each(|w| {
                            if w.focused {
                                w.open = false;
                            }
                        });
                        return CuResult {
                            ok: true,
                            op: "hotkey".into(),
                            error: None,
                            active: Some(self.active_title()),
                            path: Some("mock://after-destructive".into()),
                            before_path: Some("mock://before-destructive".into()),
                            check: Some(format!("DESTRUCTIVE combo '{combo}' fired — confirm '{active}' closed was intended")),
                            dry_run: None,
                            focus: None,
                            ms: 9,
                            next: Some("verify".into()),
                            hint: None,
                        };
                    }
                }
                if combo == "ctrl+a" || combo == "ctrl+c" || combo == "ctrl+v" {
                    if combo == "ctrl+c" {
                        self.world.clipboard = "selected-text".into();
                    }
                    if combo == "ctrl+v" {
                        let pasted = self.world.clipboard.clone();
                        return self.type_text(&pasted, window.as_deref());
                    }
                }
                if combo == "ctrl+s" {
                    let Some(w) = self.world.windows.iter_mut().find(|w| w.focused && w.open)
                    else {
                        return CuResult::fail("hotkey", "no focused window to save");
                    };
                    if w.process == "notepad.exe" {
                        w.title = "saved.txt - Notepad".into();
                    }
                    return CuResult {
                        ok: true,
                        op: "hotkey".into(),
                        error: None,
                        active: Some(w.title.clone()),
                        path: None,
                        before_path: None,
                        check: Some("confirm the save dialog/title change".into()),
                        dry_run: None,
                        focus: None,
                        ms: 9,
                        next: Some("verify".into()),
                        hint: None,
                    };
                }
                CuResult {
                    ok: true,
                    op: "hotkey".into(),
                    error: None,
                    active: Some(self.active_title()),
                    path: None,
                    before_path: None,
                    check: None,
                    dry_run: None,
                    focus: None,
                    ms: 5,
                    next: None,
                    hint: None,
                }
            }
            CuOp::Key { key, window } => {
                if key == "enter" {
                    // Enter on a focused text field commits it (dialog OK).
                    return self.type_text("\n", window.as_deref());
                }
                CuResult {
                    ok: true,
                    op: "key".into(),
                    error: None,
                    active: Some(self.active_title()),
                    path: None,
                    before_path: None,
                    check: None,
                    dry_run: None,
                    focus: None,
                    ms: 3,
                    next: None,
                    hint: None,
                }
            }
            CuOp::Keys { sequence, window } => {
                for k in sequence.split_whitespace() {
                    let r = self.step(&CuOp::Hotkey {
                        keys: k.into(),
                        window: window.clone(),
                    });
                    if !r.ok {
                        return r;
                    }
                }
                CuResult {
                    ok: true,
                    op: "keys".into(),
                    error: None,
                    active: Some(self.active_title()),
                    path: None,
                    before_path: None,
                    check: None,
                    dry_run: None,
                    focus: None,
                    ms: 12,
                    next: None,
                    hint: None,
                }
            }
            CuOp::Scroll { .. } => CuResult {
                ok: true,
                op: "scroll".into(),
                error: None,
                active: Some(self.active_title()),
                path: None,
                before_path: None,
                check: None,
                dry_run: None,
                focus: None,
                ms: 4,
                next: None,
                hint: None,
            },
            CuOp::Wait { .. } => CuResult {
                ok: true,
                op: "wait".into(),
                error: None,
                active: Some(self.active_title()),
                path: None,
                before_path: None,
                check: None,
                dry_run: None,
                focus: None,
                ms: 0,
                next: None,
                hint: None,
            },
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
            CuOp::CloseWindow { title, confirm } => self.close_window(title, *confirm),
            CuOp::CloseApp { process, confirm } => self.close_app(process, *confirm),
        }
    }

    fn type_text(&mut self, text: &str, window: Option<&str>) -> CuResult {
        // Focus-first: if a window was named, it must BE the active window.
        if let Some(wname) = window {
            match self.find_window(wname) {
                None => {
                    return CuResult::fail(
                        "type",
                        format!("window '{wname}' not found/open — restore it first"),
                    )
                }
                Some(w) => {
                    if !w.focused {
                        let t = w.title.clone();
                        for win in &mut self.world.windows {
                            win.focused = win.title == t;
                        }
                    }
                }
            }
        }
        let active = self.active_title();
        let Some(w) = self.world.windows.iter_mut().find(|w| w.focused && w.open) else {
            return CuResult::fail(
                "type",
                format!("nothing focused to type into (active: '{active}') — focus first"),
            );
        };
        let Some(field) = w
            .widgets
            .iter_mut()
            .find(|wd| matches!(wd.kind, WidgetKind::TextField(_)))
        else {
            return CuResult::fail(
                "type",
                format!("window '{}' has no text field to receive input", w.title),
            );
        };
        if let WidgetKind::TextField(s) = &mut field.kind {
            s.push_str(text);
        }
        CuResult {
            ok: true,
            op: "type".into(),
            error: None,
            active: Some(active),
            path: None,
            before_path: None,
            check: Some("confirm the text landed in the expected window".into()),
            dry_run: None,
            focus: None,
            ms: 10,
            next: Some("verify".into()),
            hint: None,
        }
    }

    fn close_window(&mut self, title: &str, confirm: bool) -> CuResult {
        let Some(w) = self.find_window(title) else {
            return CuResult::fail("close_window", format!("no open window matching '{title}'"));
        };
        let found = w.title.clone();
        if !confirm {
            return CuResult {
                ok: false,
                op: "close_window".into(),
                error: Some(format!("dry run — would close '{found}'")),
                active: Some(self.active_title()),
                path: Some("mock://dry-run-preview".into()),
                before_path: Some("mock://before-close".into()),
                check: Some(
                    "confirm this is the right window, then re-run with confirm=true".into(),
                ),
                dry_run: Some(true),
                focus: None,
                ms: 3,
                next: Some("ask the owner (L2 gate) then confirm".into()),
                hint: None,
            };
        }
        self.world.windows.iter_mut().for_each(|w| {
            if w.title == found {
                w.open = false;
            }
        });
        CuResult {
            ok: true,
            op: "close_window".into(),
            error: None,
            active: Some(self.active_title()),
            path: Some("mock://after-close".into()),
            before_path: Some("mock://before-close".into()),
            check: Some(format!("'{found}' closed — verify shot attached")),
            dry_run: None,
            focus: None,
            ms: 15,
            next: None,
            hint: None,
        }
    }

    fn close_app(&mut self, process: &str, confirm: bool) -> CuResult {
        let matches: Vec<String> = self
            .world
            .windows
            .iter()
            .filter(|w| w.open && w.process.eq_ignore_ascii_case(process))
            .map(|w| w.title.clone())
            .collect();
        if matches.is_empty() {
            return CuResult::fail("close_app", format!("no processes matching '{process}'"));
        }
        if !confirm {
            return CuResult {
                ok: false,
                op: "close_app".into(),
                error: Some(format!(
                    "dry run — would kill {} window(s): {}",
                    matches.len(),
                    matches.join(", ")
                )),
                active: Some(self.active_title()),
                path: Some("mock://dry-run-preview".into()),
                before_path: None,
                check: Some("review the PID list, then re-run with confirm=true".into()),
                dry_run: Some(true),
                focus: None,
                ms: 4,
                next: Some("ask the owner (L2 gate) then confirm".into()),
                hint: None,
            };
        }
        self.world.windows.iter_mut().for_each(|w| {
            if w.process.eq_ignore_ascii_case(process) {
                w.open = false;
            }
        });
        CuResult {
            ok: true,
            op: "close_app".into(),
            error: None,
            active: Some(self.active_title()),
            path: Some("mock://after-kill".into()),
            before_path: Some("mock://before-kill".into()),
            check: Some("verify shot attached — confirm intended".into()),
            dry_run: None,
            focus: None,
            ms: 20,
            next: None,
            hint: None,
        }
    }
}

impl ComputerUseAdapter for MockComputerUse {
    fn execute(&mut self, op: &CuOp) -> CuResult {
        self.execute(op)
    }
}
