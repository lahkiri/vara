// Global app state with Svelte 5 runes + the live entity event stream,
// the chat layer, and the over-the-air update state.

import { listen } from "@tauri-apps/api/event";
import { api, isTauri, type EntityEvent, type Bootstrap, type EntityStatus, type Stats, type ChatDeltaPayload } from "./api";
import type { Settings, Mission, Conversation, ChatMessageRecord, SysAction, ActionProposal, UpdateInfo, ChatDonePayload } from "./types";
import { loadLang, applyLangDom } from "./i18n.svelte";
// Protocol parsing lives in exactly one place: ./protocol.ts (locked to
// crates/vara-core/src/chat.rs by the shared fixture + Rust parity test).
import { extractMissionGoal, stripProtocolBlocks, stripMissionBlock } from "./protocol";

export { extractMissionGoal, stripProtocolBlocks, stripMissionBlock };

export const app = $state<{
  ready: boolean;
  view: string;
  busy: boolean;
  paused: boolean;
  stats: Stats;
  activeMission: Mission | null;
  settings: Settings | null;
  dataDir: string;
  fts: boolean;
  feed: EntityEvent[];
  lastReportId: number | null;
}>({
  ready: false,
  view: "chat",
  busy: false,
  paused: false,
  stats: { missions_total: 0, missions_completed: 0, notes_total: 0, reports_total: 0, avg_backed_ratio: null },
  activeMission: null,
  settings: null,
  dataDir: "",
  fts: false,
  feed: [],
  lastReportId: null,
});

// ---------- chat state ----------

export const chat = $state<{
  conversations: Conversation[];
  activeId: number | null;
  messages: ChatMessageRecord[];
  loadingThread: boolean;
  error: string;
  pendingQuestion: string;
  /// Live mission progress keyed by mission id — feeds the in-thread cards.
  liveMissions: Record<number, { status: string; steps_done: number; max_steps: number; spent_tokens: number }>;
  /// Proposals waiting for the owner's decision, keyed by message id. Refused
  /// entries (policy rejected them before a card could be shown) are kept here
  /// too, so the thread explains the refusal instead of hiding it.
  pendingSys: Record<number, ActionProposal[]>;
  /// The goal the backend extracted for a message, keyed by message id. The
  /// displayed text has the protocol markers stripped, so the proposal has to
  /// be remembered rather than re-parsed from what the user sees.
  missionGoals: Record<number, string>;
}>({
  conversations: [],
  activeId: null,
  messages: [],
  loadingThread: false,
  error: "",
  pendingQuestion: "",
  liveMissions: {},
  pendingSys: {},
  missionGoals: {},
});

export async function loadConversations(): Promise<void> {
  chat.conversations = await api.conversations().catch(() => []);
}

export async function openConversation(id: number): Promise<void> {
  chat.activeId = id;
  chat.loadingThread = true;
  chat.error = "";
  chat.messages = await api.messages(id).catch(() => []);
  chat.loadingThread = false;
  // Re-render proposals that are still awaiting a decision. They live in the
  // database, not in this object, so a reload (or a different window) shows the
  // same pending card instead of losing an open question.
  chat.pendingSys = {};
  const open = await api.proposals(id).catch(() => [] as ActionProposal[]);
  for (const p of open) {
    if (p.state !== "pending" || !p.message_id) continue;
    (chat.pendingSys[p.message_id] ??= []).push(p);
  }
  // seed live mission cards with the persisted status (historical threads)
  const cardIds = new Set(
    chat.messages.filter((m) => m.kind === "mission" && m.mission_id).map((m) => m.mission_id as number),
  );
  if (cardIds.size > 0) {
    const all: Mission[] = await api.missions(100).catch(() => []);
    for (const m of all) {
      if (cardIds.has(m.id) && !chat.liveMissions[m.id]) {
        chat.liveMissions[m.id] = { status: m.status, steps_done: m.steps_done, max_steps: m.max_steps, spent_tokens: m.spent_tokens };
      }
    }
  }
  if (!isTauri()) scrollChatToBottom();
}

