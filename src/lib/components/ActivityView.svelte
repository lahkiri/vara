<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { api, type EventRecord } from "../api";

  let events = $state<EventRecord[]>([]);
  let loading = $state(true);

  const kindT: Record<string, () => string> = {
    state: () => t("events_state"),
    search: () => t("events_search"),
    fetch: () => t("events_fetch"),
    note: () => t("events_note"),
    plan: () => t("events_plan"),
    replan: () => t("events_replan"),
    report: () => t("events_report"),
    checker: () => t("events_checker"),
    error: () => t("events_error"),
    dedup: () => t("events_dedup"),
    budget: () => t("events_budget"),
    mission: () => t("events_mission"),
    reflection: () => t("events_reflection"),
    watcher: () => t("events_watcher"),
    pause: () => t("events_pause"),
    settings: () => t("events_settings"),
    app: () => t("events_app"),
    tokens: () => t("events_tokens"),
  };

  async function load() {
    loading = true;
    events = await api.events(200).catch(() => []);
    loading = false;
  }

  // Merge the live feed on top: newest first.
  const merged = $derived([
    ...app.feed
      .filter((e) => e.type === "activity")
      .map((e) => {
        const a = e as Extract<(typeof app.feed)[number], { type: "activity" }>;
        return { id: "live-" + Math.random(), ts: "", level: "info", kind: a.kind, message: a.message, live: true };
      }),
    ...events.map((e) => ({ ...e, live: false })),
  ].slice(0, 250));

  onMount(load);
</script>

<div class="card p-6">
  <div class="flex items-center justify-between mb-4">
    <h2 class="font-bold">{t("nav_activity")}</h2>
    <button class="text-xs text-[var(--accent)]" onclick={load}>⟳</button>
  </div>
  {#if loading && merged.length === 0}
    <div class="text-[var(--muted)] text-sm">…</div>
  {:else if merged.length === 0}
    <div class="text-[var(--muted)] text-sm">{t("activity_empty")}</div>
  {:else}
    <ul class="flex flex-col gap-1.5 max-h-[70vh] overflow-y-auto pe-1">
      {#each merged as e ("id" in e ? String(e.id) : "")}
        <li class="flex items-center gap-3 text-sm fade-up" class:opacity-70={!("live" in e && e.live)}>
          <span class="text-[10px] text-[var(--muted)] w-36 shrink-0 hidden md:inline">{e.ts}</span>
          <span class="chip w-20 justify-center shrink-0 {e.level === 'error' ? 'border-[var(--bad)] text-[var(--bad)]' : ''}">
            {kindT[e.kind]?.() ?? e.kind}
          </span>
          <span class="truncate">{e.message}</span>
        </li>
      {/each}
    </ul>
  {/if}
</div>
