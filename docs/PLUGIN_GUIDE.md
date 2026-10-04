# Writing a plugin for فارا (Vara)

This is the guide for **both** audiences, and the split is deliberate: most
people who extend Vara will never open this file, and the ones who do should
never have to read Rust.

---

## 1. What a plugin is

A plugin is **a folder with a `plugin.toml` in it**. That is the whole format.
Drop the folder in `plugins/`, and Vara sees it; delete the folder, and Vara
forgets it. Nothing is compiled, nothing is registered in a global list, no code
in the product has to change.

```
plugins/
  my-plugin/
    plugin.toml        ← required
    README.md          ← recommended: what it does, in the owner's words
    anything-else/     ← your assets, themes, prompts, data
```

## 2. The smallest useful plugin

```toml
id = "yourname.hello"
name = "Hello"
version = "1.0.0"
summary = "A theme that makes everything readable in sunlight"
slots = ["theme"]
```

Save it, and the plugin appears in the list immediately. It is **enabled by
default** because it asks for nothing.

## 3. The twelve slots

A plugin declares what it contributes. A plugin with no slots is refused,
because a plugin that contributes nothing is a mistake, not a plugin.

| Slot | What you are adding | Example |
|---|---|---|
| `tool` | Something the entity can call | `read_file`, `send_email` |
| `toolset` | A bundle of tools for one job | browser use, computer use |
| `brain` | A model provider | a local llama.cpp server |
| `memory` | Where notes live | a vector store instead of SQLite |
| `interface` | A way to talk to the entity | desktop, terminal, headless |
| `theme` | Visual tokens | dark, high-contrast, your own colours |
| `persona` | How it speaks and what it refuses | "engineer", "warm", "direct" |
| `channel` | A place it can reach you | Discord, Slack, mail |
| `goal_engine` | How it decides what to do next | marketing, research, monitoring |
| `subagent` | A scoped worker it can spawn | researcher, builder, critic |
| `mcp` | A Model Context Protocol server | someone else's tool server |
| `skill` | A document the model reads first | "read this before touching the DB" |

## 4. Permissions: nothing is granted by silence

**A permission you do not declare is a permission you do not have.** This is the
single most important rule in the format, because the alternative — a plugin
quietly acquiring a capability nobody reviewed — is how plugin ecosystems go
wrong.

```toml
[permissions]
read_files   = true                       # read under the allowed roots
write_files  = false                      # create, move, trash (needs the undo journal)
run_programs = false                      # argv-only, gated, never a shell
network      = false                      # reach the internet
read_memory  = true                       # read the entity's notes
write_memory = false                      # change the entity's notes
notify       = false                      # send you a notification
file_extensions = ["txt", "md", "json"]   # which files it cares about
```

`read_memory` and `write_memory` are separate on purpose. A plugin that can
**write** memory changes every answer the entity gives afterwards, which is a
different decision from one that only reads.

### What happens when you ask for something dangerous

1. The plugin still **appears** in the list, with the exact phrase it is asking
   for: `asks to: write files, use the network`.
2. It **cannot be enabled** until the owner approves it. Vara refuses with the
   reason, it does not fail silently.
3. Once approved, the approval is **bound to what was shown**. If you later add
   `run_programs`, the plugin is *unapproved again* and must be re-approved. An
   approval is for the version the owner saw.

Dangerous permissions are `write_files`, `run_programs`, `network`,
`write_memory` and `notify`. Everything else is quiet.

## 5. Integrity: the hash is over your claims

```toml
sha256 = "…"
```

Compute it with:

```sh
vara-plugins hash plugins/my-plugin      # prints the value to paste in
```

The hash covers **id, version, slots, requires and permissions** — the things
you are claiming. It does **not** cover comments, key order or whitespace, so
reformatting your manifest does not break it, while changing what it claims
always does.

- **No `sha256`** → the plugin works and is labelled **unverified**. Right for
  something you wrote yourself on your own machine.
- **`sha256` present and correct** → **verified**.
- **`sha256` present and wrong** → **BROKEN**, and it will not load. This is the
  case that matters when a plugin arrived from somewhere else.

Vara also computes a **content hash** over every file in the folder (not just
the ones you listed), which is the number to publish and compare. Listing fewer
files cannot hide anything, because the folder is what gets hashed.

## 6. Dependencies

```toml
requires = ["vara.tools.read"]
```

Dependencies load before you. Two rules keep this debuggable:

- If a dependency is **disabled**, Vara tells the owner *which* plugin needs it
  and that it is disabled — it does not load you and let you fail later.
- If a dependency is **missing**, Vara says *not installed*, which is a different
  problem and a different fix.

Cycles are refused with the loop named (`a → b → a`) rather than resolved by an
arbitrary order, because a nondeterministic load order makes a bug impossible to
reproduce.

## 7. Shipping your own settings

```toml
[config]
engine = "webview2"
headless = "false"
```

The host **never parses these**. Your plugin owns its own settings, which is why
adding one never requires a change to Vara itself.

## 8. Checking your work

```sh
vara-plugins list                 # everything installed, what it asks for, enabled or not
vara-plugins plan                 # the exact load order, dependency-checked
vara-plugins check                # exits non-zero if the composition is broken (use in CI)
vara-plugins enable  yourname.hello
vara-plugins disable yourname.hello
vara-plugins approve yourname.hello   # accept what it asks for
```

For a plugin no one else depends on, `check` passing and `list` showing the
right permissions is the whole test.

## 9. The rules Vara will not break for you

These hold for every plugin, including the ones Vara ships:

1. **A plugin never executes anything by registering.** Registering a tool is not
   running a tool. Execution always goes through the gate, with the owner's
   policy and, for anything consequential, an approval.
2. **The hard-deny floor is outside your control.** Certain paths and actions are
   refused before your code sees them, and no manifest setting can open them.
3. **Secrets never reach you.** Spawned programs get a scrubbed environment, so a
   command a plugin runs cannot read the owner's API key.
4. **Commands are argv-only.** There is no shell, no pipes, no composition —
   `["git", "status"]` runs; `"git status | mail"` does not.

If your plugin needs one of these relaxed, the answer is no, and the reason is
in [`AGENTS.md`](../../AGENTS.md).

## 10. For the person who does not write TOML

You do not have to. Ask the entity:

> "اصنع لي إضافة تقرأ ملفات `.log` في مجلد التنزيلات وتلخّصها"

Vara will draft the manifest, show you **what it does and what it asks for in
plain words**, and wait. Nothing runs, and nothing is enabled, until you say yes.
The four authoring modes exist so you decide how much it may draft on its own:

| Mode | What it may do |
|---|---|
| `off` (default) | nothing — it will not offer to create plugins at all |
| `ask` | drafts one when it notices it is missing a capability, and waits |
| `delegated` | drafts and tests, but cannot enable anything |
| `auto` | drafts, tests and enables **within a scope you set** |

Even in `auto`, a plugin that asks for a dangerous permission still stops and
asks. That is not configurable, by design.
