# Computer-Use Sidecar — Packaging Guide

Vara's computer-use capability runs the owner's Python computer-use MCP
server (`server.py`, 38 tools, see→act→confirm discipline) as a **Tauri
sidecar** on Windows. Vara's ActLoop (Rust, `vara-core::computer_use`)
drives it over MCP stdio and enforces the autonomy policy around it.

## Why a sidecar first

Phase 1 (this release) prioritizes a working Windows MVP over a full Rust
port. The sidecar keeps the proven Python implementation; the Rust side owns
validation, grounding, grants, and the Action Journal. Phase 2 replaces the
Python with native crates behind the same `ComputerUseAdapter` contract —
the shell and the loop do not change.

## Freeze the server into one exe (PyInstaller)

```powershell
# one-time
pip install pyinstaller

# from the folder containing server.py + requirements.txt deps installed
pyinstaller --onefile --name vara-cu --console ^
  --hidden-import mcp ^
  server.py

# result: dist\vara-cu.exe  (self-contained, no Python needed on user machines)
```

Copy `dist\vara-cu.exe` next to the Vara binary (or point
`VARA_CU_SIDECAR` at any path). The Tauri bundler can carry it as an
external binary:

```json
// src-tauri/tauri.conf.json → bundle
"externalBin": ["../sidecar/vara-cu"]
```

(Tauri appends the platform triple to the filename: `vara-cu-x86_64-pc-windows-msvc.exe`.)

## Runtime configuration

| Env | Meaning |
|---|---|
| `VARA_CU_SIDECAR` | Command to spawn, e.g. `vara-cu` (PATH) or an absolute path. Quotes allowed: `"C:\Program Files\Vara\vara-cu.exe"`. |

The shell (`src-tauri/commands.rs::run_computer_use`):

1. Parses the sequence JSON (`CuSequence::parse` — validate-then-execute).
2. Applies the policy gate: `L1` default; `L2` only with
   `computer_use_allow_close` enabled by the owner in Settings.
3. Spawns the sidecar, performs the MCP `initialize` handshake.
4. Maps each `CuOp` to the server's tools (`screenshot`, `click`,
   `click_window`, `type_text`, `hotkey`, `press_key`, `key_sequence`,
   `scroll`, `wait`, `focus_window`, `close_window`, `close_app`).
5. Journals every step into SQLite (`cu_journal`) with grant level,
   before/after refs, and the server's `check` guidance.
6. Returns a compact per-step receipt that lands in the chat as an action card.

## Developer mode (no Windows)

`VARA_CU_SIDECAR` is only read on execution; on Linux/macOS dev machines the
capability stays enabled-in-UI but fails honestly with "sidecar not
configured". All discipline logic is tested headlessly against
`MockComputerUse` (`crates/vara-core/tests/computer_use_harness.rs`) — the
same `ComputerUseAdapter` contract the sidecar implements.

## Phase 2 preview — the Rust port targets

| Python dep | Rust replacement |
|---|---|
| pyautogui | `enigo` + `windows-rs` SendInput |
| pygetwindow | `windows-rs` EnumWindows/SetForegroundWindow (+AttachThreadInput) |
| Pillow/capture | DXGI Desktop Duplication or PrintWindow+BitBlt |
| pytesseract | Windows.Media.Ocr (WinRT) — dependency-free |
| psutil | `sysinfo` |
| pyperclip | `arboard` |
