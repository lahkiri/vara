<script lang="ts">
  import { onMount } from "svelte";
  import { app, chat, refreshStatus, loadConversations, openConversation } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { api, type EntityEvent, type ReportRecord } from "../api";
  import Avatar from "./Avatar.svelte";
  import ProvenanceBadge from "./ProvenanceBadge.svelte";

  let goal = $state("");
  let budget = $state(30000);
  let steps = $state(14);
  let starting = $state(false);
  let error = $state("");
  let quick = $state("");
  let recent = $state<ReportRecord[]>([]);

  onMount(async () => {
    recent = await api.reports(5).catch(() => []);
    budget = app.settings?.mission_defaults?.budget_tokens ?? 30000;
    steps = app.settings?.mission_defaults?.max_steps ?? 14;
  });

  $effect(() => {
    // refresh recent reports when a new one arrives
    void app.stats.reports_total;
    api.reports(5).then((r) => (recent = r)).catch(() => {});
  });

  function firstLine(md: string): string {
    const l = md.split("\n").find((x) => x.trim().length > 0) ?? "";
    return l.replace(/^#+\s*/, "").slice(0, 80);
  }

  const busyState = $derived(
    app.paused ? "paused" : app.activeMission
      ? app.activeMission.status === "planning"
        ? "deliberating"
        : "working"
      : "attentive"
  );

  function greeting(): string {
    const h = new Date().getHours();
    if (h >= 5 && h < 12) return t("greeting_morning");
    if (h >= 12 && h < 21) return t("greeting_evening");
    return t("greeting_night");
  }

  function quickAsk() {
    const q = quick.trim();
    if (!q) return;
    quick = "";
    chat.pendingQuestion = q;
    app.view = "chat";
  }

  // Live checklist like the brand panel: "Vara is working..."
  const checklist = $derived(
    app.feed
      .filter((e) => e.type === "activity" && ["search", "fetch", "plan", "replan", "dedup", "report", "checker", "error"].includes(e.kind))
      .slice(0, 7)
  );

  const kindLabel: Record<string, () => string> = {
    search: () => t("events_search"),
    fetch: () => t("events_fetch"),
    plan: () => t("events_plan"),
    replan: () => t("events_replan"),
    dedup: () => t("events_dedup"),
    report: () => t("events_report"),
    checker: () => t("events_checker"),
    error: () => t("events_error"),
  };

  const stepIcon: Record<string, string> = {
    search: "⌕",
    fetch: "☰",
    plan: "◆",
    replan: "◈",
    dedup: "⧉",
    report: "▤",
    checker: "✓",
    error: "✗",
  };

  async function startMission() {
    error = "";
    if (!goal.trim()) return;
    starting = true;
    try {
      await api.startMission(goal.trim(), budget, steps);
      goal = "";
      await refreshStatus();
      app.view = "missions";
    } catch (e) {
      error = String(e);
    } finally {
      starting = false;
    }
  }

  async function togglePause() {
    const next = !app.paused;
    app.paused = next;
    await api.pause(next).catch(() => {});
  }

  async function cancelMission() {
    await api.cancelMission().catch((e) => (error = String(e)));
    await refreshStatus();
  }

  async function discuss(r: ReportRecord) {
    const conv = await api.startReportDiscussion(r.id).catch(() => null);
    if (!conv) return;
    await loadConversations();
    await openConversation(conv.id);
    app.view = "chat";
  }
</script>

<!-- companion hero -->
<section class="card p-6 flex items-center gap-6 fade-up relative overflow-hidden">
  <div class="absolute -top-24 -end-24 w-72 h-72 rounded-full opacity-30 blur-3xl pointer-events-none" style="background: radial-gradient(circle, var(--accent-soft), transparent 70%);"></div>
  <div class="avatar-ring rounded-2xl relative">
    <Avatar state={busyState} style={app.settings?.persona_style ?? "classic"} size={104} />
  </div>
  <div class="flex-1 min-w-0 relative">
    <div class="flex items-center gap-3 flex-wrap">
      <h1 class="text-2xl font-extrabold">{greeting()} — فارَا هنا</h1>
      <span class="chip" class:pulse={app.busy}>
        {app.busy ? t("vara_is_working") : app.paused ? t("vara_paused") : t("vara_idle")}
      </span>
    </div>
    <p class="text-[var(--muted)] mt-1">{t("app_tagline")}</p>
    <div class="flex items-center gap-2 mt-4">
      <input
        class="input flex-1"
        placeholder={t("quick_ask")}
        bind:value={quick}
        onkeydown={(e) => e.key === "Enter" && quickAsk()}
      />
      <button class="btn" onclick={quickAsk} disabled={!quick.trim()}>✦</button>
    </div>
    {#if !app.settings?.provider.base_url}
      <button class="chip mt-2 border-[var(--warn)] text-[var(--warn)]" onclick={() => (app.view = "settings")}>
        ⚠ {t("no_provider")}
      </button>
    {/if}
  </div>
  {#if app.busy || app.paused}
    <div class="flex flex-col gap-2 relative">
      <button class="btn-ghost text-xs" onclick={togglePause}>
        {app.paused ? t("resume") : t("pause")}
      </button>
      {#if app.busy}
        <button class="btn-ghost text-xs border-[var(--bad)] text-[var(--bad)]" onclick={cancelMission}>
          {t("cancel_mission")}
        </button>
      {/if}
    </div>
  {/if}
</section>

<!-- live working panel -->
{#if app.busy || app.paused}
  <section class="card p-6 mt-4 fade-up border-[var(--accent)]/40">
    <h2 class="font-bold mb-3 flex items-center gap-2">
      <span class="w-2 h-2 rounded-full bg-[var(--accent)] pulse inline-block"></span>
      {t("vara_is_working")}
      {#if app.activeMission}
        <span class="chip ms-auto">
          {app.activeMission.steps_done}/{app.activeMission.max_steps} {t("mission_steps")} ·
          {app.activeMission.spent_tokens.toLocaleString()}/{app.activeMission.budget_tokens.toLocaleString()} {t("mission_tokens")}
        </span>
      {/if}
    </h2>
    {#if checklist.length === 0}
      <div class="text-[var(--muted)] text-sm">{t("activity_empty")}</div>
    {:else}
      <ul class="flex flex-col gap-2">
        {#each checklist as ev, i (i)}
          {@const act = ev as Extract<EntityEvent, { type: "activity" }>}
          <li class="flex items-center gap-3 text-sm fade-up">
            <span class="w-6 h-6 rounded-lg grid place-items-center text-xs bg-[var(--accent-soft)] text-[var(--accent)] font-bold">
              {stepIcon[act.kind] ?? "•"}
            </span>
            <span class="text-[var(--muted)] text-xs w-24 shrink-0">{kindLabel[act.kind]?.() ?? act.kind}</span>
            <span class="truncate">{act.message}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<!-- mission composer (research is how the entity works, opened from her) -->
<section class="card p-6 mt-4 fade-up">
  <h2 class="font-bold mb-3">{t("give_mission")}</h2>
  <textarea
    class="input min-h-24 resize-y"
    placeholder={t("mission_placeholder")}
    bind:value={goal}
    disabled={app.busy}
  ></textarea>
  <details class="mt-3 text-xs text-[var(--muted)]">
    <summary class="cursor-pointer w-fit">{t("advanced_opts")}</summary>
    <div class="flex flex-wrap items-center gap-4 mt-2">
      <label class="flex items-center gap-2">
        {t("budget")}
        <select class="input !w-auto !py-1.5" bind:value={budget} disabled={app.busy}>
          {#each [9000, 30000, 60000, 90000] as b}
            <option value={b}>{b.toLocaleString()}</option>
          {/each}
        </select>
      </label>
      <label class="flex items-center gap-2">
        {t("max_steps")}
        <input type="number" class="input !w-20 !py-1.5" min="4" max="40" bind:value={steps} disabled={app.busy} />
      </label>
    </div>
  </details>
  <div class="flex items-center gap-3 mt-3">
    <button class="btn ms-auto" onclick={startMission} disabled={app.busy || starting || !goal.trim()}>
      {starting ? t("starting") : t("start_mission")}
    </button>
  </div>
  {#if error}
    <div class="mt-3 text-xs text-[var(--bad)]">{error}</div>
  {/if}
</section>

<!-- stats -->
<section class="grid grid-cols-2 md:grid-cols-4 gap-4 mt-4">
  <div class="card p-4">
    <div class="text-2xl font-extrabold text-[var(--accent)]">{app.stats.missions_completed}</div>
    <div class="text-xs text-[var(--muted)] mt-1">{t("stats_missions")}</div>
  </div>
  <div class="card p-4">
    <div class="text-2xl font-extrabold text-[var(--accent)]">{app.stats.notes_total}</div>
    <div class="text-xs text-[var(--muted)] mt-1">{t("stats_notes")}</div>
  </div>
  <div class="card p-4">
    <div class="text-2xl font-extrabold text-[var(--accent)]">{app.stats.reports_total}</div>
    <div class="text-xs text-[var(--muted)] mt-1">{t("stats_reports")}</div>
  </div>
  <div class="card p-4 flex flex-col justify-between">
    <div class="text-2xl font-extrabold text-[var(--accent)]">
      {app.stats.avg_backed_ratio === null ? "—" : Math.round(app.stats.avg_backed_ratio * 100) + "%"}
    </div>
    <div class="text-xs text-[var(--muted)] mt-1">{t("stats_provenance")}</div>
  </div>
</section>

<!-- recent reports -->
<section class="card p-6 mt-4">
  <div class="flex items-center justify-between mb-3">
    <h2 class="font-bold">{t("recent_reports")}</h2>
    <button class="text-xs text-[var(--accent)]" onclick={() => (app.view = "reports")}>→</button>
  </div>
  {#if recent.length === 0}
    <div class="text-[var(--muted)] text-sm">{t("activity_empty")}</div>
  {:else}
    <ul class="flex flex-col gap-2">
      {#each recent as r (r.id)}
        <li class="flex items-center gap-3 text-sm">
          <ProvenanceBadge verdict={r.verdict} ratio={r.backed_ratio} cited={r.check_json?.metrics.cited_total ?? 0} />
          <button class="truncate hover:text-[var(--accent)]" onclick={() => (app.view = "reports")}>
            #{r.id} — {firstLine(r.markdown)}
          </button>
          <button class="chip shrink-0 hover:border-[var(--accent)] hover:text-[var(--accent)]" onclick={() => discuss(r)}>
            ✷ {t("discuss_report")}
          </button>
          <span class="ms-auto text-[var(--muted)] text-xs shrink-0">{r.created_at}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
