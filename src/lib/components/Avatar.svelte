<script lang="ts">
  // Maps the entity state to one of Vara's 8 character states / 5 styles,
  // exactly like the brand sheet: Different moments. Same mission.
  import happy from "../../assets/characters/happy.png";
  import focused from "../../assets/characters/focused.png";
  import thinking from "../../assets/characters/thinking.png";
  import excited from "../../assets/characters/excited.png";
  import serious from "../../assets/characters/serious.png";
  import working from "../../assets/characters/working.png";
  import planning from "../../assets/characters/planning.png";
  import mission from "../../assets/characters/mission.png";
  import classic from "../../assets/characters/classic.png";
  import dark from "../../assets/characters/dark.png";
  import stealth from "../../assets/characters/stealth.png";
  import tech from "../../assets/characters/tech.png";
  import nature from "../../assets/characters/nature.png";

  const byState: Record<string, string> = {
    deliberating: thinking,
    working: working,
    reporting: planning,
    paused: serious,
    dormant: focused,
    sleeping: dark,
    success: excited,
  };
  const byStyle: Record<string, string> = {
    classic, dark, stealth, tech, nature,
  };

  let {
    state = "attentive",
    style = "classic",
    size = 96,
  }: { state?: string; style?: string; size?: number } = $props();

  const src = $derived(byState[state] ?? byStyle[style] ?? classic);
</script>

<div
  class="rounded-2xl overflow-hidden shrink-0 {state === 'working' || state === 'deliberating' || state === 'reporting' ? 'pulse' : ''}"
  style={"width:" + size + "px;height:" + size + "px;border:1px solid var(--line);background:var(--bg2)"}
>
  <img src={src} alt={"Vara " + state} class="w-full h-full object-cover" />
</div>
