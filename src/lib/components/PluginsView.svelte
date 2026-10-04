<script lang="ts">
  /**
   * The plugin list — every capability the entity has, with the owner's switch
   * on it.
   *
   * The design rule this screen follows: **never hide a refusal**. A plugin that
   * cannot be enabled says why, in the place where the owner tried to enable it.
   * A plugin whose manifest does not match its hash is shown as broken rather
   * than filtered out, because "I installed that" and "it is not running" must
   * not both be true without an explanation.
   */
  import { onMount } from "svelte";
  import { api } from "../api";
  import type { PluginReport, PluginView } from "../types";
  import { t } from "../i18n.svelte";

  let report = $state<PluginReport | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let busyId = $state<string | null>(null);
  /** The refusal the core returned for the last attempt, shown in place. */
  let refusal = $state<{ id: string; message: string } | null>(null);
  let openSlots = $state<Record<string, boolean>>({});

  async function load() {
    try {
      report = await api.listPlugins();
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function toggle(plugin: PluginView) {
    busyId = plugin.id;
    refusal = null;
    try {
      report = await api.setPluginEnabled(plugin.id, !plugin.enabled);
    } catch (e) {
      // The core's reason, verbatim — the UI does not paraphrase policy.
      refusal = { id: plugin.id, message: String(e) };
    } finally {
      busyId = null;
    }
  }

  async function approve(plugin: PluginView) {
    busyId = plugin.id;
    refusal = null;
    try {
      report = await api.approvePlugin(plugin.id);
    } catch (e) {
      refusal = { id: plugin.id, message: String(e) };
    } finally {
      busyId = null;
    }
  }

  const slotLabels: Record<string, string> = {
    tool: "أدوات",
    toolset: "حزمة أدوات",
    brain: "نماذج",
    memory: "ذاكرة",
    interface: "واجهات",
    theme: "ثيمات",
    persona: "شخصيات",
    channel: "قنوات",
    goal_engine: "محرّكات أهداف",
    subagent: "وكلاء فرعيون",
    mcp: "MCP",
    skill: "مهارات",
  };

  let grouped = $derived.by(() => {
    const map: Record<string, PluginView[]> = {};
    for (const p of report?.plugins ?? []) {
      for (const slot of p.slots) {
        (map[slot] ??= []).push(p);
      }
    }
    return map;
  });

  function integrityClass(value: string): string {
    if (value === "BROKEN") return "pi-broken";
    if (value === "unverified") return "pi-unverified";
    return "pi-verified";
  }

  function integrityLabel(value: string): string {
    if (value === "BROKEN") return t("plugins_integrity_broken");
    if (value === "unverified") return t("plugins_integrity_unverified");
    return t("plugins_integrity_verified");
  }
</script>

<section class="plugins">
  {#if loading}
    <p class="muted">{t("loading")}</p>
  {:else if error}
    <p class="err">{error}</p>
  {:else if report}
    <header class="head">
      <div>
        <h2>{t("plugins_title")}</h2>
        <p class="muted small">
          {t("plugins_summary")
            .replace("{enabled}", String(report.enabled_count))
            .replace("{installed}", String(report.installed_count))}
        </p>
      </div>
      <button class="ghost" onclick={() => api.openUserPluginDir()}>
        {t("plugins_open_folder")}
      </button>
    </header>

    {#if report.plan_error}
      <p class="warn">{t("plugins_plan_error")} {report.plan_error}</p>
    {/if}

    {#each report.errors as message}
      <p class="warn small">{message}</p>
    {/each}

    {#each Object.entries(grouped) as [slot, plugins]}
      <details
        class="group"
        open={openSlots[slot] ?? true}
        ontoggle={(e) => (openSlots[slot] = (e.currentTarget as HTMLDetailsElement).open)}
      >
        <summary>
          <span>{slotLabels[slot] ?? slot}</span>
          <span class="count">{plugins.length}</span>
        </summary>

        {#each plugins as plugin (plugin.id)}
          <article class="row" class:on={plugin.enabled} class:broken={plugin.integrity === "BROKEN"}>
            <div class="info">
              <div class="line">
                <span class="name">{plugin.name}</span>
                <span class="ver">{plugin.version}</span>
                <span class="tag {integrityClass(plugin.integrity)}">
                  {integrityLabel(plugin.integrity)}
                </span>
                {#if !plugin.shipped}<span class="tag shipped">{t("plugins_external")}</span>{/if}
              </div>
              <p class="summary">{plugin.summary}</p>

              {#if plugin.asks.length > 0}
                <p class="asks">
                  {t("plugins_asks")} <strong>{plugin.asks.join(" · ")}</strong>
                  {#if !plugin.approved}
                    <span class="pending">— {t("plugins_needs_approval")}</span>
                  {/if}
                </p>
              {/if}

              {#if refusal && refusal.id === plugin.id}
                <p class="refusal">{refusal.message}</p>
              {/if}
            </div>

            <div class="actions">
              <button class="ghost tiny" onclick={() => api.revealPlugin(plugin.id)}>
                {t("plugins_reveal")}
              </button>
              {#if !plugin.approved && plugin.asks.length > 0}
                <button
                  class="approve"
                  disabled={busyId === plugin.id}
                  onclick={() => approve(plugin)}
                >
                  {t("plugins_approve")}
                </button>
              {/if}
              <label class="switch" title={plugin.enabled ? t("plugins_disable") : t("plugins_enable")}>
                <input
                  type="checkbox"
                  checked={plugin.enabled}
                  disabled={busyId === plugin.id || plugin.integrity === "BROKEN"}
                  onchange={() => toggle(plugin)}
                />
                <span class="track"><span class="knob"></span></span>
              </label>
            </div>
          </article>
        {/each}
      </details>
    {/each}

    <p class="muted tiny">{t("plugins_user_dir")} <code>{report.user_dir}</code></p>
  {/if}
</section>

<style>
  .plugins {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }
  h2 {
    margin: 0;
    font-size: 1.05rem;
  }
  .small {
    font-size: 0.8rem;
  }
  .tiny {
    font-size: 0.72rem;
  }
  .muted {
    color: var(--muted, #8a8f98);
  }
  .err {
    color: #ff6b6b;
  }
  .warn {
    color: #f0b429;
    background: rgba(240, 180, 41, 0.08);
    border: 1px solid rgba(240, 180, 41, 0.25);
    border-radius: 8px;
    padding: 0.5rem 0.7rem;
    margin: 0;
  }
  .group {
    border: 1px solid var(--line, #23262d);
    border-radius: 10px;
    background: var(--panel, #14161a);
    overflow: hidden;
  }
  summary {
    cursor: pointer;
    display: flex;
    justify-content: space-between;
    padding: 0.55rem 0.8rem;
    font-size: 0.85rem;
    font-weight: 600;
    background: var(--panel-2, #191c21);
  }
  .count {
    color: var(--muted, #8a8f98);
    font-weight: 400;
  }
  .row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.7rem 0.8rem;
    border-top: 1px solid var(--line, #23262d);
  }
  .row.on {
    background: linear-gradient(90deg, rgba(80, 200, 160, 0.05), transparent);
  }
  .row.broken {
    background: linear-gradient(90deg, rgba(255, 90, 90, 0.07), transparent);
  }
  .info {
    min-width: 0;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    flex-wrap: wrap;
  }
  .name {
    font-weight: 600;
    font-size: 0.9rem;
  }
  .ver {
    color: var(--muted, #8a8f98);
    font-size: 0.75rem;
  }
  .summary {
    margin: 0.25rem 0 0;
    font-size: 0.8rem;
    color: var(--muted, #8a8f98);
  }
  .asks {
    margin: 0.3rem 0 0;
    font-size: 0.78rem;
    color: #f0b429;
  }
  .pending {
    color: var(--muted, #8a8f98);
  }
  .refusal {
    margin: 0.4rem 0 0;
    font-size: 0.78rem;
    color: #ff6b6b;
  }
  .tag {
    font-size: 0.68rem;
    padding: 0.1rem 0.4rem;
    border-radius: 999px;
    border: 1px solid transparent;
  }
  .pi-verified {
    color: #4ad6a0;
    border-color: rgba(74, 214, 160, 0.3);
    background: rgba(74, 214, 160, 0.08);
  }
  .pi-unverified {
    color: #f0b429;
    border-color: rgba(240, 180, 41, 0.3);
    background: rgba(240, 180, 41, 0.08);
  }
  .pi-broken {
    color: #ff6b6b;
    border-color: rgba(255, 107, 107, 0.35);
    background: rgba(255, 107, 107, 0.1);
  }
  .shipped {
    color: var(--muted, #8a8f98);
    border-color: var(--line, #23262d);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-shrink: 0;
  }
  button.ghost {
    background: transparent;
    color: var(--muted, #8a8f98);
    border: 1px solid var(--line, #23262d);
    border-radius: 7px;
    padding: 0.3rem 0.55rem;
    cursor: pointer;
  }
  button.ghost:hover {
    color: inherit;
  }
  button.approve {
    background: rgba(240, 180, 41, 0.12);
    color: #f0b429;
    border: 1px solid rgba(240, 180, 41, 0.35);
    border-radius: 7px;
    padding: 0.3rem 0.6rem;
    cursor: pointer;
  }
  .switch input {
    display: none;
  }
  .track {
    display: inline-block;
    width: 34px;
    height: 18px;
    border-radius: 999px;
    background: #2a2e36;
    position: relative;
    transition: background 0.15s;
  }
  .knob {
    position: absolute;
    top: 2px;
    inset-inline-start: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #8a8f98;
    transition: transform 0.15s, background 0.15s;
  }
  input:checked + .track {
    background: rgba(74, 214, 160, 0.35);
  }
  input:checked + .track .knob {
    transform: translateX(16px);
    background: #4ad6a0;
  }
  input:disabled + .track {
    opacity: 0.4;
    cursor: not-allowed;
  }
  code {
    font-size: 0.72rem;
  }
</style>
