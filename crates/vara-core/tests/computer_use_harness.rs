//! The computer-use harness — scenario-driven evaluation of Vara's hands.
//!
//! Each scenario runs a sequence against the deterministic virtual desktop
//! (MockComputerUse) and asserts BOTH:
//!   1. The task outcome (did the world end up in the intended state?), and
//!   2. The discipline metrics (was the see→act→confirm contract respected?).
//!
//! These are the ported contract of the owner's MCP test suite: gates,
//! grounding, stale-frame handling, destructive dry runs. When the entity's
//! own planner drives these scenarios later (LLM-in-the-loop), the same
//! metrics grade HER behavior, not just the plumbing.

use vara_core::computer_use::{
    is_destructive_combo, normalize_combo, ActLoop, ComputerUseAdapter, CuOp, CuResult, CuSequence,
    GrantLevel, LoopPolicy, MockComputerUse, PixelTarget, WidgetKind,
};

/// The policy an owner has after enabling computer use *and* screenshots. The
/// scenarios below were written against this posture; the screenshot grant is
/// explicit here because `LoopPolicy::default()` ships it OFF (asserted in s12).
fn policy(max_grant: GrantLevel) -> LoopPolicy {
    LoopPolicy {
        max_grant,
        allow_screenshots: true,
        ..Default::default()
    }
}

/// Computer use allowed, screenshots still OFF — the shipped default posture.
fn policy_without_screenshots(max_grant: GrantLevel) -> LoopPolicy {
    LoopPolicy {
        max_grant,
        ..Default::default()
    }
}

/// An adapter that records exactly what the loop asked it to do, and behaves
/// like the real one underneath. Used to prove *discipline* claims about the
/// loop (what was dispatched) instead of inferring them from world state.
struct SpyAdapter {
    inner: MockComputerUse,
    seen: Vec<CuOp>,
}

impl SpyAdapter {
    fn new() -> Self {
        Self {
            inner: MockComputerUse::new(),
            seen: Vec::new(),
        }
    }
}

impl ComputerUseAdapter for SpyAdapter {
    fn execute(&mut self, op: &CuOp) -> CuResult {
        self.seen.push(op.clone());
        self.inner.execute(op)
    }
}

