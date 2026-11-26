<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import OcrPanel from "../lib/OcrPanel.svelte";
  import type { OcrResult } from "../lib/api";
  import {
    COLORS,
    FONT,
    LINE_HEIGHT,
    SIZES,
    bounds,
    constrain,
    degenerate,
    draw,
    hit,
    move,
    normRect,
    type DrawEnv,
    type Pt,
    type Rect,
    type Shape,
    type Tool,
  } from "./shapes";
  import { icon } from "./icons";

  interface StartInfo {
    seq: number;
    mode: "capture" | "ocr";
    width: number;
    height: number;
    lw: number;
    lh: number;
    scale: number;
    windows: Rect[];
  }

  type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
  type Drag =
    | { kind: "select"; start: Pt; moved: boolean }
    | { kind: "move-sel"; start: Pt; orig: Rect }
    | { kind: "resize"; handle: Handle; orig: Rect }
    | { kind: "draw" }
    | { kind: "move-shape"; start: Pt; orig: Shape[]; id: number; moved: boolean };

  const TOOLS: { id: Tool; name: string; key: string }[] = [
    { id: "rect", name: "矩形", key: "R" },
    { id: "ellipse", name: "椭圆", key: "E" },
    { id: "arrow", name: "箭头", key: "A" },
    { id: "line", name: "直线", key: "L" },
    { id: "pen", name: "画笔", key: "P" },
    { id: "marker", name: "马克笔", key: "M" },
    { id: "mosaic", name: "马赛克", key: "X" },
    { id: "text", name: "文字", key: "T" },
    { id: "number", name: "序号", key: "N" },
  ];
  const TOOL_KEYS: Record<string, Tool> = Object.fromEntries(TOOLS.map((t) => [t.key.toLowerCase(), t.id]));
  const HANDLES: Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

  let info = $state<StartInfo | null>(null);
  let phase = $state<"select" | "edit" | "ocr">("select");
  let base: HTMLCanvasElement;
  let layer: HTMLCanvasElement;
  let mag = $state<HTMLCanvasElement>();
  let baseCtx: CanvasRenderingContext2D | null = null;

  let sel = $state<Rect | null>(null);
  let hover = $state<Rect | null>(null);
  let cursor = $state<Pt>({ x: -1000, y: -1000 });
  let pixel = $state({ hex: "#000000", rgb: "0, 0, 0" });
  // 放大镜十字线用强调色（取自设计令牌 --accent，画布不支持 CSS 变量，这里换算成 rgba）
  const accentWash = (() => {
    const hex = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim().replace("#", "");
    const n = parseInt(hex.length === 6 ? hex : "0a66ff", 16);
    return `rgba(${n >> 16}, ${(n >> 8) & 255}, ${n & 255}, 0.35)`;
  })();
  let showMag = $state(false);
  let cursorStyle = $state("crosshair");

  let tool = $state<Tool | null>(null);
  let color = $state(COLORS[0]);
  let sizeIdx = $state(1);
  let shapes = $state.raw<Shape[]>([]);
  let draft = $state.raw<Shape | null>(null);
  let picked = $state<number | null>(null);
  let undoStack = $state.raw<Shape[][]>([]);
  let redoStack = $state.raw<Shape[][]>([]);
  let textEdit = $state<{ a: Pt; text: string; size: number; color: string; orig: Shape[]; existing: boolean } | null>(null);
  let textEl = $state<HTMLDivElement>();
  let busy = $state(false);
  let toolbarW = $state(560);

  let drag: Drag | null = null;
  let nextId = 1;
  const mosaicCache = new Map<number, HTMLCanvasElement>();

  // ---------- 加载 ----------

  async function begin() {
    const p = await invoke<StartInfo | null>("shot_take_pending");
    if (!p) return;
    reset();
    const buf = await invoke<ArrayBuffer>("shot_image", { seq: p.seq });
    base.width = layer.width = p.width;
    base.height = layer.height = p.height;
    baseCtx = base.getContext("2d", { willReadFrequently: true })!;
    baseCtx.putImageData(new ImageData(new Uint8ClampedArray(buf), p.width, p.height), 0, 0);
    info = p;
    await tick();
    render();
    await invoke("shot_show");
  }

  function reset() {
    phase = "select";
    sel = hover = null;
    tool = null;
    shapes = [];
    draft = null;
    picked = null;
    undoStack = [];
    redoStack = [];
    textEdit = null;
    busy = false;
    showMag = false;
    drag = null;
    mosaicCache.clear();
  }

  /** 结束后释放截图占用的内存（窗口只隐藏不销毁） */
  function release() {
    info = null;
    reset();
    baseCtx = null;
    base.width = base.height = layer.width = layer.height = 1;
  }

  // ---------- 绘制 ----------

  const env: DrawEnv = {
    mosaic(block) {
      let c = mosaicCache.get(block);
      if (!c && info) {
        c = document.createElement("canvas");
        c.width = Math.ceil(info.lw / block);
        c.height = Math.ceil(info.lh / block);
        const ctx = c.getContext("2d")!;
        ctx.imageSmoothingQuality = "medium";
        // 整屏缩小：每个像素约等于一个块（源区域必须在图像范围内，否则 WebKit 不绘制）
        ctx.drawImage(base, 0, 0, base.width, base.height, 0, 0, info.lw / block, info.lh / block);
        mosaicCache.set(block, c);
      }
      return c!;
    },
  };

  function drawShapes(ctx: CanvasRenderingContext2D, list: Shape[]) {
    for (const s of list) draw(ctx, s, env);
  }

  function render() {
    if (!info || !layer) return;
    const ctx = layer.getContext("2d")!;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, layer.width, layer.height);
    if (!sel) return;
    ctx.setTransform(info.scale, 0, 0, info.scale, 0, 0);
    ctx.save();
    ctx.beginPath();
    ctx.rect(sel.x, sel.y, sel.w, sel.h);
    ctx.clip();
    drawShapes(ctx, draft ? [...shapes, draft] : shapes);
    ctx.restore();
  }

  let raf = 0;
  $effect(() => {
    void shapes;
    void draft;
    void sel;
    if (!raf) raf = requestAnimationFrame(() => ((raf = 0), render()));
  });

  /** 合成选区内的截图与标注，返回 PNG 字节 */
  async function exportPng(): Promise<Uint8Array> {
    const { scale } = info!;
    const r = sel!;
    const [px, py] = [Math.round(r.x * scale), Math.round(r.y * scale)];
    const [pw, ph] = [Math.max(1, Math.round(r.w * scale)), Math.max(1, Math.round(r.h * scale))];
    const c = document.createElement("canvas");
    c.width = pw;
    c.height = ph;
    const ctx = c.getContext("2d")!;
    ctx.drawImage(base, px, py, pw, ph, 0, 0, pw, ph);
    ctx.setTransform(scale, 0, 0, scale, -px, -py);
    drawShapes(ctx, shapes);
    const blob = await new Promise<Blob | null>((res) => c.toBlob(res, "image/png"));
    if (!blob) throw new Error("生成图片失败");
    return new Uint8Array(await blob.arrayBuffer());
  }

  // ---------- 几何 ----------

  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
  const inside = (p: Pt, r: Rect) => p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h;
  const pt = (e: MouseEvent): Pt => ({ x: e.clientX, y: e.clientY });
  const screenRect = (): Rect => ({ x: 0, y: 0, w: info!.lw, h: info!.lh });
  const findWindow = (p: Pt) => info!.windows.find((w) => inside(p, w)) ?? screenRect();
  const clampPt = (p: Pt): Pt => (sel ? { x: clamp(p.x, sel.x, sel.x + sel.w), y: clamp(p.y, sel.y, sel.y + sel.h) } : p);
  const toolSize = (t: Tool) => SIZES[t][sizeIdx];
  const topHit = (p: Pt) => [...shapes].reverse().find((s) => hit(s, p));

  function handlePoints(r: Rect): Record<Handle, Pt> {
    const [x0, y0, x1, y1] = [r.x, r.y, r.x + r.w, r.y + r.h];
    const [cx, cy] = [r.x + r.w / 2, r.y + r.h / 2];
    return { nw: { x: x0, y: y0 }, n: { x: cx, y: y0 }, ne: { x: x1, y: y0 }, e: { x: x1, y: cy }, se: { x: x1, y: y1 }, s: { x: cx, y: y1 }, sw: { x: x0, y: y1 }, w: { x: x0, y: cy } };
  }

  function handleAt(p: Pt): Handle | null {
    if (!sel) return null;
    const pts = handlePoints(sel);
    return HANDLES.find((h) => Math.abs(pts[h].x - p.x) <= 6 && Math.abs(pts[h].y - p.y) <= 6) ?? null;
  }

  function resized(r: Rect, h: Handle, p: Pt): Rect {
    let [x0, y0, x1, y1] = [r.x, r.y, r.x + r.w, r.y + r.h];
    const q = { x: clamp(p.x, 0, info!.lw), y: clamp(p.y, 0, info!.lh) };
    if (h.includes("w")) x0 = q.x;
    if (h.includes("e")) x1 = q.x;
    if (h.includes("n")) y0 = q.y;
    if (h.includes("s")) y1 = q.y;
    return normRect({ x: x0, y: y0 }, { x: x1, y: y1 });
  }

  // ---------- 鼠标 ----------

  function onDown(e: PointerEvent) {
    if (!info || e.button !== 0) return;
    const p = pt(e);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    if (textEdit) {
      commitText();
      if (tool === "text") return;
    }
    if (phase === "select") {
      drag = { kind: "select", start: p, moved: false };
      return;
    }
    if (phase !== "edit" || !sel) return;
    const h = handleAt(p);
    if (h) {
      drag = { kind: "resize", handle: h, orig: { ...sel } };
      return;
    }
    if (tool) {
      const target = topHit(p);
      if (tool === "text") {
        if (target?.t === "text") editText(target);
        else startText(clampPt(p));
        return;
      }
      if (tool === "number") {
        const n = Math.max(0, ...shapes.map((s) => (s.t === "number" ? s.n : 0))) + 1;
        commit([...shapes, { id: nextId++, t: "number", a: clampPt(p), n, color, size: toolSize("number") }]);
        return;
      }
      const a = clampPt(p);
      draft =
        tool === "pen" || tool === "marker"
          ? { id: nextId++, t: tool, pts: [a], color, size: toolSize(tool) }
          : { id: nextId++, t: tool, a, b: a, color, size: toolSize(tool) };
      drag = { kind: "draw" };
      return;
    }
    const target = topHit(p);
    if (target) {
      picked = target.id;
      drag = { kind: "move-shape", start: p, orig: shapes, id: target.id, moved: false };
      return;
    }
    picked = null;
    if (shapes.length) return;
    if (inside(p, sel)) {
      drag = { kind: "move-sel", start: p, orig: { ...sel } };
    } else {
      // 选区外按下：重新框选
      phase = "select";
      sel = null;
      drag = { kind: "select", start: p, moved: false };
    }
  }

  function onMove(e: PointerEvent) {
    if (!info) return;
    const p = pt(e);
    cursor = p;
    if (!drag) {
      if (phase === "select") {
        hover = findWindow(p);
        updateMagnifier(p);
      } else if (phase === "edit") {
        cursorStyle = cursorAt(p);
      }
      return;
    }
    switch (drag.kind) {
      case "select": {
        if (!drag.moved && Math.hypot(p.x - drag.start.x, p.y - drag.start.y) > 3) drag.moved = true;
        if (drag.moved) {
          const q = { x: clamp(p.x, 0, info.lw), y: clamp(p.y, 0, info.lh) };
          sel = normRect(drag.start, q);
          hover = null;
        }
        updateMagnifier(p);
        break;
      }
      case "move-sel": {
        const o = drag.orig;
        sel = {
          ...o,
          x: clamp(o.x + p.x - drag.start.x, 0, info.lw - o.w),
          y: clamp(o.y + p.y - drag.start.y, 0, info.lh - o.h),
        };
        break;
      }
      case "resize":
        sel = resized(drag.orig, drag.handle, p);
        break;
      case "draw": {
        const d = draft;
        if (!d) break;
        const q = clampPt(p);
        if (d.t === "pen" || d.t === "marker") {
          const last = d.pts[d.pts.length - 1];
          if (e.shiftKey) draft = { ...d, pts: [d.pts[0], { x: q.x, y: d.pts[0].y }] };
          else if (Math.hypot(q.x - last.x, q.y - last.y) >= 1.5) draft = { ...d, pts: [...d.pts, q] };
        } else if ("b" in d) {
          draft = { ...d, b: e.shiftKey ? clampPt(constrain(d.t, d.a, q)) : q };
        }
        break;
      }
      case "move-shape": {
        const { start, orig, id } = drag;
        drag.moved = true;
        shapes = orig.map((s) => (s.id === id ? move(s, p.x - start.x, p.y - start.y) : s));
        break;
      }
    }
  }

  function onUp(e: PointerEvent) {
    const d = drag;
    drag = null;
    if (!d || !info) return;
    switch (d.kind) {
      case "select":
        if (d.moved) {
          if (sel && sel.w >= 4 && sel.h >= 4) enterEdit();
          else sel = null;
        } else {
          sel = findWindow(d.start);
          enterEdit();
        }
        break;
      case "draw":
        if (draft && !degenerate(draft)) commit([...shapes, draft]);
        draft = null;
        break;
      case "move-shape":
        if (d.moved) pushHistory(d.orig);
        break;
    }
    if (phase === "edit") cursorStyle = cursorAt(pt(e));
  }

  function onDblClick(e: MouseEvent) {
    if (phase !== "edit" || !sel) return;
    const p = pt(e);
    const target = topHit(p);
    if (target?.t === "text") editText(target);
    else if (!tool && inside(p, sel)) finishCopy();
  }

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    if (phase === "edit" && !shapes.length && !textEdit) {
      // 右键：放弃当前选区，重新框选
      phase = "select";
      sel = null;
      tool = null;
      hover = findWindow(pt(e));
      cursorStyle = "crosshair";
    } else if (phase === "select") {
      cancel();
    }
  }

  function cursorAt(p: Pt): string {
    const h = handleAt(p);
    if (h) return { nw: "nwse-resize", se: "nwse-resize", ne: "nesw-resize", sw: "nesw-resize", n: "ns-resize", s: "ns-resize", e: "ew-resize", w: "ew-resize" }[h];
    if (tool) return tool === "text" ? "text" : "crosshair";
    if (topHit(p)) return "move";
    if (sel && inside(p, sel) && !shapes.length) return "move";
    return "default";
  }

  function enterEdit() {
    showMag = false;
    hover = null;
    if (info?.mode === "ocr") startOcr();
    else {
      phase = "edit";
      cursorStyle = "default";
    }
  }

  // ---------- 放大镜 / 取色 ----------

  function updateMagnifier(p: Pt) {
    showMag = true;
    if (!baseCtx || !info || !mag) return;
    const s = info.scale;
    const N = 15;
    const px = clamp(Math.floor(p.x * s), 0, info.width - 1);
    const py = clamp(Math.floor(p.y * s), 0, info.height - 1);
    const d = baseCtx.getImageData(px, py, 1, 1).data;
    pixel = {
      hex: "#" + [d[0], d[1], d[2]].map((v) => v.toString(16).padStart(2, "0")).join("").toUpperCase(),
      rgb: `${d[0]}, ${d[1]}, ${d[2]}`,
    };
    const m = mag.getContext("2d")!;
    const cell = mag.width / N;
    const sx = clamp(px - (N >> 1), 0, info.width - N);
    const sy = clamp(py - (N >> 1), 0, info.height - N);
    m.imageSmoothingEnabled = false;
    m.drawImage(base, sx, sy, N, N, 0, 0, mag.width, mag.height);
    const cx = (px - sx) * cell;
    const cy = (py - sy) * cell;
    m.fillStyle = accentWash;
    m.fillRect(0, cy, mag.width, cell);
    m.fillRect(cx, 0, cell, mag.height);
    m.strokeStyle = "#fff";
    m.lineWidth = 1.5;
    m.strokeRect(cx, cy, cell, cell);
  }

  function copyColor(rgb: boolean) {
    finishText(rgb ? `rgb(${pixel.rgb})` : pixel.hex, false);
  }

  // ---------- 标注 ----------

  function commit(next: Shape[]) {
    pushHistory(shapes);
    shapes = next;
  }

  function pushHistory(prev: Shape[]) {
    undoStack = [...undoStack, prev];
    redoStack = [];
  }

  function undo() {
    if (!undoStack.length) return;
    redoStack = [...redoStack, shapes];
    shapes = undoStack[undoStack.length - 1];
    undoStack = undoStack.slice(0, -1);
    picked = null;
  }

  function redo() {
    if (!redoStack.length) return;
    undoStack = [...undoStack, shapes];
    shapes = redoStack[redoStack.length - 1];
    redoStack = redoStack.slice(0, -1);
    picked = null;
  }

  function toggleTool(t: Tool) {
    commitText();
    tool = tool === t ? null : t;
    picked = null;
    cursorStyle = tool ? (tool === "text" ? "text" : "crosshair") : "default";
  }

  /** 选色 / 选大小：同时应用到当前选中的图形 */
  function setStyle(c: string | null, size: number | null) {
    if (c) color = c;
    if (size !== null) sizeIdx = size;
    const s = shapes.find((x) => x.id === picked);
    if (s) {
      const t: Tool = s.t;
      commit(shapes.map((x) => (x.id === s.id ? { ...x, color: c ?? x.color, size: size !== null ? SIZES[t][size] : x.size } : x)));
    }
  }

  async function startText(a: Pt) {
    textEdit = { a, text: "", size: toolSize("text"), color, orig: shapes, existing: false };
    await tick();
    textEl?.focus();
  }

  async function editText(s: Extract<Shape, { t: "text" }>) {
    commitText();
    const orig = shapes;
    shapes = shapes.filter((x) => x.id !== s.id);
    textEdit = { a: s.a, text: s.text, size: s.size, color: s.color, orig, existing: true };
    await tick();
    if (textEl) {
      textEl.innerText = s.text;
      textEl.focus();
      getSelection()?.selectAllChildren(textEl);
    }
  }

  function commitText() {
    const t = textEdit;
    if (!t) return;
    textEdit = null;
    const text = (textEl?.innerText ?? t.text).replace(/\n+$/, "");
    if (!text.trim()) {
      if (t.existing) {
        pushHistory(t.orig);
      }
      return;
    }
    const ctx = layer.getContext("2d")!;
    ctx.save();
    ctx.font = `${t.size}px ${FONT}`;
    const lines = text.split("\n");
    const w = Math.max(...lines.map((l) => ctx.measureText(l).width));
    ctx.restore();
    const shape: Shape = { id: nextId++, t: "text", a: t.a, text, color: t.color, size: t.size, w, h: lines.length * t.size * LINE_HEIGHT };
    pushHistory(t.orig);
    shapes = [...shapes, shape];
  }

  // ---------- 完成 ----------

  async function finishCopy() {
    if (!sel || busy) return;
    busy = true;
    commitText();
    try {
      await invoke("shot_copy", await exportPng());
    } finally {
      release();
    }
  }

  async function finishSave() {
    if (!sel || busy) return;
    busy = true;
    commitText();
    try {
      const png = await exportPng();
      release();
      await invoke("shot_save", png);
    } catch (e) {
      console.error(e);
      release();
    }
  }

  async function finishText(text: string, paste: boolean) {
    try {
      await invoke("shot_text", { text, paste });
    } finally {
      release();
    }
  }

  async function cancel() {
    await invoke("shot_cancel");
    release();
  }

  function startOcr() {
    if (!sel || !info) return;
    commitText();
    tool = null;
    picked = null;
    phase = "ocr";
  }

  function loadOcr(): Promise<OcrResult> {
    const { scale, seq } = info!;
    const r = sel!;
    return invoke<OcrResult>("shot_ocr", {
      seq,
      x: Math.round(r.x * scale),
      y: Math.round(r.y * scale),
      w: Math.max(1, Math.round(r.w * scale)),
      h: Math.max(1, Math.round(r.h * scale)),
    });
  }

  function closeOcr() {
    if (info?.mode === "ocr") cancel();
    else phase = "edit";
  }

  // ---------- 键盘 ----------

  function onKey(e: KeyboardEvent) {
    if (!info || phase === "ocr") return;
    if (textEdit) {
      if (e.key === "Escape" || (e.key === "Enter" && e.metaKey)) {
        e.preventDefault();
        commitText();
      }
      return;
    }
    const k = e.key.toLowerCase();
    if (e.key === "Escape") {
      e.preventDefault();
      cancel();
      return;
    }
    if (phase === "select") {
      if (k === "c" && !e.metaKey) copyColor(e.shiftKey);
      return;
    }
    if (e.metaKey) {
      if (k === "z") {
        e.preventDefault();
        if (e.shiftKey) redo();
        else undo();
      } else if (k === "c") {
        e.preventDefault();
        finishCopy();
      } else if (k === "s") {
        e.preventDefault();
        finishSave();
      }
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      finishCopy();
    } else if ((e.key === "Backspace" || e.key === "Delete") && picked !== null) {
      e.preventDefault();
      commit(shapes.filter((s) => s.id !== picked));
      picked = null;
    } else if (e.key.startsWith("Arrow")) {
      e.preventDefault();
      const step = e.shiftKey ? 10 : 1;
      const dx = e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0;
      const dy = e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0;
      if (picked !== null) commit(shapes.map((s) => (s.id === picked ? move(s, dx, dy) : s)));
      else if (sel && !shapes.length)
        sel = { ...sel, x: clamp(sel.x + dx, 0, info.lw - sel.w), y: clamp(sel.y + dy, 0, info.lh - sel.h) };
    } else if (TOOL_KEYS[k] && !e.altKey && !e.ctrlKey) {
      toggleTool(TOOL_KEYS[k]);
    } else if (k === "o") {
      startOcr();
    }
  }

  onMount(() => {
    begin();
    const un = listen("shot:start", () => begin());
    window.addEventListener("keydown", onKey);
    return () => {
      un.then((f) => f());
      window.removeEventListener("keydown", onKey);
    };
  });

  // ---------- 布局 ----------

  const frame = $derived(sel ?? (phase === "select" ? hover : null));
  const pickedBox = $derived.by(() => {
    const s = shapes.find((x) => x.id === picked);
    return s ? bounds(s) : null;
  });
  const sizeText = $derived(sel && info ? `${Math.round(sel.w * info.scale)} × ${Math.round(sel.h * info.scale)}` : "");

  /** 工具栏放在选区下方；放不下时放上方；再放不下就放在选区内底部 */
  const toolbarPos = $derived.by(() => {
    if (!sel || !info) return { x: 0, y: 0 };
    const h = tool || picked !== null ? 82 : 44;
    const x = clamp(sel.x + sel.w - toolbarW, 8, info.lw - toolbarW - 8);
    let y = sel.y + sel.h + 8;
    if (y + h > info.lh - 8) y = sel.y - h - 8;
    if (y < 8) y = sel.y + sel.h - h - 8;
    return { x, y: clamp(y, 8, info.lh - h - 8) };
  });

  const OCR_W = 480;
  const ocrPos = $derived.by(() => {
    if (!sel || !info) return { x: 0, y: 0, h: 400 };
    const h = Math.min(560, info.lh - 48);
    let x = sel.x + sel.w + 12;
    if (x + OCR_W > info.lw - 12) x = sel.x - OCR_W - 12;
    if (x < 12) x = info.lw - OCR_W - 12;
    return { x, y: clamp(sel.y, 24, info.lh - h - 24), h };
  });

  const magPos = $derived.by(() => {
    if (!info) return { x: 0, y: 0 };
    const [w, h] = [136, 196];
    let x = cursor.x + 18;
    let y = cursor.y + 18;
    if (x + w > info.lw) x = cursor.x - w - 18;
    if (y + h > info.lh) y = cursor.y - h - 18;
    return { x, y };
  });

  const styleTool = $derived<Tool | null>(tool ?? (shapes.find((x) => x.id === picked)?.t as Tool | undefined) ?? null);
