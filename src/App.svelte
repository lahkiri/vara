<script lang="ts">
  import { onMount } from "svelte";
  import { app, boot, initEvents } from "./lib/state.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import ChatView from "./lib/components/ChatView.svelte";
  import MemoryView from "./lib/components/MemoryView.svelte";
  import ReportsView from "./lib/components/ReportsView.svelte";
  import ActivityView from "./lib/components/ActivityView.svelte";
  import SettingsView from "./lib/components/SettingsView.svelte";
  import orb from "./assets/characters/orb.png";

  onMount(async () => {
    await initEvents();
    await boot();
  });
</script>

{#if !app.ready}
  <div class="h-screen w-screen flex flex-col items-center justify-center gap-4 bg-[var(--bg)]">
    <img src={orb} alt="Vara" class="w-20 h-20 rounded-2xl pulse" />
    <div class="text-[var(--muted)] text-sm tracking-wide">Vara…</div>
  </div>
{:else}
  <div class="flex h-screen overflow-hidden">
    <Sidebar />
    <main class="flex-1 overflow-hidden">
      {#if app.view === "chat"}
        <ChatView />
      {:else}
        <div class="h-full overflow-y-auto">
          <div class="max-w-5xl mx-auto px-6 py-6">
            {#if app.view === "memory"}
              <MemoryView />
            {:else if app.view === "reports"}
              <ReportsView />
            {:else if app.view === "activity"}
              <ActivityView />
            {:else if app.view === "settings"}
              <SettingsView />
            {/if}
          </div>
        </div>
      {/if}
    </main>
  </div>
{/if}
