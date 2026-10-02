/**
 * Vara model-protocol parsing — SINGLE SOURCE OF TRUTH for the webview.
 *
 * The model is asked to emit two blocks:
 *   - a mission proposal:  `[[mission]] <goal> [[/mission]]`
 *   - an OS action:        `[[sys]] {"action":"open_url","target":"..."} [[/sys]]`
 *
 * Models mangle those markers constantly (`[mission] … {MISSION_CLOSE}`,
 * `[[mission_close]]`, fenced JSON, missing close marker). Parsing is therefore
 * deliberately tolerant, and the UI must NEVER leak raw protocol text
 * (AGENTS.md invariants #4 and #6).
 *
 * This module mirrors `crates/vara-core/src/chat.rs` exactly — same regexes,
 * same caps, same "first line only" rule, same 3-block sys cap, same
 * code-fence tolerance, same blank-line tidy-up. The two implementations are
 * locked together by a shared fixture:
 *
 *   - fixture: `tests/fixtures/protocol_cases.json` (repo root, shared)
 *   - Rust lock: `crates/vara-core/tests/protocol_parity.rs`
 *   - TS lock:   `tests/protocol.test.ts`
 *
 * If you change a rule here you must change `chat.rs` the same way (or the
 * fixture first), otherwise CI fails — that divergence already shipped once.
 *
 * No Svelte runes, no imports from `state.svelte.ts`: pure functions only, so
 * they can be unit-tested in plain Node.
 */

import type { SysAction } from "./types";

// ---------- markers (mirror of chat.rs MISSION_OPEN / MISSION_CLOSE) ----------

export const MISSION_OPEN = "[[mission]]";
export const MISSION_CLOSE = "[[/mission]]";
export const SYS_OPEN = "[[sys]]";
export const SYS_CLOSE = "[[/sys]]";

/**
 * Known OPEN variants: `[[mission]]`, `[mission]`, `{{mission}}`, `{mission}`.
 * Case-insensitive and whitespace-tolerant, exactly like chat.rs.
 */
export const MISSION_OPEN_RE =
  /\[\[\s*mission\s*\]\]|\[\s*mission\s*\]|\{\{\s*mission\s*\}\}|\{\s*mission\s*\}/i;

/**
 * Known CLOSE variants: `[[/mission]]`, `[/mission]`, `{{/mission}}`,
 * `{/mission}`, `{mission_close}` and — the variant that used to leak into the
 * chat bubble — `[[mission_close]]`.
 */
export const MISSION_CLOSE_RE =
  /\[\[\s*\/\s*mission\s*\]\]|\[\s*\/\s*mission\s*\]|\{\{\s*\/\s*mission\s*\}\}|\{\s*\/\s*mission\s*\}|\{\s*mission_close\s*\}|\[\[\s*mission_close\s*\]\]/i;

/** Known sys OPEN variants: `[[sys]]`, `[sys]`, `{sys_open}`. */
export const SYS_OPEN_RE = /\[\[\s*sys\s*\]\]|\[\s*sys\s*\]|\{\s*sys_open\s*\}/i;

/** Known sys CLOSE variants: `[[/sys]]`, `[/sys]`, `{sys_close}`. */
export const SYS_CLOSE_RE = /\[\[\s*\/\s*sys\s*\]\]|\[\s*\/\s*sys\s*\]|\{\s*sys_close\s*\}/i;

/** Actions the shell will accept (everything else is dropped). */
export const SYS_ACTIONS = ["open_url", "open_path", "run", "screenshot", "computer_use"];

/** Unterminated mission block: first line only, else at most 300 UTF-8 bytes. */
export const MISSION_GOAL_CAP_BYTES = 300;
/** Unterminated sys block: first line only, else at most 400 UTF-8 bytes. */
export const SYS_BODY_CAP_BYTES = 400;
/** At most 3 sys blocks per reply — one is the norm, three is generous. */
export const MAX_SYS_BLOCKS = 3;
/** Safety-net sweeps in `stripProtocolBlocks` (see the note there). */
const MAX_STRIP_PASSES = 4;

