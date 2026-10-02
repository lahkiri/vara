//! Protocol parity lock — Rust side.
//!
//! Vara parses the model protocol twice: once in `crates/vara-core/src/chat.rs`
//! (the shell's extraction pipeline) and once in `src/lib/protocol.ts` (the
//! webview, which re-cleans stored replies that still carry raw markers). Those
//! two implementations drifted once already: the webview regex was missing the
//! `[[mission_close]]` variant, so the raw marker plus the goal text leaked into
//! the chat bubble (AGENTS.md invariant #6).
//!
//! Both sides now read the SAME fixture — `tests/fixtures/protocol_cases.json`
//! at the repository root — and both suites assert the exact `clean` text, the
//! extracted `goal` and the extracted actions for every case:
//!
//!   - Rust:  this file (`cargo test -p vara-core`)
//!   - TS:    `tests/protocol.test.ts` (`npm run test`)
//!
//! If either parser changes behaviour without the fixture being updated to the
//! *real* agreed behaviour, CI fails. The expectations in the fixture were
//! produced by this very implementation; every value below is therefore locked
//! to observed Rust output, not to a hand-written guess.

use serde_json::Value;
use std::path::PathBuf;
use vara_core::chat::{extract_mission_proposal, extract_sys_actions};

/// `CARGO_MANIFEST_DIR` is `crates/vara-core`, so the shared fixture lives two
/// levels up — the same file the vitest suite loads.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/protocol_cases.json")
}

fn load_fixture() -> Value {
    let path = fixture_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read shared fixture {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("shared fixture {} is not valid JSON: {e}", path.display()))
}

fn case_list(doc: &Value) -> &Vec<Value> {
    doc.get("cases")
        .and_then(Value::as_array)
        .expect("fixture must be an object with a `cases` array")
}

fn case_by_name<'a>(doc: &'a Value, name: &str) -> &'a Value {
    case_list(doc)
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("fixture is missing the case `{name}`"))
}

/// Every marker variant both parsers claim to tolerate. Mirrors the regexes in
/// `chat.rs` / `protocol.ts`; used here only to assert the UI-facing invariant
/// "the display text never contains raw protocol text".
fn marker_regex() -> regex::Regex {
    regex::Regex::new(concat!(
        r"(?i)\[\[\s*mission\s*\]\]|\[\s*mission\s*\]|\{\{\s*mission\s*\}\}|\{\s*mission\s*\}|",
        r"\[\[\s*/\s*mission\s*\]\]|\[\s*/\s*mission\s*\]|\{\{\s*/\s*mission\s*\}\}|\{\s*/\s*mission\s*\}|",
        r"\{\s*mission_close\s*\}|\[\[\s*mission_close\s*\]\]|",
        r"\[\[\s*sys\s*\]\]|\[\s*sys\s*\]|\{\s*sys_open\s*\}|",
        r"\[\[\s*/\s*sys\s*\]\]|\[\s*/\s*sys\s*\]|\{\s*sys_close\s*\}",
    ))
    .expect("marker regex")
}

/// The fixture must cover the whole protocol matrix, not a happy path.
#[test]
fn fixture_covers_the_protocol_matrix() {
    let doc = load_fixture();
    let cases = case_list(&doc);
    assert!(
        cases.len() >= 20,
        "fixture is too thin: {} cases",
        cases.len()
    );

    for required in [
        // plain replies
        "plain_no_markers",
        "plain_blank_lines_collapsed",
        // mission markers and their mangled variants
        "mission_canonical",
        "mission_brace_close_variant",
        "mission_double_bracket_close_variant",
        "mission_single_bracket",
        "mission_double_brace_close",
        "mission_single_brace_close",
        "mission_empty_goal",
        "mission_unterminated_first_line_only",
        "mission_unterminated_long_ascii_capped_300",
        "mission_unterminated_long_arabic_byte_cap",
        // sys actions
        "sys_open_url",
        "sys_open_path",
        "sys_run",
        "sys_screenshot_empty_target",
        "sys_screenshot_missing_target",
        "sys_computer_use",
        "sys_fenced_json",
        "sys_single_bracket_pair",
        "sys_unterminated_short",
        "sys_unterminated_long_ascii_capped_400",
        "sys_three_blocks",
        "sys_invalid_json_dropped",
        "sys_unknown_action_dropped",
        "sys_open_url_empty_target_dropped",
        // both protocols in one reply
        "mixed_mission_and_sys",
    ] {
        case_by_name(&doc, required);
    }
}

