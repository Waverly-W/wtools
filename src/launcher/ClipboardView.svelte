<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick, untrack } from "svelte";
  import { api, type ClipFilter, type ClipItem, type CustomAction } from "../lib/api";
  import { STAT_LABELS, textStats } from "../lib/textStats";
  import OcrPanel from "../lib/OcrPanel.svelte";
  import { setHeight } from "./win";

  let { showSeq, onBack }: { showSeq: number; onBack: () => void } = $props();

  const HEIGHT = 780;
  const SINGLE_FILTERS: [ClipFilter, string][] = [
    ["all", "全部"],
    ["text", "文本"],
    ["image", "图片"],
    ["files", "文件"],
    ["favorite", "★ 收藏"],
  ];
  const HISTORY_FILTERS = SINGLE_FILTERS.slice(0, 4);

  type Layout = "single" | "dual";
  interface Col {
    key: "single" | "history" | "favorite";
    filter: ClipFilter;
    tag: string | null;
    items: ClipItem[];
    cursor: number;
  }
  interface MenuItem {
    id: string;
    name: string;
    hint?: string;
    danger?: boolean;
    sep?: boolean;
    run: () => void;
  }
  type Sheet = null | { type: "stats"; text: string } | { type: "output"; name: string; text: string };

  let layout = $state<Layout>("dual");
  let maxLines = $state(4);
  let cols = $state<Col[]>([]);
  let active = $state(0);
  let query = $state("");
  let tags = $state<string[]>([]);
  /** 多选：按选择顺序记录 id */
  let picked = $state<number[]>([]);
  let expanded = $state<Set<number>>(new Set());
  /** 被行数截断、需要显示「展开」的记录 */
  let clipped = $state<Set<number>>(new Set());
  let fullText = $state<Record<number, string>>({});
  let customActions = $state<CustomAction[]>([]);
  let message = $state("");
  let ocrId = $state<number | null>(null);
  let sheet = $state<Sheet>(null);
  let menu = $state<{ x: number; y: number } | null>(null);
  let menuIndex = $state(0);
  let tagEdit = $state<{ id: number; draft: string } | null>(null);
  let input: HTMLInputElement;
  let tagInput = $state<HTMLInputElement>();
  const lists: (HTMLUListElement | undefined)[] = $state([]);
  const loadSeq: number[] = [];

  const col = $derived(cols[active]);
  const current = $derived(col?.items[col.cursor]);
  const allItems = $derived(cols.flatMap((c) => c.items));
  const pickedItems = $derived(
    picked.map((id) => allItems.find((i) => i.id === id)).filter((i): i is ClipItem => !!i),
  );
  /** 当前要操作的记录：多选时为所选（按选择顺序），否则为光标所在项 */
  const targets = $derived(picked.length ? pickedItems : current ? [current] : []);

  function makeCols(l: Layout): Col[] {
    return l === "dual"
      ? [
          { key: "history", filter: "all", tag: null, items: [], cursor: 0 },
          { key: "favorite", filter: "favorite", tag: null, items: [], cursor: 0 },
        ]
      : [{ key: "single", filter: "all", tag: null, items: [], cursor: 0 }];
  }

  async function loadCol(i: number, keep = false) {
    const c = cols[i];
    if (!c) return;
    const mySeq = (loadSeq[i] = (loadSeq[i] ?? 0) + 1);
    const res = await api.clipList(query, c.filter, c.filter === "favorite" ? c.tag : null);
    if (mySeq !== loadSeq[i] || !cols[i]) return;
    const prevId = c.items[c.cursor]?.id;
    const idx = keep ? res.findIndex((it) => it.id === prevId) : -1;
    cols[i].items = res;
    cols[i].cursor = idx >= 0 ? idx : 0;
  }

  async function loadAll(keep = false) {
    await Promise.all(cols.map((_, i) => loadCol(i, keep)));
    const ids = new Set(allItems.map((i) => i.id));
    picked = picked.filter((id) => ids.has(id));
  }

  async function loadTags() {
    tags = await api.clipTags();
    for (const c of cols) if (c.tag && !tags.includes(c.tag)) c.tag = null;
  }

  function setLayout(l: Layout) {
    layout = l;
    cols = makeCols(l);
    active = 0;
    picked = [];
    loadAll();
  }

  // 搜索词变化：所有栏一起筛选
  $effect(() => {
    void query;
    untrack(() => {
      message = "";
      picked = [];
      loadAll();
    });
  });

  // 每次进入视图：按设置初始化布局
  $effect(() => {
    void showSeq;
    untrack(async () => {
      setHeight(HEIGHT);
      query = "";
      message = "";
      ocrId = null;
      sheet = null;
      menu = null;
      tagEdit = null;
      picked = [];
      expanded = new Set();
      tick().then(() => input?.focus());
      const s = await api.getClipboardSettings();
      customActions = s.actions;
      maxLines = s.max_lines;
      setLayout(s.layout);
      loadTags();
    });
  });

  // ---------- 选择 ----------

  async function moveCursor(index: number, extend: boolean) {
    const c = col;
    if (!c || !c.items.length) return;
    index = Math.max(0, Math.min(index, c.items.length - 1));
    if (extend && current) {
      const base = picked.length ? picked : [current.id];
      const target = c.items[index];
      picked = target && !base.includes(target.id) ? [...base, target.id] : base;
    } else if (!extend) {
      picked = [];
    }
    c.cursor = index;
    await tick();
    lists[active]?.children[index]?.scrollIntoView({ block: "nearest" });
  }

  function onItemClick(e: MouseEvent, ci: number, index: number) {
    const c = cols[ci];
    const item = c.items[index];
    const prev = current;
    active = ci;
    if (e.metaKey) {
      const base = picked.length ? picked : prev ? [prev.id] : [];
      picked = base.includes(item.id) ? base.filter((id) => id !== item.id) : [...base, item.id];
    } else if (e.shiftKey && prev && cols[ci].items.includes(prev)) {
      const a = c.cursor;
      const range = (a <= index ? c.items.slice(a, index + 1) : c.items.slice(index, a + 1).reverse()).map((i) => i.id);
      picked = [...new Set([...picked.filter((id) => !range.includes(id)), ...range])];
    } else {
      picked = [];
    }
    c.cursor = index;
  }

  function switchColumn(dir: number) {
    if (cols.length < 2) return;
    active = (active + dir + cols.length) % cols.length;
  }

  function cycleFilter(dir: number) {
    const c = cols[0];
    const i = SINGLE_FILTERS.findIndex(([f]) => f === c.filter);
    c.filter = SINGLE_FILTERS[(i + dir + SINGLE_FILTERS.length) % SINGLE_FILTERS.length][0];
    c.tag = null;
    loadCol(0);
  }

  function setFilter(ci: number, f: ClipFilter) {
    cols[ci].filter = f;
    if (f !== "favorite") cols[ci].tag = null;
    active = ci;
    loadCol(ci);
  }

  function setTag(ci: number, t: string | null) {
    cols[ci].tag = t;
    active = ci;
    loadCol(ci);
  }

  // ---------- 展开 ----------

  async function toggleExpand(item: ClipItem) {
    const next = new Set(expanded);
    if (next.has(item.id)) {
      next.delete(item.id);
    } else {
      next.add(item.id);
      if (item.kind === "text" && item.more && fullText[item.id] === undefined) {
        const d = await api.clipDetail(item.id);
        fullText = { ...fullText, [item.id]: d.text ?? "" };
      }
    }
    expanded = next;
  }

  /** 检测内容是否被行数截断（截断才显示「展开」） */
  function watchClip(node: HTMLElement, id: number) {
    const check = () => {
      const over = node.scrollHeight > node.clientHeight + 1;
      if (over !== clipped.has(id)) {
        const next = new Set(clipped);
        if (over) next.add(id);
        else next.delete(id);
        clipped = next;
      }
    };
    const ro = new ResizeObserver(check);
    ro.observe(node);
    check();
    return { destroy: () => ro.disconnect() };
  }

  // ---------- 操作 ----------

  async function deliver(paste: boolean, list = targets) {
    if (!list.length) return;
    try {
      await api.clipPaste(
        list.map((i) => i.id),
        paste,
      );
    } catch (e) {
      message = String(e);
    }
  }

  async function toggleFavorite() {
    if (!targets.length) return;
    const fav = !targets.every((t) => t.favorite);
    await api.clipFavorite(
      targets.map((t) => t.id),
      fav,
    );
    await loadAll(true);
    await loadTags();
  }

  async function remove() {
    if (!targets.length) return;
    await api.clipDelete(targets.map((t) => t.id));
    picked = [];
    await loadAll(true);
    await loadTags();
  }

  async function textOf(item: ClipItem): Promise<string> {
    if (!item.more) return item.preview;
    if (fullText[item.id] === undefined) {
      const d = await api.clipDetail(item.id);
      fullText = { ...fullText, [item.id]: d.text ?? "" };
    }
    return fullText[item.id];
  }

  async function runCustom(a: CustomAction, item: ClipItem) {
    message = "";
    try {
      const r = await api.clipRunAction(a.id, item.id);
      if (r.output !== null) sheet = { type: "output", name: a.name, text: r.output };
      else if (r.message) message = r.message;
    } catch (e) {
      message = String(e);
    }
  }

  async function addTag() {
    if (!tagEdit) return;
    const t = tagEdit.draft.trim();
    const item = allItems.find((i) => i.id === tagEdit!.id);
    if (!t || !item) return;
    await api.clipSetTags(item.id, [...item.tags.filter((x) => x !== t), t]);
    tagEdit.draft = "";
    await loadAll(true);
    await loadTags();
  }

  async function removeTag(t: string) {
    const item = tagEdit && allItems.find((i) => i.id === tagEdit!.id);
    if (!item) return;
    await api.clipSetTags(
      item.id,
      item.tags.filter((x) => x !== t),
    );
    await loadAll(true);
    await loadTags();
  }

  const tagEditItem = $derived(tagEdit ? allItems.find((i) => i.id === tagEdit!.id) : undefined);

  // ---------- 右键菜单 ----------

  /** 文本以 { [ 开头（或是被转义成字符串的 JSON）时提供 JSON 相关操作 */
  const looksLikeJson = (c: ClipItem) => c.kind === "text" && /^\s*"?[{[]/.test(c.preview);

  const menuItems = $derived.by<MenuItem[]>(() => {
    const c = current;
    if (!c) return [];
    const multi = picked.length > 1;
    const items: MenuItem[] = [
      { id: "paste", name: multi ? `按顺序粘贴 ${picked.length} 条` : "粘贴", hint: "↩", run: () => deliver(true) },
      { id: "copy", name: multi ? `复制 ${picked.length} 条` : "仅复制", hint: "⌘C", run: () => deliver(false) },
    ];
    if (!multi && (c.kind === "image" || clipped.has(c.id) || c.more)) {
      items.push({ id: "expand", name: expanded.has(c.id) ? "收起" : "展开", hint: "⌘E", run: () => toggleExpand(c) });
    }
    const allFav = targets.every((t) => t.favorite);
    items.push({ id: "fav", name: allFav ? "取消收藏" : "收藏", hint: "⌘S", sep: true, run: toggleFavorite });
    if (!multi) {
      items.push({ id: "tags", name: "编辑标签…", run: () => (tagEdit = { id: c.id, draft: "" }) });
      const typed: MenuItem[] = [];
      if (c.kind === "text")
        typed.push({ id: "stats", name: "统计字数", run: async () => (sheet = { type: "stats", text: await textOf(c) }) });
      if (looksLikeJson(c))
        typed.push({ id: "json", name: "JSON 格式化", run: async () => api.jsonOpen("format", await textOf(c)) });
      if (c.kind === "image")
        typed.push({ id: "ocr", name: c.has_ocr ? "查看识别文字" : "识别文字（OCR）", run: () => (ocrId = c.id) });
      if (c.kind !== "text") typed.push({ id: "reveal", name: "在访达中显示", run: () => api.clipReveal(c.id) });
      for (const a of customActions.filter((a) => a.applies_to.includes(c.kind))) {
        typed.push({ id: `custom:${a.id}`, name: a.name, run: () => runCustom(a, c) });
      }
      if (typed.length) typed[0].sep = true;
      items.push(...typed);
    }
    if (picked.length === 2 && targets.every(looksLikeJson)) {
      const [x, y] = targets;
      items.push({
        id: "json-diff",
        name: "对比这 2 条 JSON",
        sep: true,
        run: async () => api.jsonOpen("diff", await textOf(x), await textOf(y)),
      });
    }
    items.push({ id: "delete", name: multi ? `删除 ${picked.length} 条` : "删除", hint: "⌘⌫", danger: true, sep: true, run: remove });
    return items;
  });

  function openMenu(x: number, y: number) {
    // 估算菜单尺寸，避免超出窗口
    const h = menuItems.length * 30 + 12;
    menu = { x: Math.min(x, window.innerWidth - 236), y: Math.max(8, Math.min(y, window.innerHeight - h - 8)) };
    menuIndex = 0;
  }

  function onContextMenu(e: MouseEvent, ci: number, index: number) {
    e.preventDefault();
    const item = cols[ci].items[index];
    active = ci;
    cols[ci].cursor = index;
    if (!picked.includes(item.id)) picked = [];
    openMenu(e.clientX, e.clientY);
  }

  function openMenuAtCursor() {
    const el = lists[active]?.children[col?.cursor ?? 0] as HTMLElement | undefined;
    if (!el || !current) return;
    const r = el.getBoundingClientRect();
    openMenu(r.left + 48, r.top + Math.min(r.height, 40));
  }

  function runMenu(i: number) {
    const it = menuItems[i];
    menu = null;
    it?.run();
  }

  // ---------- 键盘 ----------

  function hasTextSelection(): boolean {
    if (document.activeElement === input && input.selectionStart !== input.selectionEnd) return true;
    const sel = window.getSelection();
    return !!sel && !sel.isCollapsed && sel.toString().length > 0;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.isComposing || ocrId !== null) return;
    const k = e.key;
    if (tagEdit) {
      if (k === "Escape") {
        e.preventDefault();
        tagEdit = null;
        input.focus();
      }
      return;
    }
    if (menu) {
      e.preventDefault();
      if (k === "ArrowDown") menuIndex = (menuIndex + 1) % menuItems.length;
      else if (k === "ArrowUp") menuIndex = (menuIndex - 1 + menuItems.length) % menuItems.length;
      else if (k === "Enter") runMenu(menuIndex);
      else if (k === "Escape" || (e.metaKey && k.toLowerCase() === "k")) menu = null;
      return;
    }
    if (sheet) {
      if (k === "Escape") {
        e.preventDefault();
        sheet = null;
      }
      return;
    }

    if (k === "ArrowDown" || (e.ctrlKey && k === "n")) {
      e.preventDefault();
      moveCursor((col?.cursor ?? 0) + 1, e.shiftKey);
    } else if (k === "ArrowUp" || (e.ctrlKey && k === "p")) {
      e.preventDefault();
      moveCursor((col?.cursor ?? 0) - 1, e.shiftKey);
    } else if (k === "Enter") {
      e.preventDefault();
      deliver(!e.metaKey);
    } else if (e.metaKey && k.toLowerCase() === "c") {
      if (hasTextSelection()) return; // 选中了文字时保留系统复制
      e.preventDefault();
      deliver(false);
    } else if (e.metaKey && /^[1-9]$/.test(k)) {
      e.preventDefault();
      const item = col?.items[Number(k) - 1];
      if (item) deliver(true, [item]);
    } else if (e.metaKey && k.toLowerCase() === "s") {
      e.preventDefault();
      toggleFavorite();
    } else if (e.metaKey && k.toLowerCase() === "e") {
      e.preventDefault();
      if (current) toggleExpand(current);
    } else if (e.metaKey && k.toLowerCase() === "k") {
      e.preventDefault();
      openMenuAtCursor();
    } else if (e.metaKey && k === "Backspace") {
      e.preventDefault();
      remove();
    } else if (k === "Tab") {
      e.preventDefault();
      if (layout === "dual") switchColumn(e.shiftKey ? -1 : 1);
      else cycleFilter(e.shiftKey ? -1 : 1);
    } else if (k === "Backspace" && !query && document.activeElement === input) {
      e.preventDefault();
      onBack();
    } else if (k === "Escape") {
      e.preventDefault();
      if (picked.length) picked = [];
      else if (query) query = "";
      else api.hideLauncher();
    } else if (k.length === 1 && !e.metaKey && !e.ctrlKey && document.activeElement !== input) {
      input.focus(); // 直接打字即开始搜索
    }
  }

  // ---------- 展示 ----------

  function timeAgo(ts: number): string {
    const diff = Date.now() / 1000 - ts;
    if (diff < 60) return "刚刚";
    if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
    if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
    const d = new Date(ts * 1000);
    const days = Math.floor(diff / 86400);
    if (days === 1) return "昨天";
    if (days < 7) return `${days} 天前`;
    return `${d.getMonth() + 1}月${d.getDate()}日`;
  }

  function metaText(item: ClipItem): string {
    if (item.kind === "text") return `${item.meta} 字`;
    if (item.kind === "files") return `${item.meta} 个文件`;
    return item.meta;
  }

  const fileName = (p: string) => p.split("/").filter(Boolean).pop() ?? p;

  onMount(() => {
    const un = listen("clipboard:changed", () => loadAll(true));
    const closeMenu = () => (menu = null);
    window.addEventListener("blur", closeMenu);
    return () => {
      un.then((f) => f());
      window.removeEventListener("blur", closeMenu);
    };
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet clipRow(item: ClipItem, i: number, ci: number)}
  {@const order = picked.indexOf(item.id)}
  {@const open = expanded.has(item.id)}
  {@const isCursor = ci === active && i === cols[ci].cursor}
  <!-- 键盘操作统一在窗口级处理，列表项只需响应鼠标 -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <li
    role="option"
    aria-selected={isCursor || order >= 0}
    class:cursor={isCursor}
    class:dim-cursor={!isCursor && i === cols[ci].cursor && cols.length > 1}
    class:picked={order >= 0}
    onclick={(e) => onItemClick(e, ci, i)}
    ondblclick={() => deliver(true, [item])}
    oncontextmenu={(e) => onContextMenu(e, ci, i)}
  >
    <span class="lead">
      {#if order >= 0}
        <span class="order">{order + 1}</span>
      {:else if item.kind === "image"}
        🖼
      {:else if item.kind === "files"}
        📁
      {:else}
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 6h14M5 12h14M5 18h9" /></svg>
      {/if}
    </span>
    <div class="main">
      {#if item.kind === "image"}
        <img
          class="img"
          class:open
          src={open ? item.image : item.thumb}
          alt=""
          draggable="false"
          style:max-height={open ? "none" : `${Math.max(maxLines, 2) * 20}px`}
        />
      {:else if item.kind === "files"}
        <div class="txt" class:open use:watchClip={item.id} style:--lines={maxLines}>
          {#each item.preview.split("\n") as f (f)}<div title={f}>{fileName(f)}</div>{/each}
        </div>
      {:else}
        <div class="txt mono" class:open use:watchClip={item.id} style:--lines={maxLines}>{open
            ? (fullText[item.id] ?? item.preview)
            : item.preview}</div>
      {/if}
      <div class="meta">
        <span>{item.source_app ?? "未知来源"}</span>
        <span>{timeAgo(item.used_at)}</span>
        <span>{metaText(item)}</span>
        {#each item.tags as t (t)}<span class="tag">#{t}</span>{/each}
        {#if item.kind === "image" || clipped.has(item.id) || item.more || open}
          <button
            class="expand"
            onclick={(e) => {
              e.stopPropagation();
              toggleExpand(item);
            }}>{open ? "收起" : "展开"}</button
          >
        {/if}
      </div>
    </div>
    <span class="side">
      {#if item.favorite}<span class="star" title="已收藏">★</span>{/if}
      {#if ci === active && i < 9}<kbd class="nth">⌘{i + 1}</kbd>{/if}
    </span>
  </li>
{/snippet}

{#snippet column(c: Col, ci: number)}
  <section class="col" class:active={ci === active && cols.length > 1}>
    <nav class="filters">
      {#if c.key === "favorite"}
        <span class="col-title">★ 收藏</span>
        <button class="pill" class:on={!c.tag} onclick={() => setTag(ci, null)}>全部</button>
        {#each tags as t (t)}
          <button class="pill" class:on={c.tag === t} onclick={() => setTag(ci, c.tag === t ? null : t)}>#{t}</button>
        {/each}
      {:else}
        {#if c.key === "history"}<span class="col-title">剪贴板</span>{/if}
        {#each c.key === "history" ? HISTORY_FILTERS : SINGLE_FILTERS as [f, label] (f)}
          <button class="pill" class:on={c.filter === f} onclick={() => setFilter(ci, f)}>{label}</button>
        {/each}
        {#if c.filter === "favorite" && tags.length}
          <span class="vsep"></span>
          {#each tags as t (t)}
            <button class="pill" class:on={c.tag === t} onclick={() => setTag(ci, c.tag === t ? null : t)}>#{t}</button>
          {/each}
        {/if}
      {/if}
    </nav>
    {#if c.items.length === 0}
      <div class="empty">
        {#if query}没有匹配的记录
        {:else if c.key === "favorite" || c.filter === "favorite"}还没有收藏，右键记录选择「收藏」
        {:else}还没有剪贴板记录，复制点什么试试{/if}
      </div>
    {:else}
      <ul class="list" bind:this={lists[ci]} role="listbox" aria-multiselectable="true">
        {#each c.items as item, i (item.id)}
          {@render clipRow(item, i, ci)}
        {/each}
      </ul>
    {/if}
  </section>
{/snippet}

<div class="view">
  <div class="search">
    <button class="btn tinted" onclick={onBack} title="返回搜索（空输入时按 ⌫）">剪贴板</button>
    <input
      bind:this={input}
      bind:value={query}
      placeholder="搜索剪贴板历史（图片可按识别出的文字搜索）…"
      spellcheck="false"
      autocomplete="off"
    />
    <div class="seg" role="group" aria-label="布局">
      <button class:on={layout === "single"} onclick={() => setLayout("single")} title="单栏">单栏</button>
      <button class:on={layout === "dual"} onclick={() => setLayout("dual")} title="剪贴板 + 收藏双栏">双栏</button>
    </div>
  </div>

  {#if ocrId !== null}
    <OcrPanel load={() => api.clipOcr(ocrId!)} onClose={() => { ocrId = null; loadAll(true); }} />
  {:else}
    <div class="body" class:dual={layout === "dual"}>
      {#each cols as c, ci (c.key)}
        {@render column(c, ci)}
      {/each}

      {#if sheet}
        <div class="scrim" role="presentation" onclick={() => (sheet = null)}></div>
        <div class="sheet" role="dialog" aria-modal="true">
          {#if sheet.type === "stats"}
            {@const s = textStats(sheet.text)}
            <div class="sheet-head"><strong>字数统计</strong><button class="btn sm" onclick={() => (sheet = null)}>关闭</button></div>
            <dl class="stats">
              {#each STAT_LABELS as [k, label] (k)}
                <div><dt>{label}</dt><dd>{s[k].toLocaleString()}</dd></div>
              {/each}
            </dl>
          {:else}
            {@const out = sheet.text}
            <div class="sheet-head">
              <strong>{sheet.name}</strong>
              <button class="btn sm" onclick={() => api.clipPasteText(out, false).catch((e) => (message = String(e)))}>复制</button>
              <button class="btn sm" onclick={() => api.clipPasteText(out, true).catch((e) => (message = String(e)))}>粘贴</button>
              <button class="btn sm" onclick={() => (sheet = null)}>关闭</button>
            </div>
            <pre class="output">{out || "（无输出）"}</pre>
          {/if}
        </div>
      {/if}

      {#if tagEdit && tagEditItem}
        <div class="scrim" role="presentation" onclick={() => (tagEdit = null)}></div>
        <div class="sheet tags-sheet" role="dialog" aria-modal="true">
          <div class="sheet-head"><strong>编辑标签</strong><button class="btn sm" onclick={() => (tagEdit = null)}>完成</button></div>
          <p class="hint">添加标签会自动收藏这条记录</p>
          <div class="tag-list">
            {#each tagEditItem.tags as t (t)}
              <span class="chip">#{t}<button onclick={() => removeTag(t)} title="移除">×</button></span>
            {:else}
              <span class="hint">还没有标签</span>
            {/each}
          </div>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="input"
            bind:this={tagInput}
            bind:value={tagEdit.draft}
            placeholder="输入标签，回车添加"
            autofocus
            list="wtools-tags"
            onkeydown={(e) => {
              if (e.key === "Enter" && !e.isComposing) {
                e.preventDefault();
                addTag();
              }
            }}
          />
          <datalist id="wtools-tags">
            {#each tags as t (t)}<option value={t}></option>{/each}
          </datalist>
        </div>
      {/if}
    </div>

    <footer class="statusbar">
      {#if message}
        <span class="message">{message}</span>
      {:else if picked.length > 1}
        <span>已选 {picked.length} 条</span>
        <span><kbd>↩</kbd> 按顺序粘贴</span>
        <span><kbd>⌘C</kbd> 复制</span>
        <span><kbd>⌘S</kbd> 收藏</span>
        <span><kbd>⌘⌫</kbd> 删除</span>
        <span class="spacer"></span>
        <span><kbd>esc</kbd> 取消多选</span>
      {:else}
        <span><kbd>↩</kbd> 粘贴</span>
        <span><kbd>⌘C</kbd> 复制</span>
        <span><kbd>⌘E</kbd> 展开</span>
        <span><kbd>⌘K</kbd>/右键 更多操作</span>
        <span><kbd>⇧↓</kbd>/<kbd>⌘点击</kbd> 多选</span>
        <span class="spacer"></span>
        <span><kbd>tab</kbd> {layout === "dual" ? "切换栏" : "切换类型"}</span>
      {/if}
    </footer>
  {/if}

  {#if menu}
    <div class="menu-backdrop" role="presentation" onclick={() => (menu = null)} oncontextmenu={(e) => { e.preventDefault(); menu = null; }}></div>
    <div class="menu" role="menu" style:left="{menu.x}px" style:top="{menu.y}px">
      {#each menuItems as it, i (it.id)}
        {#if it.sep && i > 0}<div class="hsep"></div>{/if}
        <button
          role="menuitem"
          class:active={i === menuIndex}
          class:danger={it.danger}
          onmousemove={() => (menuIndex = i)}
          onclick={() => runMenu(i)}
        >
          <span>{it.name}</span>
          {#if it.hint}<kbd>{it.hint}</kbd>{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }

  .search {
    height: var(--h-search);
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: 0 var(--sp-4);
    border-bottom: 1px solid var(--border);
  }

  .search input {
    flex: 1;
    height: 100%;
    border: none;
    outline: none;
    background: transparent;
    font-size: var(--fs-xl);
    color: var(--text);
  }

  .search input::placeholder {
    color: var(--text-tertiary);
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    position: relative;
  }

  .col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .body.dual .col + .col {
    border-left: 1px solid var(--border);
  }

  .filters {
    height: var(--h-bar);
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0 var(--sp-3);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
  }

  .col-title {
    flex: none;
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
    margin-right: var(--sp-2);
    color: var(--text-secondary);
  }

  .col.active .col-title {
    color: var(--accent);
  }

  .empty {
    flex: 1;
  }

  .list {
    flex: 1;
    min-height: 0;
  }

  /* 列表行：多选的已选、焦点栏光标、非焦点栏光标三种状态 */
  li[role="option"] {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--r-md);
  }

  li[role="option"] + li[role="option"] {
    margin-top: 2px;
  }

  li.picked {
    background: color-mix(in srgb, var(--selected) 60%, transparent);
  }

  li.dim-cursor {
    background: var(--hover);
  }

  li.cursor {
    background: var(--selected);
  }

  .lead {
    width: 22px;
    height: 20px;
    flex: none;
    display: grid;
    place-items: center;
    font-size: var(--fs-md);
  }

  .lead svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: var(--text-secondary);
    stroke-width: 2;
    stroke-linecap: round;
  }

  .order {
    width: 20px;
    height: 20px;
    border-radius: var(--r-full);
    background: var(--accent);
    color: var(--on-accent);
    font-size: var(--fs-xs);
    display: grid;
    place-items: center;
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .txt {
    font-size: var(--fs-md);
    line-height: 20px;
    white-space: pre-wrap;
    word-break: break-word;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: var(--lines);
    line-clamp: var(--lines);
    overflow: hidden;
  }

  .txt.mono {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }

  .txt.open {
    display: block;
    -webkit-line-clamp: unset;
    line-clamp: unset;
    user-select: text;
    -webkit-user-select: text;
  }

  .img {
    align-self: flex-start;
    max-width: 100%;
    min-height: 32px;
    object-fit: contain;
    border-radius: var(--r-xs);
    background: repeating-conic-gradient(var(--hover) 0 25%, transparent 0 50%) 0 0 / 12px 12px;
  }

  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-1) var(--sp-3);
    font-size: var(--fs-xs);
    color: var(--text-secondary);
  }

  .tag {
    color: var(--accent);
  }

  .expand {
    margin-left: auto;
    height: 20px;
    padding: 0 var(--sp-2);
    border: 1px solid var(--border);
    border-radius: var(--r-full);
    background: var(--surface);
    color: var(--accent);
    font-size: var(--fs-xs);
    cursor: pointer;
  }

  .side {
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--sp-1);
    min-width: 24px;
  }

  .star {
    color: var(--star);
    font-size: var(--fs-md);
  }

  .nth {
    background: none;
    padding: 0;
    color: var(--text-tertiary);
  }

  .scrim {
    z-index: 5;
  }

  .sheet {
    position: absolute;
    z-index: 6;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(640px, 90%);
    max-height: 80%;
  }

  .tags-sheet {
    width: min(420px, 90%);
  }

  .output {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--sp-3);
    border-radius: var(--r-md);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }

  .stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--sp-2);
  }

  .stats div {
    padding: var(--sp-3);
    border-radius: var(--r-md);
    background: var(--surface-2);
  }

  .stats dt {
    font-size: var(--fs-xs);
    color: var(--text-secondary);
  }

  .stats dd {
    font-size: var(--fs-xl);
    font-weight: var(--fw-semibold);
    margin-top: 2px;
    font-variant-numeric: tabular-nums;
  }

  .tag-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1h);
  }

  .tags-sheet .input {
    height: var(--h-lg);
  }

  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 19;
  }

  .menu {
    position: fixed;
    z-index: 20;
  }

  .message {
    color: var(--danger);
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
