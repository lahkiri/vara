<script lang="ts">
  import { app, updater } from "../state.svelte";
  import { t, i18n, setLang } from "../i18n.svelte";
  import face from "../../assets/characters/face.png";

  const nav = [
    { id: "chat", label: () => t("nav_chat"), icon: "✷" },
    { id: "reports", label: () => t("nav_reports"), icon: "▤" },
    { id: "memory", label: () => t("nav_memory"), icon: "❖" },
    { id: "activity", label: () => t("nav_activity"), icon: "≡" },
    { id: "settings", label: () => t("nav_settings"), icon: "⚙" },
  ];

  let busyDot = $derived(app.busy ? "var(--accent)" : app.paused ? "var(--warn)" : "var(--ok)");
</script>

<aside class="w-56 shrink-0 h-full border-e border-[var(--line)] bg-[var(--bg2)]/60 flex flex-col">
  <div class="px-5 pt-5 pb-4 flex items-center gap-3">
    <img src={face} alt="Vara" class="w-11 h-11 rounded-xl avatar-ring" />
    <div>
      <div class="font-extrabold text-lg leading-none">Vara</div>
      <div class="text-[11px] text-[var(--muted)] mt-1">{t("app_motto")}</div>
    </div>
  </div>

  <nav class="px-3 mt-2 flex flex-col gap-1">
    {#each nav as item (item.id)}
      <button
        class="text-start px-3 py-2 rounded-xl text-sm flex items-center gap-3 transition-colors
          {app.view === item.id ? 'nav-active' : 'text-[var(--muted)] hover:bg-[var(--card-hover)]'}"
        onclick={() => (app.view = item.id)}
      >
        <span class="opacity-80">{item.icon}</span>
        {item.label()}
      </button>
    {/each}
  </nav>

  <div class="mt-auto px-4 pb-4 flex flex-col gap-2">
    <button
      class="btn-ghost text-xs flex items-center justify-between"
      onclick={() => setLang(i18n.lang === "ar" ? "en" : "ar")}
      title="AR / EN"
    >
      <span>{t("language")}</span>
      <span class="font-bold">{i18n.lang === "ar" ? "عربي" : "EN"}</span>
    </button>
    {#if updater.available}
      <button
        class="card px-3 py-2 flex items-center gap-2 text-xs border-[var(--accent)]/50 text-[var(--accent)] w-full"
        onclick={() => (app.view = "settings")}
        title={t("update_available") + " — v" + updater.available.version}
      >
        <span class="w-2 h-2 rounded-full bg-[var(--accent)] pulse inline-block"></span>
        <span class="font-bold">{t("update_available")} · v{updater.available.version}</span>
      </button>
    {/if}
    <div class="card px-3 py-2 flex items-center gap-2 text-xs">
      <span class="w-2 h-2 rounded-full" style={"background:" + busyDot} class:pulse={app.busy}></span>
      <span class="text-[var(--muted)]">
        {app.busy ? t("vara_is_working") : app.paused ? t("vara_paused") : t("vara_idle")}
      </span>
    </div>
  </div>
</aside>

<style>
  /* silent a11y focus */
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
