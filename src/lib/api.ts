// Typed invoke wrappers over the Tauri commands.

import { invoke } from "@tauri-apps/api/core";
import type { Settings, Mission, Note, ReportRecord, EventRecord, ProviderConfig } from "./types";

export type { Settings, Mission, Note, ReportRecord, EventRecord, ProviderConfig };

export interface Stats {
  missions_total: number;
  missions_completed: number;
  notes_total: number;
  reports_total: number;
  avg_backed_ratio: number | null;
}

export interface EntityStatus {
  busy: boolean;
  paused: boolean;
  active_mission: Mission | null;
  stats: Stats;
}

export interface Bootstrap {
  settings: Settings;
  status: EntityStatus;
  fts_enabled: boolean;
  data_dir: string;
}

export interface MissionDetail {
  mission: Mission;
  actions: { id: number; mission_id: number | null; ts: string; kind: string; ok: boolean; summary: string }[];
  sources: { id: number; mission_id: number; url: string; title: string; fetched: boolean }[];
}

export interface TestProviderResult {
  ok: boolean;
  latency_ms: number;
  reply: string;
  error: string;
}

export interface ProvenanceResult {
  verdict: string;
  rule: string;
  metrics: { cited_total: number; retrieved_total: number; backed_ratio: number };
  cited_not_retrieved: string[];
  retrieved_not_cited: string[];
  unresolved_refs: number[];
  sections: {
    title: string;
    citations: number;
    backed_citations: number;
    effectively_uncovered: boolean;
    no_citations: boolean;
  }[];
  checks: { c1_all_cited_in_retrieved: boolean; c2_refs_resolve_to_source_list: boolean };
}

export const api = {
  bootstrap: () => invoke<Bootstrap>("get_bootstrap"),
  status: () => invoke<EntityStatus>("get_entity_status"),
  saveSettings: (s: Settings) => invoke<void>("save_settings", { newSettings: s }),
  testProvider: (p: ProviderConfig) => invoke<TestProviderResult>("test_provider", { provider: p }),
  startMission: (goal: string, budgetTokens: number, maxSteps: number) =>
    invoke<number>("create_and_start_mission", { goal, budgetTokens, maxSteps }),
  pause: (paused: boolean) => invoke<void>("pause_entity", { paused }),
  cancelMission: () => invoke<void>("cancel_mission"),
  missions: (limit = 50) => invoke<Mission[]>("list_missions", { limit }),
  missionDetail: (id: number) => invoke<MissionDetail>("get_mission_detail", { id }),
  reports: (limit = 50) => invoke<ReportRecord[]>("list_reports", { limit }),
  report: (id: number) => invoke<ReportRecord>("get_report", { id }),
  notes: (query: string | null, limit = 100) => invoke<Note[]>("list_notes", { query, limit }),
  addNote: (title: string, body: string) => invoke<number>("add_manual_note", { title, body }),
  deleteNote: (id: number) => invoke<void>("delete_note", { id }),
  events: (limit = 150) => invoke<EventRecord[]>("list_events", { limit }),
  sysOpen: (target: string) => invoke<void>("sys_open", { target }),
  exportReport: (id: number) => invoke<string>("export_report", { id }),
  showWindow: () => invoke<void>("show_window"),
};

// Entity event stream payloads (tagged union via `type` field).
export type EntityEvent =
  | { type: "state"; state: string; mission_id: number | null }
  | { type: "activity"; kind: string; message: string; mission_id: number | null }
  | { type: "mission_update"; id: number; status: string; steps_done: number; max_steps: number; spent_tokens: number }
  | { type: "report_ready"; id: number; mission_id: number; verdict: string; backed_ratio: number };
