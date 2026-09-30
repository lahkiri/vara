// ============================================================
// aggregate_v2.mjs — تجميع حملة v2 + تطبيق قواعد القرار المسبقة
// المواصفة: docs/experiments/v2-design/EXPERIMENT_DESIGN.md §8
//
// الاستخدام:
//   node aggregate_v2.mjs <runs-root>            → table.md + decisions.json
//   node aggregate_v2.mjs <runs-root> --anonymize → + blind/ + blind_key.json
//
// التشغيلات متوقعة في مجلدات: <arm>_<mission>_<n>/ (مثال: D_m1_3)
// كل مجلد يحتوي: report.md, ledger.json, checker.json, run.json, findings.json
// ذراع بأقل من MIN_RUNS → الحكم UNDECIDED_INSUFFICIENT_DATA (لا يُحسم القرار).
// ============================================================
import fs from "node:fs";
import path from "node:path";

const MIN_RUNS = 3;
const args = process.argv.slice(2);
const root = args.find((a) => !a.startsWith("--"));
const anonymize = args.includes("--anonymize");
if (!root || !fs.existsSync(root)) {
  console.error("usage: node aggregate_v2.mjs <runs-root> [--anonymize]");
  process.exit(2);
}

const ARMS = ["A", "C", "B", "D"];
const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const n = s.length;
  if (!n) return null;
  return n % 2 ? s[(n - 1) / 2] : (s[n / 2 - 1] + s[n / 2]) / 2;
};
const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);

// ---------- collect runs ----------
const runs = [];
for (const dir of fs.readdirSync(root, { withFileTypes: true })) {
  if (!dir.isDirectory()) continue;
  const m = dir.name.match(/^(?:smoke_)?([A-D])_(?:(m\d)_)?(\d+)$/);
  if (!m) continue;
  const d = path.join(root, dir.name);
  const read = (f) => {
    try {
      return JSON.parse(fs.readFileSync(path.join(d, f), "utf8"));
    } catch {
      return null;
    }
  };
  const checker = read("checker.json");
  const run = read("run.json") ?? {};
  const ledger = read("ledger.json") ?? {};
  const report = (() => {
    try {
      return fs.readFileSync(path.join(d, "report.md"), "utf8");
    } catch {
      return "";
    }
  })();
  const tokens =
    run.tokens_used ??
    run.budget_used ??
    (Array.isArray(ledger.calls)
      ? ledger.calls.reduce((a, c) => a + (c.tokens ?? c.total_tokens ?? 0), 0)
      : null);
  runs.push({
    id: dir.name,
    arm: m[1],
    mission: m[2] ?? "mx",
    backed_ratio: checker?.metrics?.backed_ratio ?? null,
    verdict: checker?.verdict ?? null,
    tokens,
    report,
  });
}

if (!runs.length) {
  console.error("no runs found (expected <arm>_<mission>_<n> folders)");
  process.exit(1);
}

// ---------- per-arm / per-mission stats ----------
const byArm = {};
for (const a of ARMS) {
  const rs = runs.filter((r) => r.arm === a);
  byArm[a] = {
    runs: rs.length,
    m1_median: median(rs.map((r) => r.backed_ratio).filter((x) => x !== null)),
    m3_median: median(rs.map((r) => r.tokens).filter((x) => x !== null)),
    pass_rate: rs.length ? rs.filter((r) => r.verdict === "PASS").length / rs.length : null,
  };
}
const byArmMission = {};
for (const a of ARMS)
  for (const mi of ["m1", "m2"]) {
    const rs = runs.filter((r) => r.arm === a && r.mission === mi);
    byArmMission[`${a}_${mi}`] = {
      runs: rs.length,
      m1: rs.map((r) => r.backed_ratio),
      m3: rs.map((r) => r.tokens),
    };
  }

