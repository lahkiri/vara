<script lang="ts">
  import { app, chat, streamingNow, lastMissionProposal, newConversation, openConversation, deleteConversation, sendMessage, stopStreaming, stripMissionBlock } from "../state.svelte";
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
    return mdToHtml(stripMissionBlock(md));
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
    const defaults = app.settings?.mission_defaults ?? { budget_tokens: 30000, max_steps: 14 };
    try {
      await api.startMission(goal, defaults.budget_tokens, defaults.max_steps);
      app.view = "missions";
    } catch {
      chat.error = "…";
    }
  }

  function timeLabel(ts: string): string {
    return ts.split(" ").slice(-1)[0] ?? ts;
  }

  // quick-ask handoff from the dashboard hero
  $effect(() => {
    const q = chat.pendingQuestion;
    if (q) {
      chat.pendingQuestion = "";
      void send(q);
    }
  });
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
          <button class="text-[11px] text-[var(--accent)]" onclick={() => (app.view = "missions")}>
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
            {#if m.role === "user"}
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
