// Browser mock of the Tauri backend — lets the whole UI run (and be visually
// verified) in a plain browser with believable demo data and streaming.

import type { Api } from "./api";
import type { Bootstrap, EntityStatus, MissionDetail } from "./api";
import type {
  Mission,
  Note,
  ReportRecord,
  EventRecord,
  Settings,
  Conversation,
  ChatMessageRecord,
  SendChatStart,
} from "./types";

const settings: Settings = {
  language: "ar",
  persona_style: "classic",
  provider: {
    base_url: "https://api.z.ai/api/paas/v4",
    api_key: "",
    model: "glm-4.6",
    temperature: 0.4,
  },
  autonomy: {
    heartbeat_enabled: false,
    heartbeat_minutes: 30,
    close_to_tray: true,
    notifications_enabled: true,
    open_urls: true,
    open_paths: true,
    autostart: false,
  },
  mission_defaults: { budget_tokens: 30000, max_steps: 14 },
  watched_folder: null,
};

const missions: Mission[] = [
  {
    id: 1,
    created_at: "2026-09-30 21:12",
    goal: "اجمع أدلة موثوقة حول أفضل خوادم الاستدلال المحلي لمعالجات GPU متواضعة واكتب تقريراً بمصادر موثقة",
    status: "completed",
    budget_tokens: 30000,
    spent_tokens: 2861,
    max_steps: 14,
    steps_done: 4,
    dimensions: [
      { name: "الأداء", question: "أي خادم يقدم أعلى معدل tokens/ثانية؟" },
      { name: "الذاكرة", question: "ما حجم الذاكرة المطلوب لكل خيار؟" },
    ],
    error: null,
  },
];

const missionDetail: MissionDetail = {
  mission: missions[0],
  actions: [
    { id: 3, mission_id: 1, ts: "2026-09-30 21:13", kind: "search", ok: true, summary: "new sources: 5" },
    { id: 2, mission_id: 1, ts: "2026-09-30 21:12", kind: "fetch", ok: false, summary: "HTTP 404 Not Found" },
    { id: 1, mission_id: 1, ts: "2026-09-30 21:12", kind: "plan", ok: true, summary: "dimensions: 2, steps: 4" },
  ],
  sources: [
    { id: 1, mission_id: 1, url: "https://github.com/ggml-org/llama.cpp", title: "llama.cpp — LLM inference in C/C++", fetched: true },
    { id: 2, mission_id: 1, url: "https://docs.vllm.ai/en/latest/", title: "vLLM Documentation", fetched: true },
    { id: 3, mission_id: 1, url: "https://ollama.com/blog", title: "Ollama Blog", fetched: false },
  ],
};

const reports: ReportRecord[] = [
  {
    id: 1,
    mission_id: 1,
    created_at: "2026-09-30 21:14",
    markdown:
      "# خوادم الاستدلال المحلي: المقارنة العملية\n\n## الخلاصة\nللأجهزة المتواضعة، يظل **llama.cpp** الخيار الأمثل بفضل الكمّية الفعالة (Q4/Q5) ودعم wide CPU/GPU [1]، بينما يتقدم **vLLM** عندما تتوفر ذاكرة وفيرة وتعدد مستخدمين [2].\n\n## الأداء\n- llama.cpp: تشغيل GGUF بسلاسة على 8GB [1]\n- vLLM: throughput أعلى بوضوح على A100 [2]\n\n## التوصية\nابدأ بـ llama.cpp ثم انقل إلى vLLM عند الحاجة للخدمة الجماعية [1][2].",
    sources_json: [
      { url: "https://github.com/ggml-org/llama.cpp", title: "llama.cpp", fetched: true },
      { url: "https://docs.vllm.ai/en/latest/", title: "vLLM Docs", fetched: true },
    ],
    check_json: {
      verdict: "pass",
      rule: "structural-provenance-v2",
      metrics: { cited_total: 4, retrieved_total: 4, backed_ratio: 1.0 },
      cited_not_retrieved: [],
      retrieved_not_cited: [],
      unresolved_refs: [],
      sections: [
        { title: "الخلاصة", citations: 2, backed_citations: 2, effectively_uncovered: false, no_citations: false },
        { title: "الأداء", citations: 2, backed_citations: 2, effectively_uncovered: false, no_citations: false },
      ],
      checks: { c1_all_cited_in_retrieved: true, c2_refs_resolve_to_source_list: true },
    },
    backed_ratio: 1.0,
    verdict: "pass",
    repaired: false,
  },
];

