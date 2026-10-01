<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { app, applyPersona, updater, checkForUpdate, installUpdate } from "../state.svelte";
  import { t, setLang, i18n } from "../i18n.svelte";
  import { api, isTauri, type TestProviderResult } from "../api";
  import type { Settings } from "../types";
  import classic from "../../assets/characters/classic.png";
  import dark from "../../assets/characters/dark.png";
  import stealth from "../../assets/characters/stealth.png";
  import tech from "../../assets/characters/tech.png";
  import nature from "../../assets/characters/nature.png";

  let draft = $state<Settings | null>(null);
  let saving = $state(false);
  let saved = $state(false);
  let testing = $state(false);
  let testResult = $state<TestProviderResult | null>(null);
  let error = $state("");
  let appVersion = $state("");

  const styles = [
    { id: "classic", img: classic, label: "Classic" },
    { id: "dark", img: dark, label: "Dark Mode" },
    { id: "stealth", img: stealth, label: "Stealth" },
    { id: "tech", img: tech, label: "Tech" },
    { id: "nature", img: nature, label: "Nature" },
  ];

  const presets = [
    { name: "Z.ai", url: "https://api.z.ai/api/paas/v4", model: "glm-4.6" },
    { name: "OpenAI", url: "https://api.openai.com/v1", model: "gpt-4o-mini" },
    { name: "Ollama", url: "http://localhost:11434/v1", model: "llama3.1" },
    { name: "LM Studio", url: "http://localhost:1234/v1", model: "local-model" },
    { name: "llama-server", url: "http://localhost:8080/v1", model: "local" },
  ];

  onMount(() => {
    if (app.settings) draft = JSON.parse(JSON.stringify(app.settings));
    if (isTauri()) {
      getVersion().then((v) => (appVersion = v)).catch(() => {});
    } else {
      appVersion = "0.3.0 (demo)";
    }
  });

  function applyPreset(p: (typeof presets)[number]) {
    if (!draft) return;
    draft.provider.base_url = p.url;
    draft.provider.model = p.model;
  }

  function pickStyle(id: string) {
    if (!draft) return;
    draft.persona_style = id;
    applyPersona(id);
  }

  async function save() {
    if (!draft) return;
    saving = true;
    error = "";
    try {
      await api.saveSettings(JSON.parse(JSON.stringify(draft)));
      app.settings = JSON.parse(JSON.stringify(draft));
      saved = true;
      setTimeout(() => (saved = false), 1500);
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  async function runTest() {
    if (!draft) return;
    testing = true;
    testResult = null;
    testResult = await api.testProvider(JSON.parse(JSON.stringify(draft.provider))).catch((e) => ({
      ok: false,
      latency_ms: 0,
      reply: "",
      error: String(e),
    }));
    testing = false;
  }
</script>

{#if draft}
  <div class="flex flex-col gap-4 max-w-3xl">
    <!-- provider -->
    <section class="card p-6">
      <h2 class="font-bold mb-4">{t("settings_provider")}</h2>
      <div class="flex flex-wrap gap-2 mb-4">
        {#each presets as p}
          <button class="chip hover:border-[var(--accent)] hover:text-[var(--accent)]" onclick={() => applyPreset(p)}>
            {p.name}
          </button>
        {/each}
      </div>
      <div class="grid gap-3">
        <label class="text-xs text-[var(--muted)]">
          {t("provider_url")}
          <input class="input mt-1" bind:value={draft.provider.base_url} placeholder="https://api.z.ai/api/paas/v4" />
        </label>
        <label class="text-xs text-[var(--muted)]">
          {t("provider_model")}
          <input class="input mt-1" bind:value={draft.provider.model} placeholder="glm-4.6 / llama3.1 / ..." />
        </label>
        <label class="text-xs text-[var(--muted)]">
          {t("provider_key")}
          <input class="input mt-1" type="password" bind:value={draft.provider.api_key} placeholder="sk-..." />
        </label>
      </div>
      <div class="flex items-center gap-3 mt-4">
        <button class="btn-ghost text-xs" onclick={runTest} disabled={testing}>
          {testing ? t("testing") : t("test_connection")}
        </button>
        {#if testResult}
          {#if testResult.ok}
            <span class="text-xs text-[var(--ok)]">✓ {t("test_ok")} ({testResult.latency_ms} ms) — “{testResult.reply}”</span>
          {:else}
            <span class="text-xs text-[var(--bad)] truncate max-w-md">✗ {testResult.error}</span>
          {/if}
        {/if}
      </div>
    </section>

    <!-- persona -->
    <section class="card p-6">
      <h2 class="font-bold mb-4">{t("persona_style")}</h2>
      <div class="flex flex-wrap gap-3">
        {#each styles as s}
          <button
            class="rounded-2xl overflow-hidden border-2 transition-all {draft.persona_style === s.id ? 'border-[var(--accent)] scale-105' : 'border-transparent opacity-70 hover:opacity-100'}"
            onclick={() => pickStyle(s.id)}
            title={s.label}
          >
            <img src={s.img} alt={s.label} class="w-20 h-20 object-cover" />
            <div class="text-[10px] py-1 bg-[var(--bg2)] text-[var(--muted)]">{s.label}</div>
          </button>
        {/each}
      </div>
    </section>

    <!-- autonomy -->
    <section class="card p-6">
      <h2 class="font-bold mb-4">{t("autonomy")}</h2>
      <div class="grid gap-3 text-sm">
        <label class="flex items-center gap-3">
          <input type="checkbox" bind:checked={draft.autonomy.heartbeat_enabled} class="accent-[var(--accent)] w-4 h-4" />
          {t("heartbeat")}
        </label>
        {#if draft.autonomy.heartbeat_enabled}
          <label class="text-xs text-[var(--muted)] ms-7">
            min
            <input type="number" class="input !w-20 !py-1 mx-2" min="5" max="240" bind:value={draft.autonomy.heartbeat_minutes} />
          </label>
        {/if}
        <label class="flex items-center gap-3">
          <input type="checkbox" bind:checked={draft.autonomy.close_to_tray} class="accent-[var(--accent)] w-4 h-4" />
          {t("close_to_tray")}
        </label>
        <label class="flex items-center gap-3">
          <input type="checkbox" bind:checked={draft.autonomy.autostart} class="accent-[var(--accent)] w-4 h-4" />
          {t("autostart")}
        </label>
        <label class="flex items-center gap-3">
          <input type="checkbox" bind:checked={draft.autonomy.notifications_enabled} class="accent-[var(--accent)] w-4 h-4" />
          {t("notifications")}
        </label>
        <label class="flex items-center gap-3">
          <input type="checkbox" bind:checked={draft.autonomy.auto_start_missions} class="accent-[var(--accent)] w-4 h-4" />
          {t("auto_start_missions")}
        </label>
        <label class="flex items-center gap-3">
          <input type="checkbox" bind:checked={draft.autonomy.run_commands} class="accent-[var(--accent)] w-4 h-4" />
          {t("run_commands")}
        </label>
        <label class="flex items-center gap-3">
          <input type="checkbox" bind:checked={draft.autonomy.allow_screenshots} class="accent-[var(--accent)] w-4 h-4" />
          {t("allow_screenshots")}
        </label>
      </div>
      <label class="block text-xs text-[var(--muted)] mt-4">
        {t("watched_folder")}
        <input class="input mt-1" bind:value={draft.watched_folder} placeholder="C:\Users\me\Vara\Inbox" />
      </label>
    </section>

    <!-- updates (over-the-air) -->
    <section class="card p-6">
      <h2 class="font-bold mb-4">{t("updates_title")}</h2>
      <div class="flex items-center gap-3 flex-wrap text-sm">
        <span class="chip">v{appVersion || "…"} {updater.available ? "→ v" + updater.available.version : ""}</span>
        {#if updater.available}
          <button class="btn text-xs" onclick={installUpdate} disabled={updater.installing}>
            {updater.installing ? t("update_installing") : t("update_install")}
          </button>
          {#if updater.installing && updater.progress}
            <span class="text-xs text-[var(--muted)]">
              {t("update_progress")}
              {#if updater.progress.total}
                {Math.round((updater.progress.downloaded / updater.progress.total) * 100)}%
              {/if}
            </span>
          {/if}
        {:else}
          <button class="btn-ghost text-xs" onclick={() => checkForUpdate(false)} disabled={updater.checking}>
            {updater.checking ? t("update_checking") : t("update_check")}
          </button>
          {#if !updater.checking && appVersion}
            <span class="text-xs text-[var(--ok)]">{t("update_latest")}</span>
          {/if}
        {/if}
      </div>
      <p class="text-xs text-[var(--muted)] mt-3">{t("update_auto_note")}</p>
    </section>

    <!-- language + save -->
    <section class="card p-6 flex items-center gap-4">
      <button class="btn-ghost text-xs" onclick={() => setLang(i18n.lang === "ar" ? "en" : "ar")}>
        {t("language")}: {i18n.lang === "ar" ? "العربية" : "English"}
      </button>
      <button class="btn ms-auto" onclick={save} disabled={saving}>{saved ? "✓ " + t("saved") : t("save")}</button>
    </section>

    {#if error}
      <div class="text-xs text-[var(--bad)]">{error}</div>
    {/if}

    <div class="text-[10px] text-[var(--muted)] px-2">
      {t("data_dir")}: {app.dataDir}
    </div>
  </div>
{/if}