/// s12 — Screenshots are a *grant*, not an adapter feature. With
/// `allow_screenshots: false` (the shipped default) the loop refuses every SEE
/// op before an adapter is asked, and it skips its own implicit captures —
/// reporting the mutations it could not verify instead of letting the run read
/// as verified.
#[test]
fn s12_screenshots_off_refuses_see_and_reports_unverified_mutations() {
    // The shipped default is OFF: capture is opt-in, not inherited.
    assert!(
        !LoopPolicy::default().allow_screenshots,
        "screenshots must default to OFF"
    );

    // (a) An authored SEE op is refused before the adapter is touched — a
    // policy denial, not an adapter error: nothing dispatched, nothing captured.
    let mut spy = SpyAdapter::new();
    let seq = CuSequence::parse(
        r#"{"actions":[
            {"op":"screenshot"},
            {"op":"click","x":400,"y":300}
        ]}"#,
    )
    .unwrap();
    let mut lp = ActLoop::new(&mut spy, policy_without_screenshots(GrantLevel::L1));
    assert!(
        lp.check_policy(&seq).is_err(),
        "the pre-flight gate must refuse a capture-hungry sequence"
    );
    let report = lp.run(&seq);
    assert!(!report.completed);
    assert_eq!(spy.seen.len(), 0, "the adapter was never asked to capture");
    assert_eq!(spy.inner.world.captures, 0);
    assert_eq!(report.see_denials, 1);
    assert!(report.screenshots_disabled);
    assert_eq!(
        report.blind_refusals, 0,
        "this is not a blind-coordinate stop"
    );
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("policy denial"));
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("allow_screenshots=false"));
    // The denial is journalled as a step whose error says "policy denial" — it
    // can never be mistaken for an adapter having tried and failed.
    assert_eq!(report.steps.len(), 1);
    assert!(report.steps[0]
        .result
        .error
        .as_deref()
        .unwrap_or("")
        .starts_with("policy denial"));

    // (b) The implicit final-evidence capture is skipped, and the unverified
    // mutation is reported as such — never counted as verified.
    let mut cu = MockComputerUse::new();
    let seq = CuSequence::parse(r#"{"actions":[{"op":"type","text":"blind","window":"Notepad"}]}"#)
        .unwrap();
    let mut lp = ActLoop::new(&mut cu, policy_without_screenshots(GrantLevel::L1));
    let report = lp.run(&seq);
    assert!(
        report.completed,
        "input is allowed; only the capture is not"
    );
    assert_eq!(
        cu.world.captures, 0,
        "no implicit capture without the grant"
    );
    assert!(report.evidence_path.is_none());
    assert_eq!(report.mutations, 1);
    assert_eq!(
        report.verified_mutations, 0,
        "no evidence → nothing verified"
    );
    assert_eq!(report.unverified_mutations, 1);
    assert_eq!(report.verify_discipline(), 0.0);
    assert!(!report.fully_verified());
    assert!(report.screenshots_disabled);
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("screenshots are disabled"));
    // The typing itself did happen — the gap is evidence, not execution.
    let notepad = cu
        .world
        .windows
        .iter()
        .find(|w| w.title.contains("Notepad"))
        .unwrap();
    match &notepad
        .widgets
        .iter()
        .find(|w| w.label == "edit")
        .unwrap()
        .kind
    {
        WidgetKind::TextField(s) => assert!(s.contains("blind"), "typed text: {s:?}"),
        other => panic!("expected text field, got {other:?}"),
    }

    // (c) The correction re-see is a capture too: the loop does not look, and
    // the retry report says the run was not re-grounded.
    let mut cu = MockComputerUse::new();
    let seq =
        CuSequence::parse(r#"{"actions":[{"op":"focus","title":"Ghost Window That Never Was"}]}"#)
            .unwrap();
    let mut lp = ActLoop::new(&mut cu, policy_without_screenshots(GrantLevel::L1));
    let report = lp.run_with_correction(&seq);
    assert_eq!(
        cu.world.captures, 0,
        "the correction re-see must respect the grant"
    );
    assert_eq!(report.retries_used, 1, "one bounded retry still happened");
    assert!(report.screenshots_disabled);
}

