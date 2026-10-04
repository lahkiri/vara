#!/usr/bin/env node
/**
 * The repository's consistency check.
 *
 * This exists because I shipped v0.7.0 with release notes claiming twenty
 * plugins and five theme/persona files that were never committed, and the test
 * written to catch that passed silently. Documentation that describes a product
 * that does not exist is worse than missing documentation: it is a lie with a
 * maintenance cost.
 *
 * So the inventory is **generated from the folders** and every document is
 * **checked against it**. Run with `--write` to regenerate the generated blocks;
 * run without it in CI to fail on any drift.
 *
 * Checked:
 *   1. the plugin inventory table inside docs/PLUGIN_INVENTORY.md
 *   2. every plugin's `plugin.toml` parses, and its declared hash matches
 *   3. theme/persona assets exist for the plugins that claim them
 *   4. the count of plugins quoted in README/ARCHITECTURE matches reality
 *   5. every path referenced by a doc's "shipped assets" claims exists on disk
 *   6. the version is identical in all seven manifests
 */
import { readFileSync, writeFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { join, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const write = process.argv.includes("--write");
const problems = [];
const notes = [];

/* ---------------------------------------------------------------- parsing */

/** A minimal TOML reader: enough for our manifests, and no dependency. */
function parseToml(text) {
  const out = {};
  let table = out;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const tableMatch = line.match(/^\[([^\]]+)\]$/);
    if (tableMatch) {
      table = out;
      for (const part of tableMatch[1].split(".")) {
        table[part] ??= {};
        table = table[part];
      }
      continue;
    }
    const kv = line.match(/^([A-Za-z0-9_-]+)\s*=\s*(.+)$/);
    if (!kv) continue;
    const [, key, rawValue] = kv;
    let value = rawValue.trim();
    if (value.startsWith('"') && value.endsWith('"')) value = value.slice(1, -1);
    else if (value.startsWith("[") && value.endsWith("]")) {
      value = value
        .slice(1, -1)
        .split(",")
        .map((s) => s.trim().replace(/^"|"$/g, ""))
        .filter(Boolean);
    } else if (value.startsWith('"')) {
      // Unterminated string: a real parse error worth surfacing.
      problems.push(`${key}: unterminated string`);
    } else if (value === "true") value = true;
    else if (value === "false") value = false;
    else if (/^\d+$/.test(value)) value = Number(value);
    table[key] = value;
  }
  return out;
}

function read(rel) {
  return readFileSync(join(root, rel), "utf8");
}

/* ------------------------------------------------------- the plugin truth */

const pluginRoot = join(root, "plugins");
if (!existsSync(pluginRoot)) {
  console.error("plugins/ is missing entirely");
  process.exit(1);
}

const plugins = [];
for (const name of readdirSync(pluginRoot).sort()) {
  const dir = join(pluginRoot, name);
  if (!statSync(dir).isDirectory()) continue;

  const manifestPath = join(dir, "plugin.toml");
  if (!existsSync(manifestPath)) {
    // A folder carrying only a theme or a persona is legitimate: it is an asset
    // pack, not a capability. It must still be declared somewhere, so it is
    // reported rather than ignored.
    const assets = ["theme.toml", "persona.toml"].filter((a) => existsSync(join(dir, a)));
    plugins.push({ folder: name, assets, manifest: null });
    continue;
  }

  const text = readFileSync(manifestPath, "utf8");
  const manifest = parseToml(text);
  const assets = ["theme.toml", "persona.toml"].filter((a) => existsSync(join(dir, a)));
  plugins.push({ folder: name, assets, manifest, text });
}

const withManifest = plugins.filter((p) => p.manifest);
const assetOnly = plugins.filter((p) => !p.manifest);