const notes: Note[] = [
  {
    id: 3,
    created_at: "2026-09-30 21:14",
    kind: "research",
    title: "llama.cpp يشغل نماذج GGUF المكمّمة بكفاءة على CPU",
    body: "llama.cpp runs quantized GGUF models efficiently on CPU — the reference engine for local inference on modest hardware.",
    mission_id: 1,
    source_url: "https://github.com/ggml-org/llama.cpp",
    source_title: "llama.cpp",
  },
  {
    id: 2,
    created_at: "2026-09-30 21:13",
    kind: "research",
    title: "vLLM يتفوق في الإنتاجية الجماعية",
    body: "vLLM provides high-throughput batch serving with paged attention when GPU memory is abundant.",
    mission_id: 1,
    source_url: "https://docs.vllm.ai/en/latest/",
    source_title: "vLLM Documentation",
  },
  {
    id: 1,
    created_at: "2026-09-29 19:02",
    kind: "manual",
    title: "تفضيلات المالك",
    body: "المالك يفضل الإجابات العربية المباشرة والقصيرة، ويهتم بالخصوصية والتشغيل المحلي.",
    mission_id: null,
    source_url: null,
    source_title: null,
  },
];

const events: EventRecord[] = [
  { id: 6, ts: "2026-09-30 21:14", level: "info", kind: "checker", message: "provenance PASS — backed 4/4" },
  { id: 5, ts: "2026-09-30 21:14", level: "info", kind: "report", message: "report written (clean-context writer)" },
  { id: 4, ts: "2026-09-30 21:13", level: "info", kind: "dedup", message: "8 duplicate notes rejected" },
  { id: 3, ts: "2026-09-30 21:13", kind: "search", level: "info", message: "new sources: 5" },
  { id: 2, ts: "2026-09-30 21:12", level: "warn", kind: "fetch", message: "https://en.wikipedia.org/wiki/Ahlan_wa_sahlan: HTTP 404" },
  { id: 1, ts: "2026-09-30 21:12", level: "info", kind: "plan", message: "dimensions: 2, steps: 4" },
];

let conversations: Conversation[] = [
  {
    id: 1,
    created_at: "2026-09-30 22:40",
    updated_at: "2026-09-30 22:41",
    title: "مرحباً، من أنتِ؟",
    mission_id: null,
  },
];

let nextMsgId = 100;
const messages = new Map<number, ChatMessageRecord[]>([
  [
    1,
    [
      {
        id: 1,
        conversation_id: 1,
        role: "assistant",
        content:
          "مرحباً! أنا **فارَا** — كيان مستمر يسكن جهازك، لا مساعد يولد مع كل سؤال.\n\nأتذكر ما نجمعه من مهامي، ويمكنني تنفيذ مهام بحثية بمصادر موثقة. اسألني أي شيء، أو اطلب مني بحثاً وأتكفل به.",
        model: "glm-4.6",
        tokens: 64,
        status: "ok",
        created_at: "2026-09-30 22:40",
      },
      {
        id: 2,
        conversation_id: 1,
        role: "user",
        content: "وما الذي يميزك عن أي شات بوت آخر؟",
        model: null,
        tokens: 12,
        status: "ok",
        created_at: "2026-09-30 22:41",
      },
      {
        id: 3,
        conversation_id: 1,
        role: "assistant",
        content:
          "ثلاثة أشياء جوهرية:\n\n1. **الاستمرارية** — أعيش في شريط المهام حتى بعد إغلاق النافذة، وذاكرتي محلية دائمة.\n2. **العمل الحقيقي** — لست مقتصرة على الكلام؛ أنفّذ مهام بحث تخطط وتبحث وتقرأ وتكتب تقريراً.\n3. **الصدق القابل للفحص** — كل تقرير يمر على مدقق مصادر يتحقق أن كل استشهاد جُلب فعلاً، لا اخترع.\n\nجرّبني: اطلب بحثاً عن أي موضوع يهمك.",
        model: "glm-4.6",
        tokens: 118,
        status: "ok",
        created_at: "2026-09-30 22:41",
      },
    ],
  ],
]);