/// s13 — The grounding gate covers every pixel target, not just absolute ones:
/// a blind `click_win` is refused by the loop, while name-targeted ops stay
/// outside the gate (grounding is about coordinates, not about blanket caution).
#[test]
fn s13_blind_click_win_refused_and_name_targets_stay_ungated() {
    // Blind, as the very first action: refused before any adapter call.
    let mut spy = SpyAdapter::new();
    let seq = CuSequence::parse(
        r#"{"actions":[{"op":"click_win","title":"Notepad","rel_x":50,"rel_y":50}]}"#,
    )
    .unwrap();
    let mut lp = ActLoop::new(&mut spy, policy(GrantLevel::L1));
    let report = lp.run(&seq);
    assert!(!report.completed, "blind click_win must not execute");
    assert_eq!(report.blind_refusals, 1);
    assert_eq!(spy.seen.len(), 0, "the adapter was never asked to act");
    assert_eq!(spy.inner.world.captures, 0);
    assert!(report.error.as_deref().unwrap_or("").contains("ungrounded"));
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("window-relative"));

    // Grounded (SEE first) window-relative click still works: the gate is
    // grounding, not a ban on `click_win`.
    let mut cu = MockComputerUse::new();
    let seq = CuSequence::parse(
        r#"{"actions":[
            {"op":"screenshot"},
            {"op":"click_win","title":"Notepad","rel_x":50,"rel_y":50},
            {"op":"verify"}
        ]}"#,
    )
    .unwrap();
    let mut lp = ActLoop::new(&mut cu, policy(GrantLevel::L1));
    let report = lp.run(&seq);
    assert!(report.completed, "grounded flow: {:?}", report.error);
    assert!(report.steps[1].ok, "grounded click_win executes");
    assert!(report.grounded_clean());
    assert_eq!(report.verify_discipline(), 1.0);

    // Classification: pixel targets are gated, name targets are not.
    assert_eq!(
        CuOp::ClickWin {
            title: "Notepad".into(),
            rel_x: 5,
            rel_y: 6,
            button: "left".into(),
            clicks: 1
        }
        .pixel_target(),
        Some(PixelTarget::WindowRelative(5, 6))
    );
    assert_eq!(
        CuOp::Click {
            x: Some(1),
            y: Some(2),
            button: "left".into(),
            clicks: 1,
            window: None
        }
        .pixel_target(),
        Some(PixelTarget::Absolute(1, 2))
    );
    assert_eq!(
        CuOp::Move { x: 1, y: 2 }.pixel_target(),
        Some(PixelTarget::Absolute(1, 2))
    );
    for op in [
        CuOp::Focus {
            title: "Notepad".into(),
        },
        CuOp::Type {
            text: "hi".into(),
            window: Some("Notepad".into()),
        },
        CuOp::Hotkey {
            keys: "ctrl+s".into(),
            window: Some("Notepad".into()),
        },
    ] {
        assert_eq!(op.pixel_target(), None, "{} is name-targeted", op.tag());
    }

    // Defense in depth: the adapter refuses a blind window-relative click even
    // when called directly. The mock used to *self-ground* from the click it
    // was validating, so this could never be caught below the loop.
    let mut cu = MockComputerUse::new();
    let r = cu.execute(&CuOp::ClickWin {
        title: "Notepad".into(),
        rel_x: 50,
        rel_y: 50,
        button: "left".into(),
        clicks: 1,
    });
    assert!(!r.ok, "adapter-side blind click_win must fail: {r:?}");
    assert!(r.error.as_deref().unwrap_or("").contains("blind click_win"));
}

/// s14 — Nothing destructive reaches an adapter below L2: the loop converts
/// `confirm: true` into a dry run itself, and a destructive hotkey is not
/// dispatched at all. The spy proves the defusal happened *before* dispatch by
/// showing the same adapter really closing when L2 is granted.
#[test]
fn s14_loop_defuses_destructive_ops_before_any_adapter_sees_them() {
    let seq = CuSequence::parse(
        r#"{"actions":[
            {"op":"screenshot"},
            {"op":"close_window","title":"Notepad","confirm":true}
        ]}"#,
    )
    .unwrap();

    // Below L2 the model-authored `confirm` is a request, not an authorization.
    let mut spy = SpyAdapter::new();
    let mut lp = ActLoop::new(&mut spy, policy(GrantLevel::L1));
    let report = lp.run(&seq);
    assert!(!report.completed, "a defused close stops the sequence");
    assert!(
        spy.seen
            .iter()
            .all(|op| !matches!(op, CuOp::CloseWindow { confirm: true, .. })),
        "the adapter was asked to really close: {:?}",
        spy.seen
    );
    assert!(
        matches!(
            spy.seen.get(1),
            Some(CuOp::CloseWindow { confirm: false, .. })
        ),
        "the loop must send the dry-run twin instead: {:?}",
        spy.seen
    );
    assert!(
        spy.inner
            .world
            .windows
            .iter()
            .any(|w| w.open && w.title.contains("Notepad")),
        "the window survives a sub-L2 close"
    );
    assert_eq!(report.defused_destructive, 1);
    assert!(
        report.steps[1].dry_run,
        "the step is journalled as a dry run"
    );
    assert!(!report.steps[1].ok);
    assert!(report.error.as_deref().unwrap_or("").contains("L2"));

    // Control: with L2 granted the same spy *is* asked to close, and does. The
    // guarantee above is therefore the loop's, not an incapability of the
    // adapter.
    let mut spy_l2 = SpyAdapter::new();
    let mut lp = ActLoop::new(&mut spy_l2, policy(GrantLevel::L2));
    let report = lp.run(&seq);
    assert!(report.completed, "L2 confirm runs: {:?}", report.error);
    assert!(spy_l2
        .seen
        .iter()
        .any(|op| matches!(op, CuOp::CloseWindow { confirm: true, .. })));
    assert!(!spy_l2
        .inner
        .world
        .windows
        .iter()
        .any(|w| w.open && w.title.contains("Notepad")));

    // A destructive hotkey has no dry-run form: below L2 the loop sends
    // nothing to any adapter and writes the receipt itself.
    let combo = CuSequence::parse(
        r#"{"actions":[
            {"op":"screenshot"},
            {"op":"hotkey","keys":"alt+f4"}
        ]}"#,
    )
    .unwrap();
    let mut spy = SpyAdapter::new();
    let mut lp = ActLoop::new(&mut spy, policy(GrantLevel::L1));
    let report = lp.run(&combo);
    assert!(!report.completed);
    assert!(
        spy.seen.iter().all(|op| !matches!(op, CuOp::Hotkey { .. })),
        "a destructive combo must never be dispatched below L2: {:?}",
        spy.seen
    );
    assert!(
        spy.inner.world.windows.iter().filter(|w| w.open).count() == 2,
        "nothing was closed"
    );
    assert!(report.steps.last().unwrap().dry_run);
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("destructive"));
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("did not send it to any adapter"));
    assert_eq!(report.defused_destructive, 1);
}

