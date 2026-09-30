# Personas — 8 states, 5 styles

> "Different moments. Same mission." — "Different styles. Same Vara."

The character art ships with the app (`src/assets/characters/`) and is used
in the UI, the tray, and the installer icon.

## Entity states → character moments

| entity state | character | used when |
|---|---|---|
| `deliberating` | **Thinking** | planning a mission / building dimensions |
| `working` | **Working** | executing steps (search/fetch/notes) |
| `reporting` | **Planning** | the clean-context writer + checker pass |
| `paused` | **Serious** | owner paused the entity |
| `dormant` | **Focused** | before first mission |
| `sleeping` | **Dark Mode** | heartbeat asleep |
| success flash | **Excited** | report passed the gate |
| idle | **Happy** | everything done, memory healthy |

## Style personas (owner-selectable)

| style | accent | vibe |
|---|---|---|
| **Classic** | violet `#8b8cf0` | the original white-hooded Vara |
| **Dark Mode** | purple `#a855f7` | night-shift Vara |
| **Stealth** | slate `#94a3b8` | low-glow, quiet presence |
| **Tech** | cyan `#22d3ee` | maximum throughput energy |
| **Nature** | emerald `#34d399` | calm growth mode |

The style switches the UI accent (`--accent` / `--accent-soft` CSS custom
properties on `<html data-persona="…">`) and the idle avatar. All five appear
in Settings with thumbnails, so picking a persona is one click.

## The orb

Vara's little one-eyed companion appears in the splash screen ("Vara…") and
empty states — always watching, never in the way.
