<script lang="ts">
  import { onMount } from "svelte";
  import { api, type OcrResult } from "./api";

  let {
    load,
    onClose,
    deliver: send = (text: string, paste: boolean) => api.clipPasteText(text, paste),
    closeLabel = "‹ 返回",
    compact = false,
  }: {
    /** 获取识别结果 */
    load: () => Promise<OcrResult>;
    onClose: () => void;
    /** 复制 / 粘贴选中文字，默认走剪贴板模块 */
    deliver?: (text: string, paste: boolean) => Promise<void>;
    closeLabel?: string;
    /** 窄面板：底部只显示快捷键 */
    compact?: boolean;
  } = $props();

  let result = $state<OcrResult | null>(null);
  let error = $state("");
  let message = $state("");
  // 选中的词块，键为 "行号:词号"
  let picked = $state<Set<string>>(new Set());
  let board = $state<HTMLDivElement>();

  // 框选
  let box = $state<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
  let boxBase: Set<string> = new Set();

  const key = (l: number, t: number) => `${l}:${t}`;
  const total = $derived(result?.lines.reduce((n, l) => n + l.tokens.length, 0) ?? 0);

  onMount(() => {
    load()
      .then((r) => (result = r))
      .catch((e) => (error = String(e)));
  });

  function toggle(l: number, t: number) {
    const k = key(l, t);
    const next = new Set(picked);
    if (next.has(k)) next.delete(k);
    else next.add(k);
    picked = next;
  }

  function toggleLine(l: number) {
    if (!result) return;
    const keys = result.lines[l].tokens.map((_, t) => key(l, t));
    const allIn = keys.every((k) => picked.has(k));
    const next = new Set(picked);
    keys.forEach((k) => (allIn ? next.delete(k) : next.add(k)));
    picked = next;
  }

  function selectAll() {
    if (!result) return;
    picked =
      picked.size === total
        ? new Set()
        : new Set(result.lines.flatMap((line, l) => line.tokens.map((_, t) => key(l, t))));
  }

  /** 按阅读顺序拼接选中的词块；未选中任何词块时返回全文 */
  function selectedText(): string {
    if (!result) return "";
    if (picked.size === 0) return result.lines.map((l) => l.text).join("\n");
    const out: string[] = [];
    result.lines.forEach((line, l) => {
      let s = "";
      line.tokens.forEach((tok, t) => {
        if (!picked.has(key(l, t))) return;
        s += (s && tok.space ? " " : "") + tok.text;
      });
      if (s) out.push(s);
    });
    return out.join("\n");
  }

  async function deliver(paste: boolean) {
    const text = selectedText();
    if (!text) return;
    try {
      await send(text, paste);
    } catch (e) {
      message = String(e);
    }
  }

  // ---- 框选：在空白处按下拖动，选中与矩形相交的词块；按住 ⌘ 时追加 ----
  function onPointerDown(e: PointerEvent) {
    if ((e.target as HTMLElement).closest("button")) return;
    const r = board!.getBoundingClientRect();
    const x = e.clientX - r.left + board!.scrollLeft;
    const y = e.clientY - r.top + board!.scrollTop;
    box = { x0: x, y0: y, x1: x, y1: y };
    boxBase = e.metaKey ? new Set(picked) : new Set();
    if (!e.metaKey) picked = new Set();
    board!.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!box || !board) return;
    const r = board.getBoundingClientRect();
    box = { ...box, x1: e.clientX - r.left + board.scrollLeft, y1: e.clientY - r.top + board.scrollTop };
    const [left, right] = [Math.min(box.x0, box.x1), Math.max(box.x0, box.x1)];
    const [top, bottom] = [Math.min(box.y0, box.y1), Math.max(box.y0, box.y1)];
    const next = new Set(boxBase);
    board.querySelectorAll<HTMLElement>("[data-k]").forEach((el) => {
      const b = el.getBoundingClientRect();
      const bx = b.left - r.left + board!.scrollLeft;
      const by = b.top - r.top + board!.scrollTop;
      if (bx < right && bx + b.width > left && by < bottom && by + b.height > top) {
        next.add(el.dataset.k!);
      }
    });
    picked = next;
  }

  function onPointerUp() {
    box = null;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (picked.size) picked = new Set();
      else onClose();
    } else if (e.metaKey && e.key.toLowerCase() === "a") {
      e.preventDefault();
      selectAll();
    } else if (e.metaKey && e.key.toLowerCase() === "c") {
      e.preventDefault();
      deliver(false);
    } else if (e.key === "Enter") {
      e.preventDefault();
      deliver(true);
    }
  }