export interface MissionExtraction {
  /** Reply text with the mission block removed (Rust's `clean`). */
  clean: string;
  /** Proposed goal, or null when there is no block / the goal is empty. */
  goal: string | null;
}

export interface SysExtraction {
  /** Reply text with every extracted sys block removed (Rust's `clean`). */
  clean: string;
  /** Accepted actions, in the order they appeared. */
  actions: SysAction[];
}

// ---------- public API ----------

/**
 * Extracts the mission proposal from a chat reply: `(clean, goal)`.
 * Mirror of `vara_core::chat::extract_mission_proposal`.
 *
 * Only the FIRST block is extracted (Rust does not loop here). An unterminated
 * block contributes its first line only, capped at 300 UTF-8 bytes when there
 * is no newline; the lines after that line stay in the display text.
 */
export function extractMissionProposal(reply: string): MissionExtraction {
  const open = MISSION_OPEN_RE.exec(reply);
  if (!open) return { clean: reply.trim(), goal: null };

  const afterOpen = reply.slice(open.index + open[0].length);
  const close = MISSION_CLOSE_RE.exec(afterOpen);

  let goal: string;
  let consumedTo: number;
  if (close) {
    // body ends where the close marker STARTS; removal ends where it ENDS
    goal = afterOpen.slice(0, close.index).trim();
    consumedTo = open.index + open[0].length + close.index + close[0].length;
  } else {
    // Unterminated block: take the first line after the marker only.
    const lineEnd = firstLineEnd(afterOpen, MISSION_GOAL_CAP_BYTES);
    goal = afterOpen.slice(0, lineEnd).trim();
    consumedTo = open.index + open[0].length + lineEnd;
  }

  const clean = collapseBlankLines(reply.slice(0, open.index) + reply.slice(consumedTo));
  return goal ? { clean, goal } : { clean, goal: null };
}

/**
 * Extracts OS-action proposals (`[[sys]] {json} [[/sys]]`) from a chat reply:
 * `(clean, actions)`. Mirror of `vara_core::chat::extract_sys_actions`.
 *
 * At most `MAX_SYS_BLOCKS` blocks are processed; a block is always removed from
 * the display text, even when its JSON is invalid or its action is rejected.
 * `screenshot` may omit the target and is normalized to the literal `screen`.
 */
export function extractSysActions(reply: string): SysExtraction {
  const actions: SysAction[] = [];
  let clean = reply;

  for (let i = 0; i < MAX_SYS_BLOCKS; i++) {
    const open = SYS_OPEN_RE.exec(clean);
    if (!open) break;
    const after = clean.slice(open.index + open[0].length);
    const close = SYS_CLOSE_RE.exec(after);

    // body ends where the close marker STARTS; removal ends where it ENDS
    let bodyTo: number;
    let consumedTo: number;
    if (close) {
      bodyTo = open.index + open[0].length + close.index;
      consumedTo = open.index + open[0].length + close.index + close[0].length;
    } else {
      const cut = open.index + open[0].length + firstLineEnd(after, SYS_BODY_CAP_BYTES);
      bodyTo = cut;
      consumedTo = cut;
    }

    const jsonText = stripCodeFences(clean.slice(open.index + open[0].length, bodyTo));
    const action = parseSysAction(jsonText);
    if (action) actions.push(action);

    clean = clean.slice(0, open.index) + clean.slice(consumedTo);
  }

  return { clean: collapseBlankLines(clean), actions };
}

/**
 * Convenience wrapper used by the chat UI: the proposed goal, or null.
 * Identical semantics to `extractMissionProposal(reply).goal`.
 */
export function extractMissionGoal(content: string): string | null {
  return extractMissionProposal(content).goal;
}

/**
 * Display-text cleaner: the exact pipeline the shell runs after a reply
 * (`src-tauri/src/commands.rs` extracts the mission first, then the sys
 * blocks), so a rendered bubble shows what the backend emitted as `content`.
 *
 * One deliberate addition: the extractors stop after the first mission block
 * and after three sys blocks (Rust's caps), so a pathological reply could keep
 * a raw marker. The UI promise is stronger than the extractor caps — this
 * sweeps while any known marker remains, so raw protocol text can never reach
 * the bubble.
 */
