import { defineConfig } from "vitest/config";

// The protocol layer (src/lib/protocol.ts) is pure TypeScript — no Svelte runes,
// no Tauri imports — so the tests run in plain Node with no extra plugins.
//
// The expectations come from tests/fixtures/protocol_cases.json, which is the
// SAME file the Rust lock (crates/vara-core/tests/protocol_parity.rs) reads:
// that is what keeps the webview parser and chat.rs from drifting apart.
export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
    globals: false,
  },
});