</script>

<div class="root" class:hidden={!info}>
  <canvas class="base" bind:this={base} style:width="{info?.lw ?? 0}px" style:height="{info?.lh ?? 0}px"></canvas>
  <canvas class="layer" bind:this={layer} style:width="{info?.lw ?? 0}px" style:height="{info?.lh ?? 0}px"></canvas>

  {#if frame}
    <div class="frame" class:editing={phase !== "select"} style:left="{frame.x}px" style:top="{frame.y}px" style:width="{frame.w}px" style:height="{frame.h}px">
      {#if phase === "edit"}
        {#each HANDLES as h (h)}
          <span class="handle {h}"></span>
        {/each}
      {/if}
    </div>
  {:else if info}
    <div class="dim-all"></div>
  {/if}

  {#if sel && sizeText}
    <div class="size" style:left="{sel.x}px" style:top="{sel.y >= 30 ? sel.y - 28 : sel.y + 6}px">{sizeText}</div>
  {/if}

  {#if pickedBox}
    <div class="picked" style:left="{pickedBox.x - 4}px" style:top="{pickedBox.y - 4}px" style:width="{pickedBox.w + 8}px" style:height="{pickedBox.h + 8}px"></div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="stage"
    style:cursor={phase === "select" ? "crosshair" : cursorStyle}
    onpointerdown={onDown}
    onpointermove={onMove}
    onpointerup={onUp}
    onpointerleave={() => (showMag = false)}
    ondblclick={onDblClick}
    oncontextmenu={onContextMenu}
  ></div>

  {#if textEdit}
    <div
      class="text-edit"
      bind:this={textEl}
      contenteditable="plaintext-only"
      role="textbox"
      tabindex="0"
      style:left="{textEdit.a.x - 3}px"
      style:top="{textEdit.a.y - 3}px"
      style:font-size="{textEdit.size}px"
      style:color={textEdit.color}
      style:font-family={FONT}
    ></div>
  {/if}

  {#if phase === "select" && info}
    <div class="mag" class:off={!showMag} style:left="{magPos.x}px" style:top="{magPos.y}px">
      <canvas bind:this={mag} width="240" height="240"></canvas>
      <div class="mag-info">
        <div>{Math.round(cursor.x * info.scale)}, {Math.round(cursor.y * info.scale)}{sel ? ` · ${sizeText}` : ""}</div>
        <div class="swatch-row"><span class="swatch" style:background={pixel.hex}></span>{pixel.hex}</div>
        <div class="muted">RGB {pixel.rgb}</div>
        <div class="muted">C / ⇧C 复制颜色</div>
      </div>
    </div>
  {/if}

  {#if phase === "select" && info && !sel}
    <div class="tip">
      {info.mode === "ocr" ? "框选要识别文字的区域" : "拖动框选区域，单击选中窗口"} · 右键或 Esc 退出
    </div>
  {/if}

  {#if phase === "edit" && sel}
    <div class="toolbar" bind:clientWidth={toolbarW} style:left="{toolbarPos.x}px" style:top="{toolbarPos.y}px">
      <div class="row">
        {#each TOOLS as t (t.id)}
          <button class:on={tool === t.id} title="{t.name}（{t.key}）" onclick={() => toggleTool(t.id)}>{@html icon(t.id)}</button>
        {/each}
        <span class="sep"></span>
        <button title="撤销（⌘Z）" disabled={!undoStack.length} onclick={undo}>{@html icon("undo")}</button>
        <button title="重做（⇧⌘Z）" disabled={!redoStack.length} onclick={redo}>{@html icon("redo")}</button>
        <span class="sep"></span>
        <button title="识别文字（O）" onclick={startOcr}>{@html icon("ocr")}</button>
        <button title="保存（⌘S）" onclick={finishSave}>{@html icon("save")}</button>
        <span class="sep"></span>
        <button title="退出（Esc）" onclick={cancel}>{@html icon("close")}</button>
        <button class="done" title="复制到剪贴板（↩ / ⌘C / 双击）" onclick={finishCopy}>{@html icon("done")}</button>
      </div>
      {#if styleTool}
        <div class="row style">
          {#if styleTool !== "mosaic"}
            {#each COLORS as c (c)}
              <button class="color" class:on={color === c} style:--c={c} title={c} aria-label={c} onclick={() => setStyle(c, null)}></button>
            {/each}
            <span class="sep"></span>
          {/if}
          {#each [0, 1, 2] as i (i)}
            <button class="dot" class:on={sizeIdx === i} title={["小", "中", "大"][i]} aria-label={["小", "中", "大"][i]} onclick={() => setStyle(null, i)}>
              <span style:width="{6 + i * 4}px" style:height="{6 + i * 4}px"></span>
            </button>
          {/each}
          <span class="hint">{styleTool === "mosaic" ? "块大小" : "⇧ 拖动可画正方形 / 水平线"}</span>
        </div>
      {/if}
    </div>
  {/if}

  {#if phase === "ocr" && sel && info}
    <div class="ocr-panel" style:left="{ocrPos.x}px" style:top="{ocrPos.y}px" style:height="{ocrPos.h}px" style:width="{OCR_W}px">
      <OcrPanel
        load={loadOcr}
        onClose={closeOcr}
        deliver={(text, paste) => finishText(text, paste)}
        closeLabel={info.mode === "ocr" ? "✕ 关闭" : "‹ 返回"}
        compact
      />
    </div>
  {/if}
</div>