export function stripProtocolBlocks(content: string): string {
  let text = stripOnce(content);
  for (let i = 0; i < MAX_STRIP_PASSES && hasProtocolMarkers(text); i++) {
    const next = stripOnce(text);
    if (next === text) break;
    text = next;
  }
  return text;
}

/**
 * Kept for API compatibility with the old `state.svelte.ts` helper: it always
 * stripped both protocols, and still does.
 */
export function stripMissionBlock(content: string): string {
  return stripProtocolBlocks(content);
}

// ---------- internals (line-by-line ports of chat.rs) ----------

function stripOnce(content: string): string {
  return extractSysActions(extractMissionProposal(content).clean).clean;
}

function hasProtocolMarkers(content: string): boolean {
  return (
    MISSION_OPEN_RE.test(content) ||
    MISSION_CLOSE_RE.test(content) ||
    SYS_OPEN_RE.test(content) ||
    SYS_CLOSE_RE.test(content)
  );
}

/**
 * Port of `strip_code_fences`: tolerates a ```json or ``` fence around the body.
 * Case-sensitive, like the Rust version.
 */
function stripCodeFences(s: string): string {
  let t = s.trim();
  if (t.startsWith("```json")) t = t.slice("```json".length);
  else if (t.startsWith("```")) t = t.slice("```".length);
  if (t.endsWith("```")) t = t.slice(0, -"```".length);
  return t.trim();
}

/**
 * Port of the `after.find('\n').unwrap_or(after.len().min(cap))` rule: the first
 * line when there is one, otherwise at most `cap` UTF-8 bytes — floored to a
 * character boundary, exactly like Rust's string slicing (which would panic on
 * a non-boundary index).
 */
function firstLineEnd(s: string, capBytes: number): number {
  const nl = s.indexOf("\n");
  if (nl >= 0) return nl;
  return utf8CapIndex(s, capBytes);
}

/**
 * Largest index whose UTF-8 encoding fits in `maxBytes`.
 * `codePointAt` + `slice` are code-unit based, so a surrogate pair counts once.
 */
function utf8CapIndex(s: string, maxBytes: number): number {
  let bytes = 0;
  let i = 0;
  while (i < s.length) {
    const cp = s.codePointAt(i) as number;
    const size = cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
    if (bytes + size > maxBytes) break;
    bytes += size;
    i += cp > 0xffff ? 2 : 1;
  }
  return i;
}

/**
 * Port of `parse` inside `extract_sys_actions`: valid JSON, an action on the
 * allow-list, a non-empty target unless the action is `screenshot`.
 */
function parseSysAction(jsonText: string): SysAction | null {
  let value: unknown;
  try {
    value = JSON.parse(jsonText.trim());
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null) return null;

  const raw = value as { action?: unknown; target?: unknown };
  const action = typeof raw.action === "string" ? raw.action : "";
  const target = (typeof raw.target === "string" ? raw.target : "").trim();

  if (!SYS_ACTIONS.includes(action)) return null;
  if (target === "" && action !== "screenshot") return null;
  // "screenshot" needs no target — normalize it so the receipt, the approval
  // card and the dedup key stay stable.
  return { action, target: target === "" ? "screen" : target };
}

/**
 * Port of `collapse_blank_lines`: no more than one blank line in a row, and the
 * result is trimmed. Uses Rust's `str::lines()` semantics (a trailing newline
 * adds no empty line, a single trailing \r per line is dropped).
 */
function collapseBlankLines(s: string): string {
  const parts = s.split("\n");
  if (parts.length > 0 && parts[parts.length - 1] === "") parts.pop();

  let out = "";
  let blanks = 0;
  for (const part of parts) {
    const line = part.endsWith("\r") ? part.slice(0, -1) : part;
    if (line.trim() === "") {
      blanks += 1;
      if (blanks > 1) continue;
    } else {
      blanks = 0;
    }
    out += `${line}\n`;
  }
  return out.trim();
}