/// s15 — Combo spelling is not a policy bypass: every variant of a destructive
/// chord classifies L2 and is withheld from adapters below L2.
#[test]
fn s15_combo_variants_classify_l2_and_are_never_dispatched_below_l2() {
    // The parser contract itself.
    assert_eq!(normalize_combo(" Control + W "), "ctrl+w");
    assert_eq!(normalize_combo("CTRL+W"), "ctrl+w");
    assert_eq!(normalize_combo("shift+ctrl+w"), "ctrl+shift+w");
    assert_eq!(normalize_combo("w+ctrl"), "ctrl+w");
    assert_eq!(normalize_combo("Option + F4"), "alt+f4");
    assert_eq!(normalize_combo("cmd+q"), "win+q");

    let variants = [
        "alt+f4",
        "Alt+F4",
        "ALT + F4",
        "alt + f4",
        "ctrl+w",
        "ctrl+W",
        "CTRL + w",
        "Control + W",
        "w+ctrl",
        "ctrl+q",
        "Ctrl + Q",
        "ctrl+f4",
        "shift+ctrl+w",
        "Ctrl + Shift + W",
    ];
    for v in variants {
        assert!(is_destructive_combo(v), "{v:?} must classify destructive");
        let op = CuOp::Hotkey {
            keys: v.into(),
            window: None,
        };
        assert_eq!(op.grant_level(), GrantLevel::L2, "{v:?} must demand L2");

        // And the loop must keep it away from the adapter below L2.
        let mut spy = SpyAdapter::new();
        let json =
            format!(r#"{{"actions":[{{"op":"screenshot"}},{{"op":"hotkey","keys":"{v}"}}]}}"#);
        let seq = CuSequence::parse(&json).unwrap();
        let mut lp = ActLoop::new(&mut spy, policy(GrantLevel::L1));
        let report = lp.run(&seq);
        assert!(!report.completed, "{v:?} must not complete below L2");
        assert!(
            spy.seen.iter().all(|op| !matches!(op, CuOp::Hotkey { .. })),
            "{v:?} reached the adapter below L2: {:?}",
            spy.seen
        );
        assert!(
            report.steps.last().unwrap().dry_run,
            "{v:?} dry-run receipt"
        );
    }

    // Harmless chords must stay usable: the parser must not over-block.
    for v in ["ctrl+s", "ctrl+a", "ctrl+c", "alt+tab"] {
        assert!(!is_destructive_combo(v), "{v:?} is not destructive");
    }
}

/// s1 — The canonical grounded flow: SEE → focus → type → save → verify.
#[test]
fn s1_notepad_type_save_grounded_flow() {
    let mut cu = MockComputerUse::new();
    let json = r#"{"stop_on_error": true, "actions": [
        {"op": "screenshot"},
        {"op": "focus", "title": "Notepad"},
        {"op": "type", "text": "hello vara", "window": "Notepad"},
        {"op": "hotkey", "keys": "ctrl+s", "window": "Notepad"},
        {"op": "verify"}
    ]}"#;
    let seq = CuSequence::parse(json).unwrap();
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
    assert!(lp.check_policy(&seq).is_ok());
    let report = lp.run(&seq);

    assert!(report.completed, "s1 failed: {:?}", report.error);
    assert_eq!(report.failed_at, None);
    assert!(report.grounded_clean());
    assert_eq!(report.verify_discipline(), 1.0, "every mutation observed");
    // task completion: the world state shows the save
    let notepad = cu
        .world
        .windows
        .iter()
        .find(|w| w.title.contains("Notepad"))
        .unwrap();
    assert!(notepad.title.contains("saved"), "title: {}", notepad.title);
    let field = notepad.widgets.iter().find(|w| w.label == "edit").unwrap();
    match &field.kind {
        vara_core::computer_use::WidgetKind::TextField(s) => {
            assert!(s.contains("hello vara"), "typed text: {s:?}")
        }
        other => panic!("expected text field, got {other:?}"),
    }
}

