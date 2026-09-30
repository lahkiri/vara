<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../i18n.svelte";
  import { api, type ProvenanceResult } from "../api";
  import type { ReportRecord } from "../types";
  import { mdToHtml } from "../md";
  import ProvenanceBadge from "./ProvenanceBadge.svelte";

  let reports = $state<ReportRecord[]>([]);
  let selected = $state<ReportRecord | null>(null);
  let loading = $state(true);
  let exportedPath = $state("");

  async function load() {
    loading = true;
    reports = await api.reports(80).catch(() => []);
    if (reports.length > 0 && !selected) selected = reports[0];
    loading = false;
  }

  async function open(id: number) {
    selected = await api.report(id).catch(() => null);
    exportedPath = "";
  }

  async function exportReport() {
    if (!selected) return;
    exportedPath = await api.exportReport(selected.id).catch((e) => String(e));
  }

  function html(md: string): string {
    return mdToHtml(md);
  }

  function check(r: ReportRecord): ProvenanceResult | null {
    return r.check_json ?? null;
  }

  const isRepaired = $derived(selected?.repaired ?? false);
  const repairedSibling = $derived(
    selected && reports.find((r) => r.mission_id === selected!.mission_id && r.repaired && r.id !== selected!.id)
  );
  const originalSibling = $derived(
    selected && reports.find((r) => r.mission_id === selected!.mission_id && !r.repaired && r.id !== selected!.id)
  );

  onMount(load);
</script>

<div class="grid md:grid-cols-[300px_1fr] gap-4 items-start">
  <div class="flex flex-col gap-2 max-h-[80vh] overflow-y-auto pe-1">
    {#if loading}
      <div class="card p-6 text-[var(--muted)]">…</div>
    {:else if reports.length === 0}
      <div class="card p-6 text-center text-[var(--muted)] text-sm">{t("activity_empty")}</div>
    {:else}
      {#each reports as r (r.id)}
        <button
          class="card p-4 text-start hover:bg-[var(--card-hover)] transition-colors {selected?.id === r.id ? 'border-[var(--accent)]' : ''}"
          onclick={() => open(r.id)}
        >
          <div class="flex items-center gap-2 mb-1">
            <ProvenanceBadge verdict={r.verdict} ratio={r.backed_ratio} cited={r.check_json?.metrics.cited_total ?? 0} />
            {#if r.repaired}
              <span class="chip">⚡ fix</span>
            {/if}
          </div>
          <div class="text-xs text-[var(--muted)]">#{r.id} · {t("nav_missions")} #{r.mission_id} · {r.created_at}</div>
        </button>
      {/each}
    {/if}
  </div>

  <div class="card p-6 min-h-[50vh]">
    {#if !selected}
      <div class="text-[var(--muted)] text-sm">{t("activity_empty")}</div>
    {:else}
      <div class="flex items-center gap-3 flex-wrap mb-3">
        <ProvenanceBadge verdict={selected.verdict} ratio={selected.backed_ratio} cited={selected.check_json?.metrics.cited_total ?? 0} />
        <span class="chip">#{selected.id}</span>
        <button class="btn-ghost text-xs ms-auto" onclick={exportReport}>{t("export_report")}</button>
      </div>
      {#if exportedPath}
        <div class="text-xs text-[var(--ok)] mb-2">{exportedPath}</div>
      {/if}

      {#if isRepaired}
        <div class="text-xs text-[var(--warn)] mb-2 border border-[var(--warn)]/40 rounded-lg px-3 py-2">
          ⚡ {t("repair_note")}
          {#if originalSibling}
            <button class="underline ms-1" onclick={() => open(originalSibling!.id)}>{t("original_note")}</button>
          {/if}
        </div>
      {/if}

      <!-- checker details -->
      {#if check(selected)}
        {@const c = check(selected)!}
        <details class="mb-4 border border-[var(--line)] rounded-xl px-4 py-3 text-xs">
          <summary class="cursor-pointer font-bold text-[var(--muted)]">
            {t("provenance")} — {t("backed_ratio")}: {(c.metrics.backed_ratio * 100).toFixed(1)}% ({c.metrics.cited_total}/{c.metrics.retrieved_total})
          </summary>
          <div class="mt-3 grid gap-2">
            {#if c.cited_not_retrieved.length > 0}
              <div>
                <div class="font-bold text-[var(--bad)]">{t("cited_not_retrieved")} ({c.cited_not_retrieved.length}):</div>
                <ul class="list-disc ms-5 text-[var(--muted)]">
                  {#each c.cited_not_retrieved.slice(0, 12) as u}
                    <li class="truncate">{u}</li>
                  {/each}
                </ul>
              </div>
            {/if}
            {#if c.unresolved_refs.length > 0}
              <div class="text-[var(--bad)]">{t("unresolved_refs")}: {c.unresolved_refs.join(", ")}</div>
            {/if}
            {#if c.sections.some((s) => s.effectively_uncovered)}
              <div class="text-[var(--warn)]">
                {t("sections_uncovered")}:
                {c.sections.filter((s) => s.effectively_uncovered).map((s) => s.title).join(" · ")}
              </div>
            {/if}
            {#if c.cited_not_retrieved.length === 0 && c.unresolved_refs.length === 0}
              <div class="text-[var(--ok)]">✓ C1 · ✓ C2</div>
            {/if}
          </div>
        </details>
      {/if}

      <div class="md-body" style="direction:{document.documentElement.dir}">
        {@html html(selected.markdown)}
      </div>

      {#if selected.sources_json && selected.sources_json.length > 0}
        <details class="mt-5 border-t border-[var(--line)] pt-4">
          <summary class="cursor-pointer text-xs font-bold text-[var(--muted)]">
            {t("mission_sources")} ({selected.sources_json.length})
          </summary>
          <ul class="mt-2 flex flex-col gap-1 text-xs">
            {#each selected.sources_json as s, i}
              <li class="flex items-center gap-2">
                <span class="md-cite">{i + 1}</span>
                <button class="truncate hover:text-[var(--accent)]" onclick={() => api.sysOpen(s.url).catch(() => {})}>{s.title || s.url}</button>
                <span class={s.fetched ? "text-[var(--ok)]" : "text-[var(--muted)]"}>{s.fetched ? t("fetched") : t("seen_only")}</span>
              </li>
            {/each}
          </ul>
        </details>
      {/if}
    {/if}
  </div>
</div>
