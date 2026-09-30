// Shared frontend type shapes (mirror of vara-core::types).

export interface ProviderConfig {
  base_url: string;
  api_key: string;
  model: string;
  temperature: number;
}

export interface AutonomyConfig {
  heartbeat_enabled: boolean;
  heartbeat_minutes: number;
  close_to_tray: boolean;
  notifications_enabled: boolean;
  open_urls: boolean;
  open_paths: boolean;
  autostart: boolean;
}

export interface MissionDefaults {
  budget_tokens: number;
  max_steps: number;
}

export interface Settings {
  language: string;
  persona_style: string;
  provider: ProviderConfig;
  autonomy: AutonomyConfig;
  mission_defaults: MissionDefaults;
  watched_folder: string | null;
}

export interface Mission {
  id: number;
  created_at: string;
  goal: string;
  status: string;
  budget_tokens: number;
  spent_tokens: number;
  max_steps: number;
  steps_done: number;
  dimensions: { name: string; question: string }[] | null;
  error: string | null;
}

export interface Note {
  id: number;
  created_at: string;
  kind: string;
  title: string;
  body: string;
  mission_id: number | null;
  source_url: string | null;
  source_title: string | null;
}

export interface ReportRecord {
  id: number;
  mission_id: number;
  created_at: string;
  markdown: string;
  sources_json: { url: string; title: string; fetched: boolean }[] | null;
  check_json: import("./api").ProvenanceResult | null;
  backed_ratio: number | null;
  verdict: string | null;
  repaired: boolean;
}

export interface EventRecord {
  id: number;
  ts: string;
  level: string;
  kind: string;
  message: string;
}

// ---------- Conversations (chat with the entity) ----------

export interface Conversation {
  id: number;
  created_at: string;
  updated_at: string;
  title: string;
  mission_id: number | null;
}

export interface ChatMessageRecord {
  id: number;
  conversation_id: number;
  role: string; // "user" | "assistant"
  content: string;
  model: string | null;
  tokens: number;
  status: string; // "streaming" | "ok" | "stopped" | "error"
  created_at: string;
}

export interface SendChatStart {
  conversation_id: number;
  user_message_id: number;
  assistant_message_id: number;
}

export interface ChatDonePayload {
  conversation_id: number;
  message_id: number;
  content: string;
  tokens: number;
  model: string | null;
  status: string;
  mission_goal: string | null;
  error: string | null;
}

export interface UpdateInfo {
  version: string;
  current_version: string;
  notes: string;
}
