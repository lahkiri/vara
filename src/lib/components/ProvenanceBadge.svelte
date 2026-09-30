<script lang="ts">
  import { t } from "../i18n.svelte";

  let {
    verdict = null,
    ratio = null,
    cited = 0,
  }: { verdict?: string | null; ratio?: number | null; cited?: number } = $props();

  const vacuous = $derived(cited === 0);
  const cls = $derived(
    verdict === "PASS" && !vacuous
      ? "border-[var(--ok)] text-[var(--ok)]"
      : verdict === "FAIL"
        ? "border-[var(--bad)] text-[var(--bad)]"
        : "border-[var(--warn)] text-[var(--warn)]"
  );
  const label = $derived(
    verdict === "FAIL"
      ? t("provenance_fail")
      : vacuous
        ? t("provenance_vacuous")
        : t("provenance_pass")
  );
  const pct = $derived(ratio === null ? null : Math.round(ratio * 100) + "%");
</script>

<span class={"chip " + cls} title={t("backed_ratio")}>
  <span class="font-bold">{label}</span>
  {#if pct}
    <span class="opacity-80">· {pct}</span>
  {/if}
</span>
