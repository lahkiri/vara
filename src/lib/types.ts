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
  auto_start_missions: boolean;
  run_commands: boolean;
  allow_screenshots: boolean;
  allow_computer_use: boolean;
  computer_use_allow_close: boolean;
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
  kind: string; // "text" | "mission" | "action"
  mission_id: number | null;
}

/// An OS action Vara proposes from inside the chat ([[sys]] protocol).
export interface SysAction {
  action: string; // "open_url" | "open_path" | "run" | "screenshot" | "computer_use"
  target: string;
}

/// A backend-owned proposal: the only thing the UI may approve or execute.
///
/// The webview never receives an executable payload it can forge — the shell
/// mints these rows from model output, hands out ids, and `sys_execute` reads
/// the target from the database row. `refused` is set when the policy rejected
/// the proposal before the owner ever saw a card (an invalid URL, an empty
/// target): those are shown as a refusal, never as an approval button.
export interface ActionProposal {
  id: number | null;
  action: string;
  target: string;
  risk: "low" | "medium" | "high";
  expires_at: number;
  refused: string | null;
  /// Present when the list comes from the backend (`list_action_proposals`):
  /// which message proposed it, and where it is in the state machine.
  message_id?: number | null;
  state?: "pending" | "approved" | "denied" | "executing" | "executed" | "failed" | "expired";
}

export interface SysExecuteResult {
  ok: boolean;
  action: string;
  target: string;
  output: string;
  error: string;
}

/// A bundled rules document (`skills/vara/*/SKILL.md`), shown read-only.
export interface SkillDoc {
  name: string;
  description: string;
  body: string;
  path: string;
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
  mission_started: number | null;
  sys_actions: SysAction[];
  proposals?: ActionProposal[];
  error: string | null;
}

export interface UpdateInfo {
  version: string;
  current_version: string;
  notes: string;
}

/** One plugin as the UI renders it. Mirrors `plugin_bridge::PluginView`. */
export interface PluginView {
  id: string;
  name: string;
  version: string;
  summary: string;
  /** Slot names: tool, toolset, brain, memory, interface, theme, persona,
   *  channel, goal_engine, subagent, mcp, skill */
  slots: string[];
  shipped: boolean;
  enabled: boolean;
  approved: boolean;
  /** "verified" | "unverified" | "BROKEN" */
  integrity: string;
  /** What it asks for, in the owner's words. */
  asks: string[];
  path: string;
}

export interface PluginReport {
  plugins: PluginView[];
  errors: string[];
  /** The plugins that would load next start, in dependency order. */
  plan: string[];
  plan_error: string | null;
  user_dir: string;
  enabled_count: number;
  installed_count: number;
}