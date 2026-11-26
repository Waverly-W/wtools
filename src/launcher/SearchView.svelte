<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { api, type Item } from "../lib/api";
  import { setHeight } from "./win";

  let { showSeq }: { showSeq: number } = $props();

  const INPUT_HEIGHT = 56;
  const ITEM_HEIGHT = 48;
  const MAX_VISIBLE = 12;
  const LIST_PADDING = 4; // 与 .list 的内边距 --sp-1 一致

  let query = $state("");
  let items = $state<Item[]>([]);
  let selected = $state(0);
  let error = $state("");
  let input: HTMLInputElement;
  let list = $state<HTMLUListElement>();
  let seq = 0;

  // 输入变化时搜索；用序号丢弃过期结果
  $effect(() => {
    const q = query;
    const mySeq = ++seq;
    error = "";
    if (!q.trim()) {
      items = [];
      return;
    }
    api.search(q).then((res) => {
      if (mySeq !== seq) return;
      items = res;
      selected = 0;
    });
  });

  // 结果数量变化时调整窗口高度
  $effect(() => {
    const n = Math.min(items.length, MAX_VISIBLE);
    const extra = error ? 36 : 0;
    setHeight(INPUT_HEIGHT + (n > 0 ? n * ITEM_HEIGHT + LIST_PADDING * 2 + 1 : 0) + extra);
  });

  // 每次呼出时聚焦并全选输入框
  $effect(() => {
    void showSeq;
    input?.focus();
    input?.select();
  });

  function reset() {
    query = "";
    items = [];
    selected = 0;
    error = "";
  }

  async function run(index: number) {
    const item = items[index];
    if (!item) return;
    try {
      await api.execute(item.id);
      reset();
    } catch (e) {
      error = String(e);
    }
  }

  async function select(index: number) {
    selected = index;
    await tick();
    list?.children[index]?.scrollIntoView({ block: "nearest" });
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.isComposing) return; // 中文输入法选词中，不拦截
    const n = items.length;
    if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
      e.preventDefault();
      if (n) select((selected + 1) % n);
    } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
      e.preventDefault();
      if (n) select((selected - 1 + n) % n);
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(selected);
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (query) reset();
      else api.hideLauncher();
    } else if (e.metaKey && /^[1-9]$/.test(e.key)) {
      e.preventDefault();
      run(Number(e.key) - 1);
    } else if (e.metaKey && e.key === ",") {
      e.preventDefault();
      api.execute("wtools:settings");
    }
  }

  onMount(() => {
    const un = listen("launcher:hide", reset);
    return () => {
      un.then((f) => f());
    };
  });
</script>

<div class="view">
  <div class="search">
    <svg class="search-icon" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="10.5" cy="10.5" r="6.5" />
      <path d="M15.5 15.5 21 21" />
    </svg>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      bind:this={input}
      bind:value={query}
      onkeydown={onKeydown}
      placeholder="搜索应用和命令…"
      spellcheck="false"
      autocomplete="off"
      autofocus
    />
  </div>

  {#if items.length}
    <ul class="list" bind:this={list} role="listbox">
      {#each items as item, i (item.id)}
        <!-- 键盘操作统一由输入框处理，列表项只需响应鼠标 -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          class="list-row"
          role="option"
          aria-selected={i === selected}
          class:selected={i === selected}
          onmousemove={() => (selected = i)}
          onclick={() => run(i)}
        >
          <span class="icon">
            {#if item.icon.includes("://")}
              <img src={item.icon} alt="" width="32" height="32" draggable="false" />
            {:else}
              {item.icon}
            {/if}
          </span>
          <span class="text">
            <span class="title">{item.title}</span>
            {#if item.subtitle}<span class="subtitle">{item.subtitle}</span>{/if}
          </span>
          {#if i < 9}<kbd>⌘{i + 1}</kbd>{/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if error}
    <p class="error">{error}</p>
  {/if}
</div>

<style>
  .view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .search {
    height: var(--h-search);
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: 0 var(--sp-4);
  }

  .search-icon {
    width: 20px;
    height: 20px;
    flex: none;
    fill: none;
    stroke: var(--text-secondary);
    stroke-width: 2;
    stroke-linecap: round;
  }

  input {
    flex: 1;
    height: 100%;
    border: none;
    outline: none;
    background: transparent;
    font-size: var(--fs-xl);
    color: var(--text);
  }

  input::placeholder {
    color: var(--text-tertiary);
  }

  .list {
    flex: 1;
    border-top: 1px solid var(--border);
  }

  .list-row {
    height: 48px;
  }

  .icon {
    width: 32px;
    height: 32px;
    font-size: var(--fs-xl);
    line-height: 32px;
    text-align: center;
    flex: none;
  }

  .icon img {
    display: block;
    width: 32px;
    height: 32px;
  }

  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: var(--sp-3);
  }

  .title {
    font-size: var(--fs-lg);
    white-space: nowrap;
  }

  .subtitle {
    font-size: var(--fs-sm);
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .error {
    height: 36px;
    flex: none;
    line-height: 36px;
    padding: 0 var(--sp-4);
    font-size: var(--fs-md);
    color: var(--danger);
    border-top: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