// replies the mock entity streams, matched loosely on keywords
const CANNED: { match: RegExp; reply: string }[] = [
  {
    match: /بحث|قارن|تقرير|research|compare/i,
    reply:
      "سؤال ممتاز. بناءً على ما أذكره من مهامي السابقة، llama.cpp خيار مثالي للتشغيل المحلي على أجهزة متواضعة، بينما يتفوق vLLM عند توفر ذاكرة كبيرة.\n\nلكن إجابة مؤكدة تحتاج بحثاً حياً بمصادر موثقة — وهذا تخصصي.\n\n[[mission]] قارن بين llama.cpp و vLLM و Ollama في أداء واستهلاك الذاكرة على GPU متواضع، واكتب تقريراً موثقاً بالمصادر [[/mission]]",
  },
  {
    match: /من انت|من أنت|who are you|identity/i,
    reply:
      "أنا **فارَا** — كيان مستمر يسكن جهازك.\n\nلست نموذجاً في متصفح: عندي ذاكرة محلية دائمة، وأنفذ مهام بحثية حقيقية تخطط وتقرأ وتكتب تقارير موثقة، وأبقى حاضرة في شريط المهام حتى وأنت لا تنظر.",
  },
  {
    match: /.*/,
    reply:
      "فهمت قصدك، وهذا ما أعرفه من ذاكرتي حتى الآن: كل بحث أجريته سابقاً يؤكد أن التشغيل المحلي للنماذج صار عملياً بحجم الذاكرة المتاح.\n\nإن أردت عمقاً أكبر، أستطيع فتح مهمة بحث حقيقية الآن وإحالتك تقريراً موثقاً.",
  },
];

function pushMsg(convId: number, m: Omit<ChatMessageRecord, "id" | "conversation_id">): ChatMessageRecord {
  const rec: ChatMessageRecord = { ...m, id: nextMsgId++, conversation_id: convId };
  const arr = messages.get(convId) ?? [];
  arr.push(rec);
  messages.set(convId, arr);
  return rec;
}

async function streamReply(convId: number, assistantId: number, full: string): Promise<void> {
  const chunks = full.match(/\S+\s*|\s+/g) ?? [full];
  for (const c of chunks) {
    await new Promise((r) => setTimeout(r, 45));
    const arr = messages.get(convId);
    const m = arr?.find((x) => x.id === assistantId);
    if (m) m.content += c;
    window.dispatchEvent(
      new CustomEvent("mock:chat/delta", { detail: { conversation_id: convId, message_id: assistantId, delta: c } }),
    );
  }
  window.dispatchEvent(
    new CustomEvent("mock:chat/done", {
      detail: {
        conversation_id: convId,
        message_id: assistantId,
        content: full,
        tokens: Math.round(full.length / 4),
        model: "glm-4.6 (demo)",
        status: "ok",
        mission_goal: null,
        error: null,
      },
    }),
  );
}

let nextConvId = 2;