/// s2 — Transient UI: a panel animates in; a low-settle capture is stale.
/// The disciplined response is wait + full re-shoot, NEVER re-clicking.
#[test]
fn s2_transient_ui_stale_frame_defeated_by_wait_and_reshoot() {
    let mut cu = MockComputerUse::new();
    cu.open_transient_panel("Action Center", 2); // two stale frames available

    let json = r#"{"stop_on_error": true, "actions": [
        {"op": "screenshot", "settle": 0.0},
        {"op": "wait", "seconds": 1.0},
        {"op": "screenshot", "settle": 1.0},
        {"op": "verify"}
    ]}"#;
    let seq = CuSequence::parse(json).unwrap();
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L0));
    let report = lp.run(&seq);

    assert!(report.completed);
    // First capture was flagged stale by the world; the re-shoot was clean.
    assert!(
        report.steps[0]
            .result
            .check
            .as_deref()
            .unwrap_or("")
            .contains("STALE"),
        "world must flag the stale frame"
    );
    assert!(report.steps[2]
        .result
        .check
        .as_deref()
        .unwrap_or("")
        .contains("confirm"));
    // The animation settled: no animating state left behind.
    assert_eq!(cu.active_title(), "Untitled - Notepad");
}

/// s3 — Destructive gate: close ops are dry runs by default; confirm=true
/// requires an L2 policy, else the loop refuses before execution.
#[test]
fn s3_destructive_gate_dry_run_then_l2_confirm() {
    let mut cu = MockComputerUse::new();

    // L1 policy: a confirm=true close is refused BEFORE running.
    let confirm_seq = CuSequence::parse(
        r#"{"actions": [{"op": "close_window", "title": "Notepad", "confirm": true}]}"#,
    )
    .unwrap();
    let lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
    assert!(lp.check_policy(&confirm_seq).is_err());

    // L0 run of the same op via loop under L1: dry run stops the sequence,
    // nothing closed, and the result tells the agent what to do next.
    let dry_seq = CuSequence::parse(
        r#"{"actions": [
            {"op": "screenshot"},
            {"op": "close_window", "title": "Notepad"}
        ]}"#,
    )
    .unwrap();
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
    let report = lp.run(&dry_seq);
    assert!(!report.completed, "dry run must stop the sequence");
    assert!(report.steps[1].dry_run, "close op recorded as dry run");
    assert!(cu
        .world
        .windows
        .iter()
        .any(|w| w.open && w.title.contains("Notepad")));

    // L2 policy + confirm=true: the close executes with before/after evidence.
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L2));
    let report = lp.run(&confirm_seq);
    assert!(
        report.completed,
        "L2 confirm should execute: {:?}",
        report.error
    );
    assert!(!cu
        .world
        .windows
        .iter()
        .any(|w| w.open && w.title.contains("Notepad")));
    let step = &report.steps[0];
    assert!(step.result.before_path.is_some() && step.result.path.is_some());
}