</script>

<svelte:window onkeydowncapture={onKeydown} />

<section class="ocr">
  <header class="bar">
    <button class="btn sm plain back" onclick={onClose}>{closeLabel}</button>
    <span class="title">识别结果</span>
    {#if result}
      <span class="count">{picked.size ? `已选 ${picked.size} / ${total}` : `共 ${total} 个词块`}</span>
    {/if}
    <span class="spacer"></span>
    <button class="btn sm" onclick={selectAll} disabled={!result}>{picked.size === total && total ? "取消全选" : "全选"}</button>
    <button class="btn sm" onclick={() => deliver(false)} disabled={!result}>复制{picked.size ? "所选" : "全部"}</button>
    <button class="btn sm primary" onclick={() => deliver(true)} disabled={!result}>粘贴{picked.size ? "所选" : "全部"}</button>
  </header>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="board"
    bind:this={board}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
  >
    {#if error}
      <p class="state error">{error}</p>
    {:else if !result}
      <p class="state">正在识别…</p>
    {:else if result.lines.length === 0}
      <p class="state">没有识别到文字</p>
    {:else}
      {#each result.lines as line, l (l)}
        <div class="line">
          <button class="gutter" title="选择整行" onclick={() => toggleLine(l)}>{l + 1}</button>
          <div class="tokens">
            {#each line.tokens as tok, t (t)}
              <button
                class="tok"
                class:on={picked.has(key(l, t))}
                class:atomic={tok.atomic}
                class:gap={tok.space}
                data-k={key(l, t)}
                onclick={() => toggle(l, t)}>{tok.text}</button
              >
            {/each}
          </div>
        </div>
      {/each}
    {/if}
    {#if box}
      <div
        class="marquee"
        style:left="{Math.min(box.x0, box.x1)}px"
        style:top="{Math.min(box.y0, box.y1)}px"
        style:width="{Math.abs(box.x1 - box.x0)}px"
        style:height="{Math.abs(box.y1 - box.y0)}px"
      ></div>
    {/if}
  </div>

  <footer class="statusbar">
    {#if message}
      <span class="message">{message}</span>
    {:else}
      <span>{compact ? "点选 / 拖动框选 / 点行号" : "点选词块、在空白处拖动框选、点行号选整行（按住 ⌘ 追加）"}</span>
      <span class="spacer"></span>
      <span><kbd>⌘A</kbd> 全选</span>
      <span><kbd>⌘C</kbd> 复制</span>
      <span><kbd>↩</kbd> 粘贴</span>
      <span><kbd>esc</kbd> {closeLabel.replace(/^[‹✕]\s*/, "")}</span>
    {/if}
  </footer>
</section>

<style>
  .ocr {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .back {
    color: var(--accent);
    padding: 0 var(--sp-1h);
  }

  .title {
    font-weight: var(--fw-semibold);
  }

  .count {
    color: var(--text-secondary);
    font-size: var(--fs-sm);
  }

  .board {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--sp-3);
    position: relative;
    touch-action: none;
  }

  .state {
    color: var(--text-secondary);
    font-size: var(--fs-md);
    text-align: center;
    margin-top: 80px;
  }

  .state.error {
    color: var(--danger);
  }

  .line {
    display: flex;
    gap: var(--sp-2);
    align-items: flex-start;
    padding: 3px 0;
  }

  .gutter {
    width: 26px;
    flex: none;
    height: 26px;
    border: none;
    background: none;
    color: var(--text-secondary);
    font-size: var(--fs-xs);
    cursor: pointer;
    border-radius: var(--r-xs);
  }

  .gutter:hover {
    background: var(--hover);
  }

  .tokens {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
  }

  .tok {
    height: 26px;
    padding: 0 var(--sp-2);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-md);
    cursor: pointer;
    white-space: pre;
  }

  .tok.gap {
    margin-left: var(--sp-1h);
  }

  .tok.atomic {
    border-style: dashed;
  }

  .tok.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }

  .marquee {
    position: absolute;
    border: 1px solid var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    pointer-events: none;
  }

  .message {
    color: var(--danger);
  }
</style>