export const mockApi: Api = {
  bootstrap: async (): Promise<Bootstrap> => ({
    settings,
    status: { busy: false, paused: false, active_mission: null, stats: { missions_total: 1, missions_completed: 1, notes_total: notes.length, reports_total: 1, avg_backed_ratio: 1.0 } },
    fts_enabled: true,
    data_dir: "C:\\Users\\demo\\AppData\\Roaming\\app.vara.entity (demo)",
  }),
  status: async (): Promise<EntityStatus> => ({
    busy: false,
    paused: false,
    active_mission: null,
    stats: { missions_total: 1, missions_completed: 1, notes_total: notes.length, reports_total: 1, avg_backed_ratio: 1.0 },
  }),
  saveSettings: async (s: Settings) => {
    Object.assign(settings, s);
  },
  testProvider: async () => ({ ok: true, latency_ms: 340, reply: "OK", error: "" }),
  startMission: async (goal: string) => {
    const id = missions.length + 1;
    missions.unshift({
      id,
      created_at: "الآن",
      goal,
      status: "completed",
      budget_tokens: 30000,
      spent_tokens: 2911,
      max_steps: 14,
      steps_done: 5,
      dimensions: [{ name: "تجريبي", question: "وضع العرض فقط" }],
      error: null,
    });
    return id;
  },
  pause: async () => {},
  cancelMission: async () => {},
  missions: async () => missions,
  missionDetail: async () => missionDetail,
  reports: async () => reports,
  report: async (id: number) => reports.find((r) => r.id === id) ?? reports[0],
  notes: async () => notes,
  addNote: async (title: string, body: string) => {
    notes.unshift({ id: notes.length + 1, created_at: "الآن", kind: "manual", title, body, mission_id: null, source_url: null, source_title: null });
    return notes[0].id;
  },
  deleteNote: async (id: number) => {
    const i = notes.findIndex((n) => n.id === id);
    if (i >= 0) notes.splice(i, 1);
  },
  events: async () => events,
  sysOpen: async () => {},
  exportReport: async () => "C:\\demo\\vara_report_1.md",
  showWindow: async () => {},

  createConversation: async (title: string | null, missionId: number | null): Promise<Conversation> => {
    const conv: Conversation = {
      id: nextConvId++,
      created_at: "الآن",
      updated_at: "الآن",
      title: title ?? "",
      mission_id: missionId,
    };
    conversations.unshift(conv);
    messages.set(conv.id, []);
    return conv;
  },
  conversations: async () => conversations,
  renameConversation: async (id: number, title: string) => {
    const c = conversations.find((x) => x.id === id);
    if (c) c.title = title;
  },
  deleteConversation: async (id: number) => {
    conversations = conversations.filter((c) => c.id !== id);
    messages.delete(id);
  },
  messages: async (conversationId: number): Promise<ChatMessageRecord[]> => messages.get(conversationId) ?? [],
  startReportDiscussion: async (reportId: number): Promise<Conversation> => {
    const conv: Conversation = {
      id: nextConvId++,
      created_at: "الآن",
      updated_at: "الآن",
      title: `مناقشة التقرير #${reportId}`,
      mission_id: reports.find((r) => r.id === reportId)?.mission_id ?? null,
    };
    conversations.unshift(conv);
    messages.set(conv.id, []);
    return conv;
  },
  sendChat: async (conversationId: number, content: string): Promise<SendChatStart> => {
    const user = pushMsg(conversationId, { role: "user", content, model: null, tokens: 8, status: "ok", created_at: "الآن" });
    if (conversations.find((c) => c.id === conversationId)?.title === "") {
      const c = conversations.find((x) => x.id === conversationId)!;
      c.title = content.slice(0, 48);
    }
    const assistant = pushMsg(conversationId, { role: "assistant", content: "", model: null, tokens: 0, status: "streaming", created_at: "الآن" });
    void (async () => {
      await new Promise((r) => setTimeout(r, 700));
      const canned = CANNED.find((c) => c.match.test(content))!;
      await streamReply(conversationId, assistant.id, canned.reply);
    })();
    return { conversation_id: conversationId, user_message_id: user.id, assistant_message_id: assistant.id };
  },
  stopChat: async () => {},
  checkForUpdate: async () => null,
  installUpdate: async () => {},
};
