<script lang="ts">
  import { app, chat, streamingNow, lastMissionProposal, newConversation, openConversation, deleteConversation, sendMessage, stopStreaming, stripProtocolBlocks, startMissionFromChat, executeSysAction, dismissSysActions } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { api } from "../api";
  import { mdToHtml } from "../md";
  import Avatar from "./Avatar.svelte";

  let draft = $state("");
  let copiedId = $state<number | null>(null);
  let proposalDone = $state<number | null>(null); // message id whose proposal was already turned into a mission

  const active = $derived(chat.conversations.find((c) => c.id === chat.activeId) ?? null);

  const suggestions = $derived([
    t("chat_sug_1"),
    t("chat_sug_2"),
    t("chat_sug_3"),
    t("chat_sug_4"),
  ]);

  function html(md: string): string {
    return mdToHtml(stripProtocolBlocks(md));
  }

  async function send(text: string): Promise<void> {
    const content = text.trim();
    if (!content || streamingNow()) return;
    // auto-create a thread when none exists yet (first hello)
    if (chat.activeId === null) await newConversation();
    draft = "";
    await sendMessage(content);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send(draft);
    }
  }

  async function copy(mId: number, content: string) {
    try {
      await navigator.clipboard.writeText(content);
      copiedId = mId;
      setTimeout(() => (copiedId = null), 1200);
    } catch { /* clipboard may be blocked */ }
  }

  async function turnIntoMission(goal: string, messageId: number) {
    proposalDone = messageId;
    await startMissionFromChat(goal);
  }

  function timeLabel(ts: string): string {
    return ts.split(" ").slice(-1)[0] ?? ts;
  }

  // ---------- card payloads ----------

  interface MissionCard {
    goal: string;
    mission_id: number;
  }
  function missionCard(m: { content: string }): MissionCard | null {
    try {
      const v = JSON.parse(m.content);
      if (typeof v?.goal === "string" && typeof v?.mission_id === "number") {
        return { goal: v.goal, mission_id: v.mission_id };
      }
    } catch { /* legacy rows */ }
    return null;
  }

  interface ActionCard {
    action: string;
    target: string;
    ok: boolean;
    output: string;
    error: string;
  }
  function actionCard(m: { content: string }): ActionCard | null {
    try {
      const v = JSON.parse(m.content);
      if (typeof v?.action === "string" && typeof v?.target === "string") {
        return {
          action: v.action,
          target: v.target,
          ok: Boolean(v.ok),
          output: typeof v.output === "string" ? v.output : "",
          error: typeof v.error === "string" ? v.error : "",
        };
      }
    } catch { /* ignore */ }
    return null;
  }

  function actionLabel(a: string): string {
    if (a === "open_url") return t("action_open_url");
    if (a === "open_path") return t("action_open_path");
    if (a === "screenshot") return t("action_screenshot");
    if (a === "computer_use") return t("action_computer_use");
    return t("action_run");
  }

  function statusLabel(s: string): string {
    if (s === "running") return t("status_running");
    if (s === "completed") return t("status_completed");
    if (s === "failed") return t("status_failed");
    if (s === "cancelled") return t("status_cancelled");
    return s;
  }
</script>