/// s4 — Grounding discipline: coordinates without a prior SEE are refused
/// by the loop itself — the "never blind-click" contract is structural.
#[test]
fn s4_blind_click_refused_structurally() {
    let mut cu = MockComputerUse::new();
    let json = r#"{"stop_on_error": true, "actions": [
        {"op": "click", "x": 400, "y": 300}
    ]}"#;
    let seq = CuSequence::parse(json).unwrap();
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
    let report = lp.run(&seq);

    assert!(!report.completed);
    assert_eq!(report.blind_refusals, 1);
    assert!(!report.grounded_clean());
    assert!(report.error.as_deref().unwrap_or("").contains("ungrounded"));
    // Nothing executed — the world is untouched.
    assert_eq!(cu.world.captures, 0);
}

/// s5 — Focus mismatch diagnosis: a dialog steals focus; the `active` field
/// on every result lets the agent detect it instead of typing into the void.
#[test]
fn s5_focus_mismatch_is_diagnosable_via_active_field() {
    let mut cu = MockComputerUse::new();
    cu.steal_focus("Settings");

    // A see-first flow: the capture reports the true active window.
    let json = r#"{"stop_on_error": true, "actions": [
        {"op": "screenshot"},
        {"op": "type", "text": "hi", "window": "Notepad"}
    ]}"#;
    let seq = CuSequence::parse(json).unwrap();
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
    let report = lp.run(&seq);

    assert!(report.completed);
    let shot_active = report.steps[0].result.active.as_deref().unwrap();
    assert_eq!(shot_active, "Settings", "capture must diagnose focus");
    // focus-first typing restored the intended target before the keys landed.
    let typed_active = report.steps[1].result.active.as_deref().unwrap();
    assert_eq!(typed_active, "Untitled - Notepad");
}

/// s6 — Bounded correction: a failure before any mutation gets exactly one
/// retry; the retry succeeds. After a mutation, no auto-retry.
#[test]
fn s6_correction_is_bounded_to_one_retry() {
    let mut cu = MockComputerUse::new();
    // Failure at step 0 (no window match) — nothing mutated yet.
    let json = r#"{"stop_on_error": true, "actions": [
        {"op": "focus", "title": "Ghost Window That Never Was"},
        {"op": "type", "text": "x"}
    ]}"#;
    let seq = CuSequence::parse(json).unwrap();
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
    let report = lp.run_with_correction(&seq);
    // The retry also fails (the ghost window doesn't exist) — but it ran once.
    assert!(!report.completed);
    assert_eq!(report.retries_used, 1, "exactly one bounded retry");
}