export async function newConversation(title: string | null = null, missionId: number | null = null): Promise<void> {
  const conv = await api.createConversation(title, missionId).catch(() => null);
  if (!conv) return;
  chat.conversations.unshift(conv);
  chat.activeId = conv.id;
  chat.messages = [];
}

export async function deleteConversation(id: number): Promise<void> {
  await api.deleteConversation(id).catch(() => {});
  chat.conversations = chat.conversations.filter((c) => c.id !== id);
  if (chat.activeId === id) {
    chat.activeId = null;
    chat.messages = [];
    if (chat.conversations.length > 0) await openConversation(chat.conversations[0].id);
  }
}

/// Starts a mission that LIVES in the open thread (manual fallback when the
/// auto-start was disabled or Vara was busy at proposal time).
export async function startMissionFromChat(goal: string): Promise<void> {
  if (!goal.trim()) return;
  if (chat.activeId === null) await newConversation();
  if (chat.activeId === null) return;
  try {
    await api.startMissionInConversation(chat.activeId, goal.trim());
    chat.error = "";
  } catch (e) {
    chat.error = String(e);
  }
}

/// Approves and executes one proposal. Two backend calls on purpose: the
/// decision and the execution are separate rows in the state machine, so an
/// approval cannot be replayed and an expired card cannot run anything.
/// The target is never sent from here — the shell executes what it stored.
export async function executeSysAction(messageId: number, proposal: ActionProposal): Promise<void> {
  if (proposal.id === null) return;
  chat.error = "";
  try {
    await api.sysApprove(proposal.id, true);
    await api.sysExecute(proposal.id);
  } catch (e) {
    chat.error = String(e);
  } finally {
    removePending(messageId, proposal.id);
  }
}

/// Declines a proposal (or clears the whole card): a denial is recorded, not
/// just forgotten, so the entity learns the owner said no.
export async function dismissSysActions(messageId: number): Promise<void> {
  const list = chat.pendingSys[messageId] ?? [];
  await Promise.all(
    list
      .filter((p) => p.id !== null)
      .map((p) => api.sysApprove(p.id as number, false).catch(() => undefined)),
  );
  delete chat.pendingSys[messageId];
}

function removePending(messageId: number, proposalId: number | null): void {
  const list = chat.pendingSys[messageId];
  if (!list) return;
  chat.pendingSys[messageId] = list.filter((p) => p.id !== proposalId);
  if (chat.pendingSys[messageId].length === 0) delete chat.pendingSys[messageId];
}

export async function sendMessage(text: string): Promise<void> {
  const content = text.trim();
  if (!content || chat.activeId === null) return;
  chat.error = "";
  try {
    const start = await api.sendChat(chat.activeId, content);
    // the backend already persisted both rows — append optimistic copies
    chat.messages.push({
      id: start.user_message_id,
      conversation_id: start.conversation_id,
      role: "user",
      content,
      model: null,
      tokens: 0,
      status: "ok",
      created_at: "",
      kind: "text",
      mission_id: null,
    });
    chat.messages.push({
      id: start.assistant_message_id,
      conversation_id: start.conversation_id,
      role: "assistant",
      content: "",
      model: null,
      tokens: 0,
      status: "streaming",
      created_at: "",
      kind: "text",
      mission_id: null,
    });
    if (!isTauri()) scrollChatToBottom();
  } catch (e) {
    chat.error = String(e);
  }
}

export async function stopStreaming(): Promise<void> {
  if (chat.activeId === null) return;
  await api.stopChat(chat.activeId).catch(() => {});
}

function applyDelta(p: ChatDeltaPayload): void {
  if (p.conversation_id !== chat.activeId) return;
  const m = chat.messages.find((x) => x.id === p.message_id);
  if (m) {
    m.content += p.delta;
    m.status = "streaming";
  }
  if (!isTauri()) scrollChatToBottom(true);
}

