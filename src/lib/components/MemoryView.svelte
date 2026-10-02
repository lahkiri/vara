<script lang="ts">
  import { t } from "../i18n.svelte";
  import { api, type Note } from "../api";

  let query = $state("");
  let notes = $state<Note[]>([]);
  let loading = $state(true);
  let adding = $state(false);
  let newTitle = $state("");
  let newBody = $state("");
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  /// The bundled rules documents (read-only) — see `list_skills` in the shell.
  let skills = $state<{ name: string; description: string; body: string }[]>([]);

  const kindT: Record<string, () => string> = {
    research: () => t("kind_research"),
    file_observation: () => t("kind_file"),
    reflection: () => t("kind_reflection"),
    manual: () => t("kind_manual"),
  };

  async function load() {
    loading = true;
    notes = await api.notes(query.trim() || null, 120).catch(() => []);
    loading = false;
  }

  function onInput() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(load, 250);
  }

  async function addNote() {
    if (!newTitle.trim() && !newBody.trim()) return;
    await api.addNote(newTitle.trim(), newBody.trim()).catch(() => {});
    newTitle = "";
    newBody = "";
    adding = false;
    await load();
  }

  async function remove(id: number) {
    await api.deleteNote(id).catch(() => {});
    await load();
  }

  load();
  api.skills().then((s) => (skills = s)).catch(() => (skills = []));
</script>

<div class="flex flex-col gap-4">
  <div class="flex items-center gap-3">
    <input class="input" placeholder={t("memory_search")} bind:value={query} oninput={onInput} />
    <button class="btn-ghost text-xs shrink-0" onclick={() => (adding = !adding)}>+ {t("add_note")}</button>
  </div>

  {#if adding}
    <div class="card p-5 flex flex-col gap-3 fade-up">
      <input class="input" placeholder={t("note_title")} bind:value={newTitle} />
      <textarea class="input min-h-20" placeholder={t("note_body")} bind:value={newBody}></textarea>
      <div class="flex gap-2 justify-end">
        <button class="btn-ghost text-xs" onclick={() => (adding = false)}>{t("close")}</button>
        <button class="btn text-xs" onclick={addNote}>{t("add_note")}</button>
      </div>
    </div>
  {/if}

  {#if loading}
    <div class="card p-6 text-[var(--muted)]">…</div>
  {:else if notes.length === 0}
    <div class="card p-10 text-center text-[var(--muted)]">{t("empty_memory")}</div>
  {:else}
    <div class="grid md:grid-cols-2 gap-3">
      {#each notes as n (n.id)}
        <div class="card p-4 flex flex-col gap-2 fade-up">
          <div class="flex items-center gap-2">
            <span class="chip">{kindT[n.kind]?.() ?? n.kind}</span>
            {#if n.mission_id}
              <span class="chip">#{n.mission_id}</span>
            {/if}
            <button class="ms-auto text-[var(--muted)] hover:text-[var(--bad)] text-xs" onclick={() => remove(n.id)} title={t("delete")}>
              ✕
            </button>
          </div>
          <div class="font-semibold text-sm">{n.title}</div>
          <div class="text-xs text-[var(--muted)] leading-relaxed line-clamp-4">{n.body}</div>
          {#if n.source_url}
            <button
              class="text-xs text-[var(--accent)] truncate text-start"
              onclick={() => api.sysOpen(n.source_url!).catch(() => {})}
              title={n.source_url}
            >
              {t("source")}: {n.source_title || n.source_url}
            </button>
          {/if}
          <div class="text-[10px] text-[var(--muted)]">{n.created_at}</div>
        </div>
      {/each}
    </div>
  {/if}

  <!-- The entity's own rules, shown from the bundled skills documents. Read-only
       on purpose: this is what the owner can hold Vara to, not a plugin surface. -->
  {#if skills.length > 0}
    <div class="mt-2">
      <div class="text-[11px] font-bold text-[var(--muted)] uppercase tracking-wide mb-2">{t("skills_title")}</div>
      <div class="grid md:grid-cols-2 gap-3">
        {#each skills as s (s.name)}
          <details class="card p-4 fade-up">
            <summary class="cursor-pointer text-sm font-semibold">{s.name}</summary>
            {#if s.description}
              <div class="text-xs text-[var(--muted)] mt-1">{s.description}</div>
            {/if}
            <pre class="action-output mt-2">{s.body}</pre>
          </details>
        {/each}
      </div>
    </div>
  {/if}
</div>