<div class="flex h-[calc(100vh-2rem)] mx-auto max-w-6xl gap-0 overflow-hidden rounded-2xl card !p-0">
  <!-- conversations rail -->
  <aside class="w-60 shrink-0 border-e border-[var(--line)] bg-[var(--bg2)]/40 hidden sm:flex flex-col">
    <div class="p-3">
      <button class="btn w-full text-sm" onclick={() => newConversation()}>✦ {t("chat_new")}</button>
    </div>
    <div class="flex-1 overflow-y-auto px-2 pb-3 flex flex-col gap-1">
      {#if chat.conversations.length === 0}
        <div class="text-xs text-[var(--muted)] text-center py-6 px-2">{t("chat_no_conversations")}</div>
      {/if}
      {#each chat.conversations as c (c.id)}
        <div
          class="group flex items-center rounded-xl transition-colors {chat.activeId === c.id ? 'nav-active' : 'hover:bg-[var(--card-hover)]'}"
        >
          <button
            class="text-start px-3 py-2.5 text-sm flex items-center gap-2 flex-1 min-w-0 {chat.activeId === c.id ? 'text-[var(--accent)] font-bold' : 'text-[var(--muted)]'}"
            onclick={() => openConversation(c.id)}
          >
            <span class="truncate flex-1">{c.title || t("chat_new")}</span>
            {#if c.mission_id}
              <span class="chip !px-1.5 !py-0 !text-[10px] shrink-0">▤</span>
            {/if}
          </button>
          <button
            class="opacity-0 group-hover:opacity-100 text-[var(--bad)] text-xs shrink-0 pe-3 py-2"
            title={t("chat_delete_confirm")}
            onclick={(e) => {
              if (confirm(t("chat_delete_confirm"))) void deleteConversation(c.id);
            }}>✕</button>
        </div>
      {/each}
    </div>
  </aside>

  <!-- thread -->
  <section class="flex-1 flex flex-col min-w-0">
    <!-- header -->
    <header class="px-5 py-3 border-b border-[var(--line)] flex items-center gap-3">
      <Avatar size={34} style={app.settings?.persona_style ?? "classic"} />
      <div class="min-w-0">
        <div class="font-bold text-sm truncate">{active?.title || t("chat_new")}</div>
        {#if active?.mission_id}
          <button class="text-[11px] text-[var(--accent)]" onclick={() => (app.view = "reports")}>
            ▤ {t("chat_linked_report")} — #{active.mission_id}
          </button>
        {/if}
      </div>
      {#if streamingNow()}
        <span class="chip ms-auto border-[var(--accent)] text-[var(--accent)]">{t("chat_thinking")}</span>
      {/if}
    </header>

    <!-- messages -->
    <div id="chat-scroll" class="flex-1 overflow-y-auto px-5 py-4">
      {#if chat.messages.length === 0}
        <div class="h-full flex flex-col items-center justify-center text-center gap-4 py-10">
          <div class="avatar-ring rounded-3xl overflow-hidden">
            <Avatar size={116} style={app.settings?.persona_style ?? "classic"} />
          </div>
          <h2 class="text-xl font-extrabold">{t("chat_hello_title")}</h2>
          <p class="text-sm text-[var(--muted)] max-w-md leading-7">{t("chat_hello_body")}</p>
          <div class="flex flex-wrap justify-center gap-2 max-w-xl mt-2">
            {#each suggestions as s (s)}
              <button class="chip hover:border-[var(--accent)] hover:text-[var(--accent)] !text-xs !py-2 !px-3" onclick={() => send(s)}>
                {s}
              </button>
            {/each}
          </div>
        </div>
      {:else}
        <div class="flex flex-col gap-4">
          {#each chat.messages as m (m.id)}
            {#if m.kind === "mission"}
              {@const card = missionCard(m)}
              {#if card}
                <!-- live mission card inside the thread -->
                <div class="flex justify-center fade-up">
                  <div class="mission-card w-full max-w-xl">
                    <div class="flex items-center gap-2 mb-2">
                      <span class="mission-card-icon">✦</span>
                      <span class="text-[11px] font-bold tracking-wide text-[var(--accent)]">{t("mission_live")}</span>
                      <span class="chip !text-[10px] !py-0 ms-auto {chat.liveMissions[card.mission_id]?.status === 'completed' ? 'border-[var(--ok)] text-[var(--ok)]' : chat.liveMissions[card.mission_id]?.status === 'failed' || chat.liveMissions[card.mission_id]?.status === 'cancelled' ? 'border-[var(--bad)] text-[var(--bad)]' : 'border-[var(--accent)] text-[var(--accent)]'}">
                        {statusLabel(chat.liveMissions[card.mission_id]?.status ?? "running")}
                      </span>
                    </div>
                    <div class="text-sm font-bold leading-6">{card.goal}</div>
                    {#if chat.liveMissions[card.mission_id]?.status !== "completed" && chat.liveMissions[card.mission_id]?.status !== "failed" && chat.liveMissions[card.mission_id]?.status !== "cancelled"}
                      <div class="mission-progress mt-3">
                        <div class="mission-progress-fill" style={"width:" + Math.min(100, ((chat.liveMissions[card.mission_id]?.steps_done ?? 0) / Math.max(1, chat.liveMissions[card.mission_id]?.max_steps ?? 14)) * 100) + "%"}></div>
                      </div>
                      <div class="flex items-center gap-2 mt-1.5 text-[10px] text-[var(--muted)]">
                        <span class="typing !py-0"><i></i><i></i><i></i></span>
                        <span>{chat.liveMissions[card.mission_id]?.steps_done ?? 0}/{chat.liveMissions[card.mission_id]?.max_steps ?? "?"} {t("mission_steps")} · {chat.liveMissions[card.mission_id]?.spent_tokens ?? 0} tokens</span>
                      </div>
                    {:else if chat.liveMissions[card.mission_id]?.status === "completed"}
                      <button class="btn-ghost !py-1.5 !px-3 text-xs mt-3" onclick={() => (app.view = "reports")}>
                        ▤ {t("view_reports")}
                      </button>
                    {/if}
                  </div>
                </div>
              {/if}
            {:else if m.kind === "action"}
              {@const act = actionCard(m)}
              {#if act}
                <!-- OS action receipt inside the thread -->
                <div class="flex justify-center fade-up">
                  <div class="action-card w-full max-w-xl" class:action-failed={!act.ok}>
                    <div class="flex items-center gap-2 flex-wrap">
                      <span class="text-sm">{act.action === "open_url" ? "🌐" : act.action === "open_path" ? "📂" : act.action === "screenshot" ? "🖼" : act.action === "computer_use" ? "🤖" : "⌨"}</span>
                      <span class="text-xs font-bold">{actionLabel(act.action)}</span>
                      <span class="chip !text-[10px] !py-0 ms-auto {act.ok ? 'border-[var(--ok)] text-[var(--ok)]' : 'border-[var(--bad)] text-[var(--bad)]'}">
                        {act.ok ? t("action_done") : t("action_failed")}
                      </span>
                    </div>
                    <div class="font-mono text-[11px] mt-2 break-all text-[var(--muted)]">{act.target}</div>
                    {#if act.output}
                      <pre class="action-output">{act.output}</pre>
                    {/if}
                    {#if act.error}
                      <div class="text-[11px] text-[var(--bad)] mt-1">{act.error}</div>
                    {/if}
                  </div>
                </div>
              {/if}
            {:else if m.role === "user"}
              <div class="flex justify-end fade-up">
                <div class="bubble bubble-user">
                  <div class="text-[10px] opacity-60 mb-1 text-end">{t("chat_you")}</div>
                  <div class="whitespace-pre-wrap">{m.content}</div>
                </div>
              </div>
            {:else}
              <div class="flex items-start gap-3 fade-up">
                <Avatar size={32} style={app.settings?.persona_style ?? "classic"} state={m.status === "streaming" ? "thinking" : "attentive"} />
                <div class="min-w-0 flex-1">
                  <div class="bubble bubble-vara {m.status === 'error' ? 'bubble-error' : ''}">
                    {#if m.content}
                      <div class="md-body" style="direction:{document.documentElement.dir}">{@html html(m.content)}</div>
                      {#if m.status === "streaming"}<span class="caret"></span>{/if}
                    {:else if m.status === "streaming"}
                      <span class="typing"><i></i><i></i><i></i></span>
                    {:else if m.status === "error"}
                      <span class="text-xs text-[var(--bad)]">✗ {t("chat_error_tag")}</span>
                    {/if}
                  </div>
                  <div class="flex items-center gap-2 mt-1 text-[10px] text-[var(--muted)]">
                    {#if m.status === "stopped"}
                      <span class="text-[var(--warn)]">⏸ {t("chat_stopped_tag")}</span>
                    {/if}
                    {#if m.model && m.status === "ok"}
                      <span>{m.model}</span>
                    {/if}
                    {#if m.tokens > 0}
                      <span>· {m.tokens} tokens</span>
                    {/if}
                    {#if m.content && m.status !== "streaming"}
                      <button class="hover:text-[var(--accent)]" onclick={() => copy(m.id, m.content)}>
                        {copiedId === m.id ? t("chat_copied") : t("chat_copy")}
                      </button>
                    {/if}
                    {#if m.created_at}
                      <span>· {timeLabel(m.created_at)}</span>
                    {/if}
                  </div>
                  {#if lastMissionProposal()?.messageId === m.id && proposalDone !== m.id}
                    <div class="mission-proposal p-3 mt-2 flex items-center gap-3 flex-wrap">
                      <span class="text-xs text-[var(--muted)]">◆</span>
                      <span class="text-xs flex-1 min-w-40">{lastMissionProposal()!.goal}</span>
                      <button class="btn !py-1.5 !px-3 text-xs" onclick={() => turnIntoMission(lastMissionProposal()!.goal, m.id)}>
                        ✦ {t("chat_make_mission")}
                      </button>
                    </div>
                  {/if}
                  {#if chat.pendingSys[m.id]?.length}
                    <div class="mission-proposal p-3 mt-2" style="border-style:solid">
                      <div class="text-[11px] font-bold text-[var(--warn)] mb-2">⚠ {t("sys_approval_title")}</div>
                      {#each chat.pendingSys[m.id] as a (a.target + a.action)}
                        <div class="flex items-center gap-2 flex-wrap py-1">
                          <span class="text-xs">{actionLabel(a.action)}:</span>
                          <span class="font-mono text-[11px] break-all flex-1 min-w-30">{a.target}</span>
                          <button class="btn !py-1 !px-3 text-[11px]" onclick={() => void executeSysAction(m.id, a)}>{t("sys_execute")}</button>
                          <button class="btn-ghost !py-1 !px-3 text-[11px]" onclick={() => dismissSysActions(m.id)}>{t("sys_dismiss")}</button>
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
              </div>
            {/if}
          {/each}
        </div>
      {/if}
    </div>

    <!-- composer -->
    <footer class="px-5 pb-4 pt-2 border-t border-[var(--line)]">
      {#if chat.error}
        <div class="text-xs text-[var(--bad)] mb-2">{chat.error}</div>
      {/if}
      <div class="flex items-end gap-2">
        <textarea
          class="input resize-none max-h-36 min-h-[46px] py-3"
          rows="1"
          placeholder={t("chat_placeholder")}
          bind:value={draft}
          onkeydown={onKeydown}
          oninput={(e) => {
            const el = e.currentTarget;
            el.style.height = "auto";
            el.style.height = Math.min(el.scrollHeight, 144) + "px";
          }}
        ></textarea>
        {#if streamingNow()}
          <button class="btn-ghost !px-4 h-[46px]" title={t("chat_stop")} onclick={() => stopStreaming()}>
            <span class="block w-3 h-3 bg-[var(--bad)] rounded-sm"></span>
          </button>
        {:else}
          <button class="btn !px-5 h-[46px]" disabled={!draft.trim()} onclick={() => send(draft)} title={t("chat_send")}>
            ➤
          </button>
        {/if}
      </div>
    </footer>
  </section>
</div>