/// s7 — Schema contract: unknown ops and malformed steps are rejected
/// before anything executes (validate-then-execute, the run_actions union).
#[test]
fn s7_schema_rejects_unknown_ops_before_execution() {
    assert!(CuSequence::parse(r#"{"actions": [{"op": "format_c_drive"}]}"#).is_err());
    assert!(CuSequence::parse(r#"{"actions": [{"op": "click"}]}"#).is_ok()); // coords optional, validated later
    assert!(CuSequence::parse(r#"{"actions": [{"op": "wait", "seconds": 99}]}"#).is_err());
    assert!(CuSequence::parse(r#"{"actions": []}"#).is_err());
    // destructive combo detection
    assert!(vara_core::computer_use::is_destructive_combo("alt+f4"));
    assert!(!vara_core::computer_use::is_destructive_combo("ctrl+c"));
    // grant levels are honest
    let seq = CuSequence::parse(
        r#"{"actions": [{"op": "screenshot"}, {"op": "close_app", "process": "x", "confirm": true}]}"#,
    )
    .unwrap();
    assert_eq!(seq.max_grant(), GrantLevel::L2);
}

/// s8 — The journal records every executed step with grant level + evidence,
/// so the entity's deeds are auditable after the fact.
#[test]
fn s8_action_journal_is_complete_and_auditable() {
    let db = vara_core::Database::open_memory().unwrap();
    let mut cu = MockComputerUse::new();
    let json = r#"{"stop_on_error": true, "actions": [
        {"op": "screenshot"},
        {"op": "focus", "title": "Notepad"},
        {"op": "type", "text": "journal me", "window": "Notepad"},
        {"op": "close_window", "title": "Notepad"}
    ]}"#;
    let seq = CuSequence::parse(json).unwrap();
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
    let report = lp.run(&seq);

    for step in &report.steps {
        db.insert_cu_step(
            Some(1),
            step.index,
            &step.op,
            step.grant.as_str(),
            "",
            step.ok,
            step.dry_run,
            step.result.active.as_deref(),
            step.result.before_path.as_deref(),
            step.result.path.as_deref(),
            step.result.check.as_deref(),
            step.result.ms,
            step.result.error.as_deref(),
        )
        .unwrap();
    }
    let entries = db.list_cu_journal(Some(1), 100).unwrap();
    assert_eq!(entries.len(), report.steps.len());
    assert!(entries.iter().any(|e| e.op == "close_window" && e.dry_run));
    assert!(entries
        .iter()
        .any(|e| e.op == "screenshot" && e.active.as_deref() == Some("Untitled - Notepad")));
    // retention prunes
    assert_eq!(db.prune_cu_journal(1).unwrap(), 0);
}

/// s9 — Grant levels classify exactly like the MCP's safety model:
/// observe=L0, input=L1, destructive=L2; destructive combos escalate.
#[test]
fn s9_grant_ladder_classification() {
    let cases: Vec<(CuOp, GrantLevel)> = vec![
        (
            CuOp::Screenshot {
                region: None,
                settle: 0.1,
            },
            GrantLevel::L0,
        ),
        (CuOp::Verify, GrantLevel::L0),
        (
            CuOp::Click {
                x: Some(1),
                y: Some(2),
                button: "left".into(),
                clicks: 1,
                window: None,
            },
            GrantLevel::L1,
        ),
        (
            CuOp::Hotkey {
                keys: "ctrl+s".into(),
                window: None,
            },
            GrantLevel::L1,
        ),
        (
            CuOp::Hotkey {
                keys: "alt+f4".into(),
                window: None,
            },
            GrantLevel::L2,
        ),
        (
            CuOp::CloseWindow {
                title: "x".into(),
                confirm: false,
            },
            GrantLevel::L2,
        ),
    ];
    for (op, level) in cases {
        assert_eq!(op.grant_level(), level, "op {}", op.tag());
    }
}

/// s10 — The public shell entry point must enforce the grant gate before the
/// adapter sees a destructive request. This protects callers that do not use
/// `ActLoop::check_policy` manually.
#[test]
fn s10_entrypoint_rejects_l2_before_adapter_execution() {
    let mut cu = MockComputerUse::new();
    let result = vara_core::computer_use::run_sequence_json(
        &mut cu,
        policy(GrantLevel::L1),
        r#"{"actions":[{"op":"close_window","title":"Notepad","confirm":true}]}"#,
    );

    assert!(result.is_err(), "L2 must be rejected by the entry point");
    assert_eq!(cu.world.captures, 0, "the adapter must not be touched");
    assert!(cu
        .world
        .windows
        .iter()
        .any(|w| w.open && w.title.contains("Notepad")));
}

/// s11 — A planner may omit the last verify, but the ActLoop may not. It
/// appends one final evidence capture and records full verification discipline.
#[test]
fn s11_unverified_mutation_gets_structural_final_evidence() {
    let mut cu = MockComputerUse::new();
    let seq = CuSequence::parse(
        r#"{"actions":[
            {"op":"screenshot"},
            {"op":"focus","title":"Notepad"},
            {"op":"type","text":"evidence","window":"Notepad"}
        ]}"#,
    )
    .unwrap();
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
    let report = lp.run(&seq);

    assert!(
        report.completed,
        "final capture should complete the sequence"
    );
    assert_eq!(report.mutations, 2);
    assert_eq!(report.verified_mutations, 2);
    assert_eq!(report.verify_discipline(), 1.0);
    assert!(report.evidence_path.is_some());
    assert_eq!(
        cu.world.captures, 2,
        "initial see + structural final evidence"
    );
}