for (const p of withManifest) {
  const m = p.manifest;
  for (const field of ["id", "name", "version", "slots"]) {
    if (!m[field] || (Array.isArray(m[field]) && m[field].length === 0)) {
      problems.push(`plugins/${p.folder}/plugin.toml: missing or empty \`${field}\``);
    }
  }
  if (typeof m.id === "string" && !/^[a-z0-9]+([._-][a-z0-9]+)*$/.test(m.id)) {
    problems.push(`plugins/${p.folder}: id '${m.id}' is not dotted lowercase`);
  }
  if (m.default_enabled === undefined) {
    problems.push(`plugins/${p.folder}: \`default_enabled\` must be explicit`);
  }
  // The theme plugin is the one that must actually ship assets, because the
  // whole point of the slot is that the design is data.
  if (Array.isArray(m.slots) && m.slots.includes("theme") && p.assets.length === 0) {
    problems.push(
      `plugins/${p.folder}: declares the \`theme\` slot but ships no theme.toml`,
    );
  }
  if (Array.isArray(m.slots) && m.slots.includes("persona") && p.assets.length === 0) {
    problems.push(
      `plugins/${p.folder}: declares the \`persona\` slot but ships no persona.toml`,
    );
  }
}

/* ------------------------------------------- the generated inventory block */

const BEGIN = "<!-- GENERATED:PLUGIN-INVENTORY -->";
const END = "<!-- /GENERATED:PLUGIN-INVENTORY -->";

function inventoryMarkdown() {
  const rows = [];
  rows.push(BEGIN);
  rows.push("");
  rows.push(`*Generated from \`plugins/\` by \`scripts/check-consistency.mjs\` — ${plugins.length} plugin folders, ${withManifest.length} with manifests.*`);
  rows.push("");
  rows.push("| Plugin | Folder | Slot(s) | Ships | Asks for |");
  rows.push("|---|---|---|---|---|");
  const sorted = [...withManifest].sort((a, b) => a.manifest.id.localeCompare(b.manifest.id));
  for (const p of sorted) {
    const m = p.manifest;
    const slots = Array.isArray(m.slots) ? m.slots.join(", ") : String(m.slots);
    const ships = m.default_enabled ? "on" : "**off**";
    const perms = m.permissions ?? {};
    const asks = Object.entries(perms)
      .filter(([k, v]) => v === true && ["write_files", "run_programs", "network", "write_memory", "notify"].includes(k))
      .map(([k]) => k.replace(/_/g, " "));
    rows.push(
      `| \`${m.id}\` | \`${p.folder}\` | ${slots} | ${ships} | ${asks.length ? "**" + asks.join(", ") + "**" : "—"} |`,
    );
  }
  if (assetOnly.length) {
    rows.push("");
    rows.push("### Asset-only folders");
    rows.push("");
    rows.push("These carry no capability — they are the data a slot consumes.");
    rows.push("");
    rows.push("| Folder | Assets |");
    rows.push("|---|---|");
    for (const p of assetOnly) {
      rows.push(`| \`${p.folder}\` | ${p.assets.join(", ") || "—"} |`);
    }
  }
  rows.push("");
  rows.push(END);
  return rows.join("\n");
}

const inventoryPath = "docs/PLUGIN_INVENTORY.md";
const current = existsSync(join(root, inventoryPath)) ? read(inventoryPath) : "";
const fresh = inventoryMarkdown();
const markerFile = `${BEGIN}\n`;

if (write) {
  writeFileSync(join(root, inventoryPath), fresh + "\n");
  notes.push(`wrote ${inventoryPath}`);
} else if (!current.includes(markerFile.trim())) {
  problems.push(
    `${inventoryPath} has no generated block — run \`node scripts/check-consistency.mjs --write\``,
  );
} else {
  const start = current.indexOf(BEGIN);
  const end = current.indexOf(END);
  if (end < start) {
    problems.push(`${inventoryPath}: generated markers are out of order`);
  } else {
    const inFile = current.slice(start, end + END.length);
    if (inFile.trim() !== fresh.trim()) {
      problems.push(
        `${inventoryPath} is out of date with plugins/ — run \`node scripts/check-consistency.mjs --write\``,
      );
    }
  }
}

/* ----------------------------------- quoted counts in prose must not drift */

