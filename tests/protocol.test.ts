// Protocol parser tests — the TypeScript half of the parity lock.
//
// Every expectation is read from the SHARED fixture at
// `tests/fixtures/protocol_cases.json`. The Rust half
// (`crates/vara-core/tests/protocol_parity.rs`) reads the same file inside
// `cargo test -p vara-core`, so a drift on either side fails CI:
//
//   npm run test                       -> this file
//   cargo test -p vara-core            -> protocol_parity.rs
//
// The fixture itself was generated from observing the real Rust output, and
// every value in it is the behaviour both parsers must reproduce exactly.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  MAX_SYS_BLOCKS,
  MISSION_CLOSE,
  MISSION_CLOSE_RE,
  MISSION_GOAL_CAP_BYTES,
  MISSION_OPEN,
  MISSION_OPEN_RE,
  SYS_ACTIONS,
  SYS_BODY_CAP_BYTES,
  SYS_CLOSE,
  SYS_CLOSE_RE,
  SYS_OPEN,
  SYS_OPEN_RE,
  extractMissionGoal,
  extractMissionProposal,
  extractSysActions,
  stripMissionBlock,
  stripProtocolBlocks,
} from "../src/lib/protocol";
import type { SysAction } from "../src/lib/types";

interface FixtureCase {
  name: string;
  note: string;
  input: string;
  /** Display text after the mission stage (what commands.rs feeds the sys stage). */
  clean_after_mission: string;
  /** Display text after both stages — what the shell emits as `content`. */
  clean: string;
  goal: string | null;
  actions: SysAction[];
}

interface Fixture {
  $comment: string;
  cases: FixtureCase[];
}

const FIXTURE_PATH = fileURLToPath(new URL("./fixtures/protocol_cases.json", import.meta.url));
const doc = JSON.parse(readFileSync(FIXTURE_PATH, "utf8")) as Fixture;
const cases = doc.cases;

/** Any known marker variant, built from the exported regexes themselves. */
const MARKER_RE = new RegExp(
  [MISSION_OPEN_RE, MISSION_CLOSE_RE, SYS_OPEN_RE, SYS_CLOSE_RE].map((re) => re.source).join("|"),
  "i",
);

describe("shared fixture", () => {
  it("is the same file the Rust parity test loads", () => {
    expect(FIXTURE_PATH).toMatch(/tests[\\/]fixtures[\\/]protocol_cases\.json$/);
    expect(doc.$comment).toContain("protocol_parity.rs");
    expect(doc.$comment).toContain("protocol.ts");
    expect(cases.length).toBeGreaterThanOrEqual(20);
  });

  it("has a unique name per case", () => {
    const names = cases.map((c) => c.name);
    expect(new Set(names).size).toBe(names.length);
  });
});

describe("protocol constants match chat.rs", () => {
  it("uses the canonical markers", () => {
    expect(MISSION_OPEN).toBe("[[mission]]");
    expect(MISSION_CLOSE).toBe("[[/mission]]");
    expect(SYS_OPEN).toBe("[[sys]]");
    expect(SYS_CLOSE).toBe("[[/sys]]");
  });

  it("keeps the Rust caps", () => {
    expect(MISSION_GOAL_CAP_BYTES).toBe(300);
    expect(SYS_BODY_CAP_BYTES).toBe(400);
    expect(MAX_SYS_BLOCKS).toBe(3);
    expect(SYS_ACTIONS).toEqual(["open_url", "open_path", "run", "screenshot", "computer_use"]);
  });
});

describe("fixture cases (Rust parity)", () => {
  for (const c of cases) {
    describe(c.name, () => {
      it("mission stage reproduces clean_after_mission + goal", () => {
        expect(extractMissionProposal(c.input)).toEqual({
          clean: c.clean_after_mission,
          goal: c.goal,
        });
      });

      it("sys stage reproduces clean + actions", () => {
        expect(extractSysActions(c.clean_after_mission)).toEqual({
          clean: c.clean,
          actions: c.actions,
        });
      });

      it("full shell pipeline (mission then sys) reproduces clean", () => {
        expect(extractSysActions(extractMissionProposal(c.input).clean).clean).toBe(c.clean);
      });

      it("stripProtocolBlocks renders exactly the shell's clean text", () => {
        expect(stripProtocolBlocks(c.input)).toBe(c.clean);
        expect(stripMissionBlock(c.input)).toBe(c.clean);
      });

      it("never shows raw protocol text", () => {
        expect(MARKER_RE.test(c.clean)).toBe(false);
        expect(MARKER_RE.test(stripProtocolBlocks(c.input))).toBe(false);
        expect(c.goal === null || !MARKER_RE.test(c.goal)).toBe(true);
      });

      it("only accepts protocol actions with a usable target", () => {
        for (const a of c.actions) {
          expect(SYS_ACTIONS).toContain(a.action);
          expect(a.target.length).toBeGreaterThan(0);
        }
      });

      it("extractMissionGoal agrees with extractMissionProposal", () => {
        expect(extractMissionGoal(c.input)).toBe(c.goal);
      });
    });
  }
});

