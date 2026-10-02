# Vara product decisions

_Reviewed: 2026-10-02_

## Product model

| Concept | User-facing meaning | Where it lives |
| --- | --- | --- |
| Vara | The persistent entity the owner works with | Present in every conversation through state and identity |
| Conversation | The shared work context | Primary workspace |
| Mission | A bounded piece of work Vara is carrying out | A live card inside its conversation; history in Reports |
| Activity | A concise explanation of current or past work | In-context summary first, full chronological log on demand |
| Memory | Durable notes and retrieved knowledge | Inspectable in Memory when relevant, not continuously dumped into chat |
| Report | The durable result of research | Linked from its mission, with provenance status |
| Approval | The owner's explicit decision over an external action | Inline at the moment the action is proposed |

Tools, database migrations, provider transport, prompts, and runtime event
plumbing remain implementation details unless an owner needs to configure or
inspect them.

## Interaction decisions

1. Keep the global navigation short: conversation, reports, memory, activity,
   settings. Avoid a separate mission launcher because missions originate in
   conversation.
2. Treat the header as a context and presence strip, not a second dashboard.
   It identifies the active conversation and the entity's current state.
3. Put thread history in a compact, calm rail. The active thread should have
   clear focus without turning every conversation into a card.
4. Use a restrained dark palette: near-black surfaces, one persona-derived
   accent, and semantic success/warning/error colors. Color never carries the
   whole meaning alone.
5. Remove ambient decorative effects. Motion is limited to state transition,
   streaming, and active-work presence, and respects reduced-motion settings.

## Implementation boundary for this iteration

This iteration reworks the application shell, sidebar, conversation workspace,
and design tokens while preserving the existing Svelte state/API contracts. It
does not add a new external dependency or weaken provenance, action-policy, or
budget invariants.

## Next capability decisions

1. Extend the action proposal contract with a human-readable reason and risk
   level so approval cards can explain impact without guessing.
2. Add a keyboard command surface only after its commands map to real,
   discoverable workflows and have focus/accessibility tests.
3. Repair version and security-document consistency in the release pass.