function applyDone(p: ChatDonePayload): void {
  // refresh the conversation list ordering/title
  loadConversations().catch(() => {});
  if (p.conversation_id !== chat.activeId) return;
  const m = chat.messages.find((x) => x.id === p.message_id);
  if (m) {
    m.content = p.content;
    m.status = p.status;
    m.model = p.model;
    m.tokens = p.tokens;
  }
  // OS actions awaiting approval attach to the assistant message. The backend
  // minted them; the UI only ever holds ids + the text to display. Older
  // payloads (and the browser mock) carry `sys_actions` only: those become
  // display-only cards rather than executable ones, because nothing was
  // approved in the backend for them.
  const proposals: ActionProposal[] =
    p.proposals ??
    (p.sys_actions ?? []).map((a) => ({
      id: null,
      action: a.action,
      target: a.target,
      risk: "high" as const,
      expires_at: 0,
      refused: null,
    }));
  if (proposals.length > 0) chat.pendingSys[p.message_id] = proposals;
  // remember the proposal goal: `p.content` is the stripped text, so re-parsing
  // it later finds nothing and the "turn into a mission" button would vanish.
  if (p.mission_goal) chat.missionGoals[p.message_id] = p.mission_goal;
  if (p.error) chat.error = p.error;
  if (!isTauri()) scrollChatToBottom(true);
}

/// Background rows (mission cards, OS receipts, mission closings) arrive as
/// full records — append them to the open thread without a reload.
function applyAppended(rec: ChatMessageRecord): void {
  loadConversations().catch(() => {});
  if (rec.conversation_id !== chat.activeId) return;
  if (chat.messages.some((x) => x.id === rec.id)) return;
  chat.messages.push(rec);
  if (!isTauri()) scrollChatToBottom(true);
}

function seedLiveMission(id: number, patch: Partial<{ status: string; steps_done: number; max_steps: number; spent_tokens: number }>): void {
  const cur = chat.liveMissions[id] ?? { status: "running", steps_done: 0, max_steps: 14, spent_tokens: 0 };
  chat.liveMissions[id] = { ...cur, ...patch };
}

/// True while any assistant reply is streaming in the open thread.
export function streamingNow(): boolean {
  return chat.messages.some((m) => m.role === "assistant" && m.status === "streaming");
}

/// When the latest assistant turn proposes a mission, the UI offers to run it.
export function lastMissionProposal(): { messageId: number; goal: string } | null {
  for (let i = chat.messages.length - 1; i >= 0; i--) {
    const m = chat.messages[i];
    if (m.role !== "assistant") continue;
    // the backend's extraction first (the display text has markers stripped),
    // then the raw text for threads loaded from an older build
    const goal = chat.missionGoals[m.id] ?? extractMissionGoal(m.content);
    if (goal) return { messageId: m.id, goal };
    if (m.status !== "streaming") break;
  }
  return null;
}

export function scrollChatToBottom(smooth = false): void {
  requestAnimationFrame(() => {
    const el = document.getElementById("chat-scroll");
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: smooth ? "smooth" : "auto" });
  });
}

// ---------- updates state ----------

export const updater = $state<{
  available: UpdateInfo | null;
  checking: boolean;
  installing: boolean;
  progress: { downloaded: number; total: number | null } | null;
}>({
  available: null,
  checking: false,
  installing: false,
  progress: null,
});

export async function checkForUpdate(silent = false): Promise<void> {
  if (!isTauri()) return;
  updater.checking = true;
  try {
    updater.available = await api.checkForUpdate();
  } catch {
    if (!silent) updater.available = null;
  } finally {
    updater.checking = false;
  }
}

export async function installUpdate(): Promise<void> {
  if (!isTauri()) return;
  updater.installing = true;
  await api.installUpdate().catch(() => {});
  updater.installing = false;
}

