<script lang="ts">
  import { onMount, tick, type Component } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { api, type JsonMode } from "../lib/api";
  import { smartParse } from "./core";
  import { store } from "./editor";
  import FormatTab from "./FormatTab.svelte";

  type TabApi = { load: (text: string, explicit: boolean, right?: string | null) => void; focus: () => void };
  type TabProps = { notify: (msg: string, error?: boolean) => void };

  let mode = $state<JsonMode>(store.get("mode") === "diff" ? "diff" : "format");
  let autoClip = $state(store.get("autoClip", "1") === "1");
  // 对比页按需加载（含 diff 算法），格式化页打开更快
  let DiffTab = $state<Component<TabProps, TabApi> | null>(null);
  let formatTab = $state<TabApi>();
  let diffTab = $state<TabApi>();
  let toast = $state<{ msg: string; error: boolean } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => store.set("mode", mode));
  $effect(() => store.set("autoClip", autoClip ? "1" : "0"));

  function notify(msg: string, error = false) {
    toast = { msg, error };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), error ? 4000 : 2200);
  }

  async function switchTo(m: JsonMode) {
    mode = m;
    if (m === "diff" && !DiffTab) DiffTab = (await import("./DiffTab.svelte")).default as unknown as Component<TabProps, TabApi>;
    await tick();
    (m === "diff" ? diffTab : formatTab)?.focus();
  }

  /** 窗口打开 / 再次唤起时：切到指定页签，载入传入的内容或剪贴板里的 JSON */
  async function loadPending() {
    const p = await api.jsonTakePending().catch(() => null);
    if (p) await switchTo(p.mode);
    else if (mode === "diff") await switchTo("diff");
    let text = p?.text ?? null;
    const explicit = text != null;
    if (!explicit && autoClip) {
      const clip = await api.jsonClipboardText().catch(() => null);
      if (clip && clip.length < 10_000_000 && smartParse(clip).ok) text = clip;
    }
    if (text == null) return;
    (mode === "diff" ? diffTab : formatTab)?.load(text, explicit, p?.right);
  }

  function onKey(e: KeyboardEvent) {
    if (!e.metaKey || e.shiftKey || e.altKey) return;
    if (e.key === "w") {
      e.preventDefault();
      getCurrentWindow().close();
    } else if (e.key === "1" || e.key === "2") {
      e.preventDefault();
      switchTo(e.key === "1" ? "format" : "diff");
    }
  }

  onMount(() => {
    loadPending();
    const un = listen("json:open", () => loadPending());
    window.addEventListener("keydown", onKey);
    return () => {
      un.then((f) => f());
      window.removeEventListener("keydown", onKey);
    };
  });
</script>

<div class="app">
  <header class="bar">
    <div class="seg">
      <button class:on={mode === "format"} onclick={() => switchTo("format")} title="⌘1">格式化 / 转义</button>
      <button class:on={mode === "diff"} onclick={() => switchTo("diff")} title="⌘2">对比</button>
    </div>
    <span class="spacer"></span>
    <label class="check" title="打开窗口时，如果剪贴板里是 JSON 就自动载入">
      <input type="checkbox" bind:checked={autoClip} />打开时读取剪贴板
    </label>
  </header>

  <main>
    <div class="pane" class:hidden={mode !== "format"}>
      <FormatTab bind:this={formatTab} {notify} />
    </div>
    {#if DiffTab}
      <div class="pane" class:hidden={mode !== "diff"}>
        <DiffTab bind:this={diffTab} {notify} />
      </div>
    {/if}
  </main>

  {#if toast}
    <div class="toast" class:error={toast.error}>{toast.msg}</div>
  {/if}
</div>
