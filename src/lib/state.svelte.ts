// Global app state with Svelte 5 runes + the live entity event stream,
// the chat layer, and the over-the-air update state.

import { listen } from "@tauri-apps/api/event";
import { api, isTauri, type EntityEvent, type Bootstrap, type EntityStatus, type Stats, type ChatDeltaPayload } from "./api";
import type { Settings, Mission, Conversation, ChatMessageRecord, UpdateInfo, ChatDonePayload } from "./types";
import { loadLang, applyLangDom } from "./i18n.svelte";

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
}>({
  conversations: [],
  activeId: null,
  messages: [],
  loadingThread: false,
  error: "",
  pendingQuestion: "",
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
  if (p.error) chat.error = p.error;
  if (!isTauri()) scrollChatToBottom(true);
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
    const goal = extractMissionGoal(m.content);
    if (goal) return { messageId: m.id, goal };
    if (m.status !== "streaming") break;
  }
  return null;
}

export function extractMissionGoal(content: string): string | null {
  const open = "[[mission]]";
  const close = "[[/mission]]";
  const s = content.indexOf(open);
  if (s === -1) return null;
  const e = content.indexOf(close, s);
  if (e === -1) return null;
  return content.slice(s + open.length, e).trim() || null;
}

export function stripMissionBlock(content: string): string {
  const open = "[[mission]]";
  const close = "[[/mission]]";
  const s = content.indexOf(open);
  if (s === -1) return content.trim();
  const e = content.indexOf(close, s);
  if (e === -1) return content.trim();
  return (content.slice(0, s) + content.slice(e + close.length)).trim();
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

export async function initEvents(): Promise<void> {
  if (listening) return;
  listening = true;

  if (!isTauri()) {
    // browser mock: the mock backend speaks window CustomEvents
    window.addEventListener("mock:chat/delta", (e) => applyDelta((e as CustomEvent).detail as ChatDeltaPayload));
    window.addEventListener("mock:chat/done", (e) => applyDone((e as CustomEvent).detail as ChatDonePayload));
    return;
  }

  await listen<EntityEvent>("entity://event", (e) => {
    const ev = e.payload;
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
    }
  });
  await listen<{ report_id: number | null; status: string; mission_id: number }>("entity://done", (e) => {
    if (e.payload.report_id) app.lastReportId = e.payload.report_id;
    refreshStatus().catch(() => {});
  });
  await listen<ChatDeltaPayload>("entity://chat/delta", (e) => applyDelta(e.payload));
  await listen<ChatDonePayload>("entity://chat/done", (e) => applyDone(e.payload));
  await listen<{ downloaded: number; total: number | null }>("entity://update/progress", (e) => {
    updater.progress = e.payload;
  });
}

export function pushFeed(ev: EntityEvent) {
  app.feed.unshift(ev);
  if (app.feed.length > 250) app.feed.pop();
}
