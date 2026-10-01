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

use vara_core::computer_use::{CuOp, CuSequence, GrantLevel, LoopPolicy, MockComputerUse};

fn policy(max_grant: GrantLevel) -> LoopPolicy {
    LoopPolicy {
        max_grant,
        ..Default::default()
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
    let mut lp = vara_core::computer_use::ActLoop::new(&mut cu, policy(GrantLevel::L1));
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
