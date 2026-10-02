# Vara design research

_Reviewed: 2026-10-02. This is a decision record, not a feature catalogue._

## Product question

Vara should feel like a continuous, accountable presence that works with the
owner over time. The conversation is the primary workspace. Missions, evidence,
approvals, and outcomes appear in context rather than becoming disconnected
admin surfaces.

## Reference matrix

| Reference | What was studied | Principle worth transferring | Deliberate non-transfer |
| --- | --- | --- | --- |
| [Letta](https://docs.letta.com/platform/desktop-app) | Stateful agents; one place for chat, memory, schedules, channels, and skills | Make the entity and its durable state inspectable without making them the default task | Do not expose every runtime primitive as a top-level navigation item |
| [OpenHands](https://docs.all-hands.dev/) | Work-oriented agent execution and human intervention | Treat activity as work with an owner-visible state, not as decorative streaming text | Do not turn Vara into an IDE or require a workspace before a conversation can start |
| [Open WebUI Workspace](https://docs.openwebui.com/features/workspace/) | Separation of models, knowledge, prompts, skills, and tools | Keep configuration and reusable capability concepts distinct internally | Do not copy its broad administration-first information architecture into the everyday surface |
| [Raycast](https://www.raycast.com/) | Keyboard-first navigation and low-friction commands | Use concise labels, predictable navigation, and short paths to action | Do not force command-palette interaction for ordinary conversation |
| [Linear](https://linear.app/) | Dense but calm work states and restrained visual hierarchy | Let state, recency, and focus do the visual work; reserve color for meaning | Do not present missions as a metric dashboard |
| [Browser Use](https://browser-use.com/web-agent-api) | Streamed progress and the ability for a person to take over at sensitive moments | Represent actions as a legible sequence with an explicit owner decision point | Do not treat computer control as an opaque autonomous black box |

## Findings applied to Vara

1. **One visible center.** A conversation is the work surface. It carries the
   context, live mission, approvals, receipts, and report follow-up.
2. **Presence is quiet state.** Vara's state is visible through a small avatar,
   a semantic status label, and purposeful motion while working. It is not a
   dashboard animation.
3. **Progressive disclosure is a trust mechanism.** The default view answers
   “what is happening?”; details remain available in activity, memory, and
   reports when the owner needs to inspect them.
4. **Approval must be legible.** The user sees the proposed action and can
   approve or reject it in the thread. A missing reason or risk classification
   is a backend-contract gap, not a UI excuse to invent one.
5. **Evidence stays close to the claim.** Vara's provenance result is a product
   advantage and should be presented as an inspectable outcome, not a hidden
   implementation detail.

## Current audit snapshot

- The Rust core already has durable conversations, mission state, SQLite WAL,
  provenance checks, policy-gated actions, and a live event stream. These are
  product strengths to preserve.
- The frontend already keeps missions and action receipts in their originating
  conversation. The redesign therefore changes hierarchy and visual language,
  not this contract.
- The pre-redesign surface relied on aurora gradients, glass cards, and several
  competing rails. This made the product feel decorative rather than focused.
- Version and security documentation are currently inconsistent with v0.5.0;
  those release-hygiene issues are tracked separately from the UI work.