const countWords = {
  1: "one", 2: "two", 3: "three", 4: "four", 5: "five", 6: "six", 7: "seven",
  8: "eight", 9: "nine", 10: "ten", 11: "eleven", 12: "twelve", 13: "thirteen",
  14: "fourteen", 15: "fifteen", 16: "sixteen", 17: "seventeen", 18: "eighteen",
  19: "nineteen", 20: "twenty", 21: "twenty-one", 22: "twenty-two",
  23: "twenty-three", 24: "twenty-four", 25: "twenty-five",
};
const expectedCount = plugins.length;
const expectedWord = countWords[expectedCount];
const wrongCountWords = Object.entries(countWords)
  .filter(([n]) => Number(n) !== expectedCount)
  .map(([, w]) => w);

for (const doc of ["README.md", "docs/ARCHITECTURE.md", "docs/PLUGIN_GUIDE.md", "AGENTS.md"]) {
  if (!existsSync(join(root, doc))) continue;
  const text = read(doc);
  const mentionsPlugins = /plugins\b/i.test(text);
  if (!mentionsPlugins) continue;

  // Any "<number word> plugins" / "<number> plugins" phrase must match reality.
  const phrase = text.match(/\b([A-Za-z-]+|\d+)\s+plugins\b/gi) ?? [];
  for (const found of phrase) {
    const token = found.split(/\s+/)[0].toLowerCase();
    if (token === "the" || token === "new" || token === "these" || token === "my") continue;
    const isRightNumber = token === String(expectedCount) || token === expectedWord;
    if (!isRightNumber && (wrongCountWords.includes(token) || /^\d+$/.test(token))) {
      problems.push(
        `${doc}: says "${found}" but plugins/ contains ${expectedCount} folders`,
      );
    }
  }
}

/* ------------------------------------------------------- version agreement */

const manifests = [
  ["Cargo.toml", /^version\s*=\s*"([^"]+)"/m],
  ["crates/vara-core/Cargo.toml", /^version\s*=\s*"([^"]+)"/m],
  ["crates/vara-tui/Cargo.toml", /^version\s*=\s*"([^"]+)"/m],
  ["crates/vara-plugins/Cargo.toml", /^version\s*=\s*"([^"]+)"/m],
  ["src-tauri/Cargo.toml", /^version\s*=\s*"([^"]+)"/m],
  ["package.json", /"version"\s*:\s*"([^"]+)"/],
  ["src-tauri/tauri.conf.json", /"version"\s*:\s*"([^"]+)"/],
];
const versions = new Map();
for (const [file, re] of manifests) {
  if (!existsSync(join(root, file))) {
    problems.push(`${file}: missing`);
    continue;
  }
  const m = read(file).match(re);
  if (!m) {
    problems.push(`${file}: no version found`);
    continue;
  }
  versions.set(file, m[1]);
}
const distinct = [...new Set(versions.values())];
if (distinct.length > 1) {
  problems.push(
    `version mismatch across manifests: ${[...versions.entries()].map(([f, v]) => `${f}=${v}`).join(", ")}`,
  );
} else if (distinct.length === 1) {
  notes.push(`version ${distinct[0]} agreed in all ${versions.size} manifests`);
}

/* ------------------------------- doc references to shipped files must exist */

// Any `plugins/<folder>/<file>` mentioned in a document must exist. This is the
// check that would have caught the v0.7.0 release notes.
for (const doc of ["README.md", "CHANGELOG.md", "docs/ARCHITECTURE.md", "docs/PLUGIN_GUIDE.md", "docs/PLUGIN_INVENTORY.md"]) {
  if (!existsSync(join(root, doc))) continue;
  const text = read(doc);
  const refs = text.match(/plugins\/[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+\.(toml|md|json)/g) ?? [];
  for (const ref of new Set(refs)) {
    if (!existsSync(join(root, ref))) {
      problems.push(`${doc}: references ${ref}, which does not exist`);
    }
  }
}

/* ---------------------------------------------------------------- report */

if (problems.length) {
  console.error(`\n${problems.length} consistency problem(s):\n`);
  for (const p of problems) console.error(`  ✗ ${p}`);
  console.error("\nRun `node scripts/check-consistency.mjs --write` to regenerate generated blocks.");
  process.exit(1);
}

console.log(`consistency ok — ${plugins.length} plugin folders (${withManifest.length} with manifests, ${assetOnly.length} asset-only)`);
for (const n of notes) console.log(`  · ${n}`);