// ---------- pre-registered decision rules (frozen) ----------
function decide() {
  const d = {};
  const insufficient = (arm) => byArm[arm].runs < MIN_RUNS || byArm[arm].m1_median === null;

  // R1 — D vs C (parallel organization question)
  if (insufficient("D") || insufficient("C")) {
    d.R1 = { decision: "UNDECIDED_INSUFFICIENT_DATA", rule: "M2(D) ≥ M2(C)+1.0 AND M3(D) ≤ 1.5×M3(C)" };
  } else {
    const m2d_ok = null; // M2 (blind effective coverage) requires human scoring — flagged, not auto
    d.R1 = {
      decision: "NEEDS_M2_BLIND_SCORE",
      note: "M1/M3 computed; M2 (blind coverage) must be scored before final verdict.",
      m3_ratio: byArm.D.m3_median && byArm.C.m3_median ? +(byArm.D.m3_median / byArm.C.m3_median).toFixed(3) : null,
      cost_condition_met: byArm.D.m3_median && byArm.C.m3_median ? byArm.D.m3_median <= 1.5 * byArm.C.m3_median : null,
    };
    void m2d_ok;
  }

  // R2 — channel separation (C vs A) on M1 (automated)
  if (insufficient("C") || insufficient("A")) {
    d.R2 = { decision: "UNDECIDED_INSUFFICIENT_DATA", rule: "median(M1(A)) ≥ 0.95 → prompt-only; < 0.8 AND M1(C)≈1.0 AND cost ≤ 1.15× → clean writer becomes a permanent component" };
  } else {
    const mA = byArm.A.m1_median;
    const mC = byArm.C.m1_median;
    if (mA >= 0.95) d.R2 = { decision: "NO_CHANNEL_SPLIT_NEEDED", median_M1_A: mA };
    else if (mA < 0.8 && mC >= 0.95) {
      const ratio = byArm.C.m3_median / byArm.A.m3_median;
      d.R2 = {
        decision: ratio <= 1.15 ? "ADOPT_CLEAN_WRITER" : "ADOPT_REJECTED_COST",
        median_M1_A: mA,
        median_M1_C: mC,
        cost_ratio: +ratio.toFixed(3),
      };
    } else d.R2 = { decision: "GRAY_ZONE", median_M1_A: mA, median_M1_C: mC };
  }

  // R3 — model sensitivity: needs the sensitivity run (A_m1_S)
  const sens = runs.find((r) => r.arm === "A" && r.mission === "m1" && /s/i.test(r.id));
  d.R3 = sens
    ? { decision: "SENSITIVITY_RUN_PRESENT", run_id: sens.id, backed_ratio: sens.backed_ratio }
    : { decision: "NO_SENSITIVITY_RUN" };
  return d;
}

// ---------- outputs ----------
const lines = [
  "# v2 campaign — M1/M3 per arm",
  "",
  "| arm | runs | M1 median (backed_ratio) | M3 median (tokens) | PASS rate |",
  "|---|---|---|---|---|",
  ...ARMS.map(
    (a) =>
      `| ${a} | ${byArm[a].runs} | ${byArm[a].m1_median ?? "—"} | ${byArm[a].m3_median ?? "—"} | ${
        byArm[a].pass_rate === null ? "—" : Math.round(byArm[a].pass_rate * 100) + "%"
      } |`
  ),
  "",
  "## per arm × mission",
  "",
  "| cell | runs | M1 | M3 |",
  "|---|---|---|---|",
  ...Object.entries(byArmMission).map(
    ([k, v]) => `| ${k} | ${v.runs} | ${v.m1.join(", ") || "—"} | ${v.m3.join(", ") || "—"} |`
  ),
  "",
];
fs.writeFileSync(path.join(root, "table.md"), lines.join("\n"));
fs.writeFileSync(path.join(root, "decisions.json"), JSON.stringify(decide(), null, 2));

// ---------- anonymize (blind review prep) ----------
if (anonymize) {
  const blindDir = path.join(root, "blind");
  fs.mkdirSync(blindDir, { recursive: true });
  const key = {};
  const shuffled = [...runs].sort(() => Math.random() - 0.5);
  shuffled.forEach((r, i) => {
    const id = "RPT-" + String(i + 1).padStart(2, "0");
    key[id] = { run: r.id, arm: r.arm, mission: r.mission };
    let txt = r.report
      .replace(/^#.*$/gm, "")
      .replace(/\b(single agent|team|organizer|researcher|analyst|clean writer|وكيل واحد|فريق|منظم|باحث|محلل|كاتب)\b/gi, "[…]")
      .trim();
    fs.writeFileSync(path.join(blindDir, `${id}.md`), txt);
  });
  fs.writeFileSync(path.join(root, "blind_key.json"), JSON.stringify(key, null, 2));
}

console.log("wrote table.md + decisions.json" + (anonymize ? " + blind/" : ""));
console.log(JSON.stringify(decide(), null, 2));
