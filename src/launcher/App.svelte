<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import ClipboardView from "./ClipboardView.svelte";
  import SearchView from "./SearchView.svelte";

  type View = "search" | "clipboard";

  let view = $state<View>("search");
  // 每次呼出递增，视图据此重新聚焦输入框
  let showSeq = $state(0);

  function switchTo(next: View) {
    view = next;
    showSeq++;
    api.setLauncherView(next);
  }

  onMount(() => {
    const un = listen<string>("launcher:show", (e) => {
      view = e.payload === "clipboard" ? "clipboard" : "search";
      showSeq++;
    });
    return () => {
      un.then((f) => f());
    };
  });
</script>

<main>
  {#if view === "clipboard"}
    <ClipboardView {showSeq} onBack={() => switchTo("search")} />
  {:else}
    <SearchView {showSeq} />
  {/if}
</main>

<style>
  :global(html),
  :global(body) {
    background: transparent;
    overflow: hidden;
  }

  main {
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--floating);
    border-radius: var(--r-lg);
    overflow: hidden;
  }
</style>
