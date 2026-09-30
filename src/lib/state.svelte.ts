// Global app state with Svelte 5 runes + the live entity event stream.

import { listen } from "@tauri-apps/api/event";
import { api, type EntityEvent, type Bootstrap, type EntityStatus, type Stats } from "./api";
import type { Settings, Mission } from "./types";
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
  view: "dashboard",
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

export function pushFeed(ev: EntityEvent) {
  app.feed.unshift(ev);
  if (app.feed.length > 250) app.feed.pop();
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
}

let listening = false;

export async function initEvents(): Promise<void> {
  if (listening) return;
  listening = true;
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
}
