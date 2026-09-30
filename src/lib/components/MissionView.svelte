<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { api, type MissionDetail } from "../api";
  import type { Mission } from "../types";

  let missions = $state<Mission[]>([]);
  let detail = $state<MissionDetail | null>(null);
  let loading = $state(true);

  const statusT: Record<string, () => string> = {
    planning: () => t("status_planning"),
    running: () => t("status_running"),
    completed: () => t("status_completed"),
    failed: () => t("status_failed"),
    cancelled: () => t("status_cancelled"),
  };

  const statusCls: Record<string, string> = {
    planning: "border-[var(--warn)] text-[var(--warn)]",
    running: "border-[var(--accent)] text-[var(--accent)]",
    completed: "border-[var(--ok)] text-[var(--ok)]",
    failed: "border-[var(--bad)] text-[var(--bad)]",
    cancelled: "border-[var(--line)] text-[var(--muted)]",
  };

  async function load() {
    loading = true;
    missions = await api.missions(50).catch(() => []);
    loading = false;
  }

  async function open(id: number) {
    detail = await api.missionDetail(id).catch(() => null);
  }

  $effect(() => {
    void app.stats.missions_total;
    void app.feed.length;
    const id = app.activeMission?.id;
    if (id) open(id).catch(() => {});
  });

  onMount(load);
</script>

<div class="flex flex-col gap-4">
  {#if loading}
    <div class="card p-6 text-[var(--muted)]">…</div>
  {:else if missions.length === 0}
    <div class="card p-10 text-center text-[var(--muted)]">{t("missions_none")}</div>
  {:else}
    <div class="flex flex-col gap-3">
      {#each missions as m (m.id)}
        <div class="card p-5 fade-up">
          <div class="flex items-center gap-3 flex-wrap">
            <span class="chip {statusCls[m.status] ?? ''}">{statusT[m.status]?.() ?? m.status}</span>
            <span class="text-xs text-[var(--muted)]">#{m.id} · {m.created_at}</span>
            <span class="ms-auto text-xs text-[var(--muted)]">
              {m.steps_done}/{m.max_steps} {t("mission_steps")} · {m.spent_tokens.toLocaleString()}/{m.budget_tokens.toLocaleString()} {t("mission_tokens")}
            </span>
          </div>
          <button class="text-start mt-2 font-semibold hover:text-[var(--accent)] w-full" onclick={() => open(m.id)}>
            {m.goal}
          </button>
          {#if m.dimensions}
            <div class="flex flex-wrap gap-2 mt-2">
              {#each m.dimensions as d}
                <span class="chip">{d.name}</span>
              {/each}
            </div>
          {/if}
          {#if m.error}
            <div class="text-xs text-[var(--bad)] mt-2">{m.error}</div>
          {/if}

          {#if detail && detail.mission.id === m.id}
            <div class="mt-4 border-t border-[var(--line)] pt-4 grid md:grid-cols-2 gap-4">
              <div>
                <h3 class="text-xs font-bold text-[var(--muted)] mb-2 uppercase">{t("live_feed")}</h3>
                <ul class="flex flex-col gap-1 max-h-64 overflow-y-auto pe-1">
                  {#each detail.actions as a (a.id)}
                    <li class="text-xs flex gap-2">
                      <span class={a.ok ? "text-[var(--ok)]" : "text-[var(--bad)]"}>{a.ok ? "✓" : "✗"}</span>
                      <span class="font-bold">{a.kind}</span>
                      <span class="text-[var(--muted)] truncate">{a.summary}</span>
                    </li>
                  {/each}
                </ul>
              </div>
              <div>
                <h3 class="text-xs font-bold text-[var(--muted)] mb-2 uppercase">{t("mission_sources")} ({detail.sources.length})</h3>
                <ul class="flex flex-col gap-1 max-h-64 overflow-y-auto pe-1">
                  {#each detail.sources as s (s.id)}
                    <li class="text-xs flex items-center gap-2">
                      <span class={s.fetched ? "text-[var(--ok)]" : "text-[var(--muted)]"} title={s.fetched ? t("fetched") : t("seen_only")}>
                        {s.fetched ? "☰" : "⌕"}
                      </span>
                      <button class="truncate hover:text-[var(--accent)]" onclick={() => api.sysOpen(s.url).catch(() => {})} title={s.url}>
                        {s.title || s.url}
                      </button>
                    </li>
                  {/each}
                </ul>
              </div>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>
