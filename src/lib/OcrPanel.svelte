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

  // 滑动连选：按阅读顺序把锚点到当前位置之间的词块统一选中（或取消选中）
  type Span = [number, number];
  let sweep: { from: Span; add: boolean; base: Set<string>; lines: boolean } | null = null;
  /** 上次点选的词块序号，⇧ 点选时从这里开始连选 */
  let anchor = -1;

  const key = (l: number, t: number) => `${l}:${t}`;
  /** 所有词块按阅读顺序排列的键 */
  const order = $derived(result?.lines.flatMap((line, l) => line.tokens.map((_, t) => key(l, t))) ?? []);
  /** 每行第一个词块的序号 */
  const starts = $derived.by(() => {
    let n = 0;
    return (result?.lines ?? []).map((line) => {
      const s = n;
      n += line.tokens.length;
      return s;
    });
  });
  const total = $derived(order.length);

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
    anchor = starts[l] + t;
  }

  /** 第 l 行的词块序号区间（空行返回 [n, n-1]） */
  const lineSpan = (l: number): Span => [starts[l], starts[l] + result!.lines[l].tokens.length - 1];
  const lineOf = (i: number) => {
    let l = starts.length - 1;
    while (l > 0 && starts[l] > i) l--;
    return l;
  };

  function toggleLine(l: number) {
    if (!result) return;
    const keys = result.lines[l].tokens.map((_, t) => key(l, t));
    const allIn = keys.every((k) => picked.has(k));
    const next = new Set(picked);
    keys.forEach((k) => (allIn ? next.delete(k) : next.add(k)));
    picked = next;
    anchor = starts[l];
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

  /** 把 from 与 to 之间（含两端）的词块按 sweep 的方式应用到选择上 */
  function extendTo(to: Span) {
    if (!sweep) return;
    const lo = Math.min(sweep.from[0], to[0]);
    const hi = Math.max(sweep.from[1], to[1]);
    const next = new Set(sweep.base);
    for (let i = lo; i <= hi; i++) {
      if (sweep.add) next.add(order[i]);
      else next.delete(order[i]);
    }
    picked = next;
  }

  /** 指针位置对应的区间：词块本身、行号对应的整行；落在空白处时取最近的词块 */
  function hitSpan(x: number, y: number): Span | null {
    if (!board || !sweep) return null;
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-i],[data-line]");
    let i = -1;
    if (el && board.contains(el)) {
      i = el.dataset.i !== undefined ? +el.dataset.i : starts[+el.dataset.line!];
    } else {
      // 先比较垂直距离（同一行优先），再比较水平距离
      let best = Infinity;
      board.querySelectorAll<HTMLElement>("[data-i]").forEach((t) => {
        const b = t.getBoundingClientRect();
        const dy = Math.max(b.top - y, 0, y - b.bottom);
        const dx = Math.max(b.left - x, 0, x - b.right);
        const d = dy * 10000 + dx;
        if (d < best) {
          best = d;
          i = +t.dataset.i!;
        }
      });
    }
    if (i < 0) return null;
    return sweep.lines ? lineSpan(lineOf(i)) : [i, i];
  }

  /** 拖到可视区域边缘时滚动 */
  function autoScroll(e: PointerEvent) {
    const r = board!.getBoundingClientRect();
    const edge = 28;
    if (e.clientY < r.top + edge) board!.scrollTop -= 14;
    else if (e.clientY > r.bottom - edge) board!.scrollTop += 14;
  }

  // ---- 按下词块：点选 / 滑动连选；⇧ 从上次点选处连选到这里。按下行号同理，以整行为单位 ----
  // ---- 按下空白处：框选与矩形相交的词块；按住 ⌘ 时追加 ----
  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-i],[data-line]");
    if (el && result) {
      e.preventDefault();
      const lines = el.dataset.line !== undefined;
      const span: Span = lines ? lineSpan(+el.dataset.line!) : [+el.dataset.i!, +el.dataset.i!];
      if (span[1] < span[0]) return;
      if (e.shiftKey && anchor >= 0 && anchor < total) {
        // 区间选：保留已有选择，追加锚点到这里之间的词块
        sweep = { from: lines ? lineSpan(lineOf(anchor)) : [anchor, anchor], add: true, base: new Set(picked), lines };
      } else {
        // 起点未选中则本次拖过的都选中，已选中则都取消，单击即切换
        const add = lines
          ? !order.slice(span[0], span[1] + 1).every((k) => picked.has(k))
          : !picked.has(order[span[0]]);
        sweep = { from: span, add, base: new Set(picked), lines };
        anchor = span[0];
      }
      extendTo(span);
      board!.setPointerCapture(e.pointerId);
      return;
    }
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
    if (sweep) {
      autoScroll(e);
      const span = hitSpan(e.clientX, e.clientY);
      if (span) extendTo(span);
      return;
    }
    if (!box || !board) return;
    autoScroll(e);
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
    sweep = null;
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
    onpointercancel={onPointerUp}
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
          <button
            class="gutter"
            title="选择整行（拖动选多行，⇧ 点击连选）"
            data-line={l}
            onclick={(e) => e.detail === 0 && toggleLine(l)}>{l + 1}</button
          >
          <div class="tokens">
            {#each line.tokens as tok, t (t)}
              <button
                class="tok"
                class:on={picked.has(key(l, t))}
                class:atomic={tok.atomic}
                class:gap={tok.space}
                data-k={key(l, t)}
                data-i={starts[l] + t}
                onclick={(e) => e.detail === 0 && toggle(l, t)}>{tok.text}</button
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
      <span>{compact ? "点选 / 滑选 / ⇧ 区间" : "点选或滑动连选词块，⇧ 点击选区间，空白处拖动框选（⌘ 追加），行号选整行"}</span>
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
    user-select: none;
    -webkit-user-select: none;
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