export function applyPersona(style: string | undefined) {
  const s = style && ["classic", "dark", "stealth", "tech", "nature"].includes(style) ? style : "classic";
  document.documentElement.dataset.persona = s;
}

export async function refreshStatus(): Promise<void> {
  const st: EntityStatus = await api.status();
  app.busy = st.busy;
  app.paused = st.paused;
  app.activeMission = st.active_mission;
  app.stats = st.stats;
}

async function applyBootstrap(b: Bootstrap): Promise<void> {
  app.settings = b.settings;
  app.dataDir = b.data_dir;
  app.fts = b.fts_enabled;
  app.busy = b.status.busy;
  app.paused = b.status.paused;
  app.activeMission = b.status.active_mission;
  app.stats = b.status.stats;
  applyPersona(b.settings?.persona_style);
  applyLangDom((loadLang() as "ar" | "en") ?? "ar");
}

export async function boot(): Promise<void> {
  try {
    const b = await api.bootstrap();
    await applyBootstrap(b);
  } catch (e) {
    console.error("bootstrap failed", e);
  } finally {
    app.ready = true;
  }
  // land in the conversation that matters most (companion-first, like Dot)
  await loadConversations().catch(() => {});
  if (chat.conversations.length > 0) await openConversation(chat.conversations[0].id);
  // with no history yet, ChatView shows the first-hello state and creates the
  // thread on demand — never junk conversations on every launch
  // silent update probe on every launch — the OTA promise
  await checkForUpdate(true).catch(() => {});
}

let listening = false;

function onEntityEvent(ev: EntityEvent): void {
  pushFeed(ev);
  if (ev.type === "state") {
    if (ev.state === "attentive") {
      refreshStatus().catch(() => {});
    }
  } else if (ev.type === "mission_update") {
    if (app.activeMission && app.activeMission.id === ev.id) {
      app.activeMission.steps_done = ev.steps_done;
      app.activeMission.spent_tokens = ev.spent_tokens;
      app.activeMission.status = ev.status;
    }
    seedLiveMission(ev.id, { status: ev.status, steps_done: ev.steps_done, max_steps: ev.max_steps, spent_tokens: ev.spent_tokens });
  } else if (ev.type === "report_ready") {
    seedLiveMission(ev.mission_id, { status: "completed" });
  }
}

export async function initEvents(): Promise<void> {
  if (listening) return;
  listening = true;

  if (!isTauri()) {
    // browser mock: the mock backend speaks window CustomEvents
    window.addEventListener("mock:chat/delta", (e) => applyDelta((e as CustomEvent).detail as ChatDeltaPayload));
    window.addEventListener("mock:chat/done", (e) => applyDone((e as CustomEvent).detail as ChatDonePayload));
    window.addEventListener("mock:chat/message", (e) => applyAppended((e as CustomEvent).detail as ChatMessageRecord));
    window.addEventListener("mock:entity/event", (e) => onEntityEvent((e as CustomEvent).detail as EntityEvent));
    return;
  }

  await listen<EntityEvent>("entity://event", (e) => onEntityEvent(e.payload));
  await listen<{ report_id: number | null; status: string; mission_id: number }>("entity://done", (e) => {
    if (e.payload.report_id) app.lastReportId = e.payload.report_id;
    refreshStatus().catch(() => {});
  });
  await listen<ChatDeltaPayload>("entity://chat/delta", (e) => applyDelta(e.payload));
  await listen<ChatDonePayload>("entity://chat/done", (e) => applyDone(e.payload));
  await listen<ChatMessageRecord>("entity://chat/message", (e) => applyAppended(e.payload));
  await listen<{ downloaded: number; total: number | null }>("entity://update/progress", (e) => {
    updater.progress = e.payload;
  });
}

export function pushFeed(ev: EntityEvent) {
  app.feed.unshift(ev);
  if (app.feed.length > 250) app.feed.pop();
}