describe("regressions that shipped once", () => {
  const caseOf = (name: string): FixtureCase => {
    const found = cases.find((c) => c.name === name);
    if (!found) throw new Error(`fixture case ${name} missing`);
    return found;
  };

  // The divergence this lock exists for: the webview regex was missing
  // [[mission_close]], so the bubble showed the raw marker AND the goal text.
  it("treats [[mission_close]] as a close marker", () => {
    const c = caseOf("mission_double_bracket_close_variant");
    expect(c.input).toContain("[[mission_close]]");

    const { clean, goal } = extractMissionProposal(c.input);
    expect(goal).toBe(c.goal);
    expect(clean).toBe(c.clean_after_mission);
    expect(clean).not.toContain("[[mission_close]]");
    expect(stripProtocolBlocks(c.input)).not.toContain("mission");
  });

  it("treats {MISSION_CLOSE} and single brackets as close markers", () => {
    for (const name of ["mission_brace_close_variant", "mission_single_bracket"]) {
      const c = caseOf(name);
      expect(extractMissionProposal(c.input).clean).toBe(c.clean_after_mission);
      expect(stripProtocolBlocks(c.input)).toBe(c.clean);
    }
  });

  it("drops an invalid sys block without leaking its JSON", () => {
    const c = caseOf("sys_invalid_json_dropped");
    const { actions, clean } = extractSysActions(c.input);
    expect(actions).toEqual([]);
    expect(clean).not.toContain("open_url");
    expect(clean).not.toContain("target");
  });

  it("drops an unknown action without leaking its JSON", () => {
    const c = caseOf("sys_unknown_action_dropped");
    const { actions, clean } = extractSysActions(c.input);
    expect(actions).toEqual([]);
    expect(clean).not.toContain("format_disk");
  });
});

// Rust caps the extractors (one mission block, three sys blocks). The webview
// mirrors those caps in the extractors, but its display-text cleaner sweeps
// until nothing is left, so a pathological reply still cannot leak.
describe("extractor caps vs the UI promise", () => {
  const sysBlock = (n: number) =>
    `[[sys]] {"action":"open_url","target":"https://example.com/${n}"} [[/sys]]`;

  it("processes at most 3 sys blocks, like Rust", () => {
    const reply = `نص.\n${sysBlock(1)}\n${sysBlock(2)}\n${sysBlock(3)}\n${sysBlock(4)}`;
    const { actions, clean } = extractSysActions(reply);
    expect(actions.map((a) => a.target)).toEqual([
      "https://example.com/1",
      "https://example.com/2",
      "https://example.com/3",
    ]);
    expect(clean).toContain("example.com/4"); // the documented cap

    // ...and the display text is still clean.
    expect(stripProtocolBlocks(reply)).toBe("نص.");
  });

  it("extracts only the first mission block, like Rust, but never renders a marker", () => {
    const reply = "نص.\n[[mission]] هدف أول [[/mission]]\n[[mission]] هدف ثانٍ [[/mission]]";
    const { goal, clean } = extractMissionProposal(reply);
    expect(goal).toBe("هدف أول");
    expect(clean).toContain("[[mission]]"); // the documented cap

    const shown = stripProtocolBlocks(reply);
    expect(MARKER_RE.test(shown)).toBe(false);
    expect(shown).toBe("نص.");
  });
});

describe("unterminated blocks (first line only, byte caps)", () => {
  it("takes the first line of an unterminated mission block", () => {
    const reply = "خلاصة.\n[[mission]] أول سطر\nسطر ثانٍ لا يظهر في الهدف.";
    const { goal, clean } = extractMissionProposal(reply);
    expect(goal).toBe("أول سطر");
    expect(clean).toContain("سطر ثانٍ"); // only the goal line is consumed
  });

  it("caps the goal at 300 UTF-8 bytes and floors to a char boundary", () => {
    // Byte 300 lands on a character boundary here.
    const spaced = `ملخص.\n[[mission]] a${"ع".repeat(200)}`;
    const spacedGoal = extractMissionGoal(spaced) as string;
    expect(new TextEncoder().encode(spacedGoal).length).toBe(299);
    expect(spacedGoal.length).toBe(150); // 1 ASCII char + 149 two-byte chars

    // ...and INSIDE a character here: Rust panicked on this shape before the
    // guard in chat.rs, both parsers now floor the cap to 299 bytes.
    const splitting = `ملخص.\n[[mission]]a${"ع".repeat(200)}`;
    const splittingGoal = extractMissionGoal(splitting) as string;
    expect(new TextEncoder().encode(splittingGoal).length).toBe(299);
    expect(splittingGoal.length).toBe(150);
    expect(splittingGoal.startsWith("a")).toBe(true);
    expect(splittingGoal.endsWith("ع")).toBe(true); // never split a character
  });

  it("caps an unterminated sys body at 400 UTF-8 bytes", () => {
    const reply = `سأشغّل أمراً.\n[[sys]] {"action":"run","target":"${"b".repeat(420)}"}`;
    const { actions, clean } = extractSysActions(reply);
    expect(actions).toEqual([]);
    expect(SYS_OPEN_RE.test(clean)).toBe(false);
  });
});