/// The lock: for every case, `extract_mission_proposal` then `extract_sys_actions`
/// must reproduce the fixture exactly — the same order the shell uses in
/// `src-tauri/src/commands.rs`.
#[test]
fn rust_reproduces_every_fixture_expectation() {
    let doc = load_fixture();
    let mut checked = 0usize;

    for case in case_list(&doc) {
        let name = case["name"].as_str().unwrap_or("<unnamed>");
        let input = case["input"].as_str().expect("case.input");

        let (after_mission, goal) = extract_mission_proposal(input);
        assert_eq!(
            after_mission,
            case["clean_after_mission"]
                .as_str()
                .expect("clean_after_mission"),
            "[{name}] mission-stage clean text"
        );
        assert_eq!(
            goal.as_deref(),
            case["goal"].as_str(),
            "[{name}] extracted goal"
        );

        let (clean, actions) = extract_sys_actions(&after_mission);
        assert_eq!(
            clean,
            case["clean"].as_str().expect("clean"),
            "[{name}] sys-stage clean text"
        );

        let expected: Vec<(String, String)> = case["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .map(|a| {
                (
                    a["action"].as_str().expect("action").to_string(),
                    a["target"].as_str().expect("target").to_string(),
                )
            })
            .collect();
        let actual: Vec<(String, String)> = actions
            .iter()
            .map(|a| (a.action.clone(), a.target.clone()))
            .collect();
        assert_eq!(actual, expected, "[{name}] extracted actions");

        // Invariant #6: the text the UI renders must not carry protocol syntax.
        assert!(
            !marker_regex().is_match(&clean),
            "[{name}] raw protocol text leaked into display text: {clean:?}"
        );
        checked += 1;
    }

    assert!(checked >= 20, "only {checked} cases were checked");
}

/// The bug this whole lock exists for: `[[mission_close]]` is a close marker.
/// The webview regex was missing it, so the bubble showed the raw marker and
/// the goal text; Rust has always accepted it.
#[test]
fn double_bracket_mission_close_variant_is_extracted() {
    let doc = load_fixture();
    let case = case_by_name(&doc, "mission_double_bracket_close_variant");
    let input = case["input"].as_str().unwrap();

    let (clean, goal) = extract_mission_proposal(input);
    assert_eq!(clean, case["clean_after_mission"].as_str().unwrap());
    assert!(goal.is_some());
    assert!(!clean.contains("mission"), "marker leaked: {clean:?}");
    assert!(!marker_regex().is_match(&clean));
}

/// Caps, spelled out as executable documentation (the same numbers are asserted
/// on the TypeScript side in `tests/protocol.test.ts`).
#[test]
fn unterminated_caps_are_bytes_and_stay_on_char_boundaries() {
    // No newline: the goal is the first 300 bytes...
    let ascii = format!("خلاصة.\n[[mission]] {}", "a".repeat(400));
    let (clean, goal) = extract_mission_proposal(&ascii);
    let goal = goal.expect("goal");
    assert_eq!(
        goal.len(),
        299,
        "one byte of the cap is the separating space"
    );
    assert!(goal.chars().all(|c| c == 'a'));
    assert_eq!(clean, format!("خلاصة.\n{}", "a".repeat(400 - 299)));

    // ...floored to a UTF-8 char boundary when the text is multi-byte. The
    // fixture carries both shapes: one where byte 300 is a boundary, and one
    // where it splits a character (that one PANICKED before the boundary guard
    // in chat.rs: "byte index 300 is not a char boundary").
    let arabic = format!("ملخص.\n[[mission]] a{}", "ع".repeat(200));
    let (clean, goal) = extract_mission_proposal(&arabic);
    let goal = goal.expect("goal");
    assert_eq!(goal.len(), 299, "1 + 149 two-byte chars, the space trimmed");
    assert_eq!(goal.chars().count(), 150);
    assert!(goal.ends_with('ع'));
    assert!(!marker_regex().is_match(&clean));

    let splitting = format!("ملخص.\n[[mission]]a{}", "ع".repeat(200));
    let (clean, goal) = extract_mission_proposal(&splitting);
    let goal = goal.expect("goal");
    assert_eq!(goal.len(), 299);
    assert_eq!(goal.chars().count(), 150);
    assert!(goal.starts_with('a') && goal.ends_with('ع'));
    assert!(!marker_regex().is_match(&clean));

    // Unterminated sys block: the cap is 400 bytes.
    let sys = format!(
        "سأشغّل أمراً.\n[[sys]] {{\"action\":\"run\",\"target\":\"{}\"}}",
        "b".repeat(420)
    );
    let (clean, actions) = extract_sys_actions(&sys);
    assert!(actions.is_empty(), "the cut JSON must not parse");
    assert!(!marker_regex().is_match(&clean));
}

/// The sys extractor processes at most three blocks per reply; the fourth stays
/// in the display text. The webview mirrors the cap but its display-text
/// cleaner sweeps until no marker is left (see `stripProtocolBlocks` in
/// `src/lib/protocol.ts`), so a pathological reply cannot leak in a bubble.
#[test]
fn at_most_three_sys_blocks_are_processed() {
    let block = |n: usize| {
        format!(
            "[[sys]] {{\"action\":\"open_url\",\"target\":\"https://example.com/{n}\"}} [[/sys]]"
        )
    };
    let reply = format!(
        "نص.\n{}\n{}\n{}\n{}",
        block(1),
        block(2),
        block(3),
        block(4)
    );

    let (clean, actions) = extract_sys_actions(&reply);
    assert_eq!(actions.len(), 3);
    assert_eq!(actions[0].target, "https://example.com/1");
    assert_eq!(actions[2].target, "https://example.com/3");
    assert!(
        clean.contains("example.com/4"),
        "documented cap: the 4th block stays in Rust's clean text: {clean:?}"
    );
}
