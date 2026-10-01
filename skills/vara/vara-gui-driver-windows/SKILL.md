---
name: vara-gui-driver-windows
description: >
  Vara's GUI-driving instincts for Windows: the see→act→confirm discipline,
  the commands→GUI fallback rule, transient-UI handling, and the speed and
  safety rules that make computer use reliable. Loaded whenever Vara plans
  or runs a [[sys]] computer_use sequence on a Windows desktop.
version: 1.0.0
author: Vara project
applies_to: [computer_use, windows]
---

# GUI Driver — Windows

## CAPABILITY MAP (consult before declaring anything impossible)

You can, on this machine, through computer-use sequences:

- **SEE**: `screenshot` (full or region, with `settle` for animations), `verify`
  (evidence capture), plus `get_screen_info`-style facts from the system layer.
- **READ**: OCR of screen regions through the vision layer (falls back to a
  region screenshot when OCR is unavailable — degrade, never crash).
- **FIND**: template matching against known widget crops.
- **ACT**: `focus`, `click`, `click_win` (window-relative), `type`, `hotkey`,
  `key`, `keys`, `scroll`, `move`.
- **BATCH**: any of the above as an atomic, schema-validated sequence.
- **LIFECYCLE** (destructive, gated): `close_window`, `close_app` — dry runs
  by default; only the owner's approval turns them into real closes.

## FALLBACK RULE (first-class, not last resort)

If a command-based attempt (shell, CLI flags, URL schemes) fails or shows no
result **twice, stop retrying commands and drive the GUI**: hotkeys + mouse +
screenshot. Do not ask permission to switch to GUI — it is within your normal
powers. Windows 10 UI map worth remembering: `win+a` Action Center, `win+i`
Settings, `win` Start, `win+s` Search.

## SEE → ACT → CONFIRM (the loop enforces it; you must plan for it)

1. **Never blind-click**: coordinates must come from a capture you actually
   observed in this flow. Start every flow with `{"op":"screenshot"}`.
2. **Focus-first**: any input op takes a `window`; focus the target before
   typing. If `active` in a result is not what you expected, stop and re-plan.
3. **Confirm**: after mutating ops, observe the result. End risky flows with
   `{"op":"verify"}` or a final screenshot. One corrective retry per failure,
   then STOP and report honestly.

## TRANSIENT UI

Menus and panels animate in (~0.3–0.6 s) and edge-anchored panels clip in
narrow capture regions. After opening one: `wait(0.6–1.0)` then a full-screen
`screenshot`. If the capture contradicts the screen, **re-shoot — never click
the opener again** (a second click toggles the panel closed).

## VERIFY STATE, DON'T ASSUME

Blue/filled toggle tile = ON, grey = OFF. Destructive ops default to dry runs
— the result says `dry_run: true` and shows the matched target; ask the owner,
then re-run with `confirm=true` only under an approved L2 unlock. Prefer
numeric state (`list_processes`, `get_active_window`) over visual guessing
when the system layer offers it.

## SPEED RULES

- Batch with one sequence instead of many single-op turns.
- Use `region` screenshots for small targets; `wait` only when UI animates.
- Long, multi-line, or non-ASCII text rides the clipboard path automatically.
- Don't re-shoot a static screen — the grounding set is already warm.

## SAFETY RULES

- Never click positions you haven't seen; never type into an unfocused window.
- Destructive combos (`alt+f4`, `ctrl+w`, `ctrl+q`, `ctrl+f4`,
  `ctrl+shift+w`) auto-escalate to L2 and force before/after evidence.
- Stop on the first failed verify, report, re-plan — never thrash the UI.
- Secrets are never typed from model context: ask the owner or use the
  vault-injection path when it exists. You never see the secret itself.

## HONEST LIMITS

- Focus on elevated (admin) or UWP windows can fail — report `focused: false`
  rather than pretending.
- You read what is rendered, not the widget tree (no UIA yet): low-DPI or
  anti-aliased text may need lower OCR confidence or a zoomed region capture.
- Shell surfaces (Action Center, tray menus) can still be caught
  mid-animation; the transient-UI rule above is the recovery path.
