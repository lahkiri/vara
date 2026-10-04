<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { t } from "../i18n.svelte";
  import { isTauri } from "../api";

  // A custom titlebar only makes sense inside the desktop shell; in a plain
  // browser (the mock) it would just be three dead buttons.
  const win = isTauri() ? getCurrentWindow() : null;

  function minimize() {
    void win?.minimize();
  }
  function toggleMaximize() {
    void win?.toggleMaximize();
  }
  function close() {
    void win?.close();
  }
</script>

<!--
  Custom dark titlebar (Tauri `decorations: false`). The native chrome is white
  on this platform and clashed with the dark surface (gap G-07): the first thing
  anyone saw was a light band above a near-black app. The drag region is an
  attribute Tauri recognises, so no JS is needed to move the window.
-->
{#if win}
  <div class="titlebar" data-tauri-drag-region>
    <div class="flex items-center gap-2 ps-3 pointer-events-none">
      <span class="titlebar-dot"></span>
      <span class="titlebar-title">{t("app_name")}</span>
    </div>
    <div class="titlebar-controls">
      <button class="titlebar-btn" onclick={minimize} title={t("win_minimize")} aria-label={t("win_minimize")}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" stroke-width="1.2" /></svg>
      </button>
      <button class="titlebar-btn" onclick={toggleMaximize} title={t("win_maximize")} aria-label={t("win_maximize")}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="0.6" y="0.6" width="8.8" height="8.8" fill="none" stroke="currentColor" stroke-width="1.2" /></svg>
      </button>
      <button class="titlebar-btn titlebar-close" onclick={close} title={t("win_close")} aria-label={t("win_close")}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0.5 0.5l9 9M9.5 0.5l-9 9" stroke="currentColor" stroke-width="1.2" /></svg>
      </button>
    </div>
  </div>
{/if}
