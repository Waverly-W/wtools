<script lang="ts">
  import { onMount } from "svelte";
  import { MergeView } from "@codemirror/merge";
  import { EditorView } from "@codemirror/view";
  import {
    diff,
    preview,
    smartParse,
    stringify,
    type DiffEntry,
    type JNode,
    type ParseResult,
  } from "./core";
  import { baseExtensions, debounce, pastedWhole, setDoc, showError, store } from "./editor";

  let { notify }: { notify: (msg: string, error?: boolean) => void } = $props();

  type Side = "a" | "b";
  type Ok = Extract<ParseResult, { ok: true }>;

  let host: HTMLDivElement;
  let mv: MergeView;

  let sortKeys = $state(store.get("diffSort", "1") === "1");
  let collapse = $state(store.get("diffCollapse") === "1");
  let listOpen = $state(store.get("diffList", "1") === "1");
  let ra = $state<ParseResult | null>(null);
  let rb = $state<ParseResult | null>(null);
  let entries = $state<DiffEntry[]>([]);
  let textEqual = $state(false);
  let selected = $state(-1);

  const COLLAPSE = { margin: 3, minSize: 6 };
  const LIMIT = 2000;

  $effect(() => store.set("diffSort", sortKeys ? "1" : "0"));
  $effect(() => store.set("diffList", listOpen ? "1" : "0"));
  $effect(() => {
    store.set("diffCollapse", collapse ? "1" : "0");
    mv?.reconfigure({ collapseUnchanged: collapse ? COLLAPSE : undefined });
  });

  const ed = (s: Side) => (s === "a" ? mv.a : mv.b);
  const text = (s: Side) => ed(s).state.doc.toString();
  /** 统一格式：2 空格缩进，可选键排序；同时记录每个路径所在行 */
  const canon = (node: JNode) => {
    const lines = new Map<string, number>();
    return { text: stringify(node, { indent: "  ", sortKeys }, lines), lines };
  };

  const recompute = debounce(() => {
    const A = text("a");
    const B = text("b");
    ra = A.trim() ? smartParse(A) : null;
    rb = B.trim() ? smartParse(B) : null;
    showError(mv.a, ra);
    showError(mv.b, rb);
    textEqual = A === B;
    entries = ra?.ok && rb?.ok ? diff(ra.node, rb.node, LIMIT) : [];
    selected = -1;
  }, 250);
  const save = debounce(() => {
    store.set("diffA", text("a"));
    store.set("diffB", text("b"));
  }, 600);

  /** 把一侧整理成统一格式，返回解析结果 */
  function normalize(s: Side): Ok | null {
    const r = smartParse(text(s));
    if (!r.ok) return null;
    const c = canon(r.node).text;
    if (c !== text(s)) setDoc(ed(s), c, "input.format");
    return r;
  }

  function formatBoth() {
    const bad: string[] = [];
    for (const s of ["a", "b"] as Side[]) {
      if (!text(s).trim()) continue;
      if (!normalize(s)) bad.push(s === "a" ? "左侧" : "右侧");
    }
    if (bad.length) notify(`${bad.join("、")}不是有效的 JSON`, true);
  }

  function swap() {
    const A = text("a");
    setDoc(mv.a, text("b"));
    setDoc(mv.b, A);
  }

  function clear() {
    setDoc(mv.a, "");
    setDoc(mv.b, "");
  }

  /** 点击差异项：两侧整理成统一格式后，滚动并选中对应行 */
  function jump(i: number) {
    const e = entries[i];
    const a = normalize("a");
    const b = normalize("b");
    if (!a || !b) return;
    selected = i;
    const la = canon(a.node).lines.get(e.path);
    const lb = canon(b.node).lines.get(e.path);
    const mark = (view: EditorView, line: number | undefined, scroll: boolean) => {
      if (line === undefined) return;
      const l = view.state.doc.line(line + 1);
      view.dispatch({
        selection: { anchor: l.from, head: l.to },
        effects: scroll ? EditorView.scrollIntoView(l.from, { y: "center" }) : [],
      });
    };
    mark(mv.a, la, e.kind !== "added");
    mark(mv.b, lb, e.kind === "added");
  }

  const tidy = (t: string) => {
    const r = smartParse(t);
    return r.ok ? canon(r.node).text : t;
  };

  /** 载入外部内容：同时给了右侧就左右各放一份；否则先填左侧，左侧有内容再填右侧 */
  export function load(t: string, explicit: boolean, right?: string | null) {
    if (right != null) {
      setDoc(mv.a, tidy(t));
      setDoc(mv.b, tidy(right));
      return;
    }
    const next = tidy(t);
    if (!explicit && (next === text("a") || next === text("b"))) return;
    if (!text("a").trim()) setDoc(mv.a, next);
    else if (!text("b").trim() || explicit) setDoc(mv.b, next);
    else return;
    notify(explicit ? "已载入" : "已读取剪贴板中的 JSON（⌘Z 撤销）");
  }

  export function focus() {
    mv?.a.focus();
  }

  const kindLabel = { added: "新增", removed: "删除", changed: "修改" };
  const counts = $derived({
    added: entries.filter((e) => e.kind === "added").length,
    removed: entries.filter((e) => e.kind === "removed").length,
    changed: entries.filter((e) => e.kind === "changed").length,
  });

  onMount(() => {
    const ext = (s: Side) => [
      ...baseExtensions(),
      EditorView.updateListener.of((u) => {
        if (!u.docChanged) return;
        recompute();
        save();
        // 粘贴替换了整侧内容：自动整理成统一格式，避免格式差异干扰对比
        if (pastedWhole(u)) setTimeout(() => normalize(s));
      }),
    ];
    mv = new MergeView({
      a: { doc: store.get("diffA"), extensions: ext("a") },
      b: { doc: store.get("diffB"), extensions: ext("b") },
      parent: host,
      highlightChanges: true,
      gutter: true,
      collapseUnchanged: collapse ? COLLAPSE : undefined,
      diffConfig: { timeout: 2000 },
    });
    recompute();
    mv.a.focus();
    return () => mv.destroy();
  });
</script>

<div class="tab">
  <div class="bar toolbar">
    <button class="btn sm primary" onclick={formatBoth} title="两侧整理成相同格式后对比">格式化两侧</button>
    <label class="check" title="格式化时按键名排序，键顺序不同不算差异"><input type="checkbox" bind:checked={sortKeys} />键排序</label>
    <label class="check"><input type="checkbox" bind:checked={collapse} />折叠相同内容</label>
    <span class="vsep"></span>
    <button class="btn sm" onclick={swap}>交换左右</button>
    <button class="btn sm" onclick={clear}>清空</button>
    <span class="spacer"></span>
    {#if !ra || !rb}
      <span class="muted">在左右两侧分别粘贴 JSON</span>
    {:else if !ra.ok || !rb.ok}
      <span class="err">
        {!ra.ok ? `左侧第 ${ra.error.line} 行：${ra.error.message}` : ""}
        {!rb.ok ? `右侧第 ${rb.error.line} 行：${rb.error.message}` : ""}
      </span>
    {:else if !entries.length}
      <span class="ok">✓ {textEqual ? "两侧完全相同" : "内容一致（只有格式或键顺序不同）"}</span>
    {:else}
      <span>
        {entries.length >= LIMIT ? `超过 ${LIMIT}` : entries.length} 处差异：
        <span class="k-added">新增 {counts.added}</span> ·
        <span class="k-removed">删除 {counts.removed}</span> ·
        <span class="k-changed">修改 {counts.changed}</span>
      </span>
    {/if}
  </div>

  <div class="labels"><span>左侧（原始）</span><span>右侧（对比）</span></div>
  <div class="merge" bind:this={host}></div>

  {#if entries.length}
    <div class="diff-list" class:open={listOpen}>
      <button class="diff-head" onclick={() => (listOpen = !listOpen)}>
        {listOpen ? "▾" : "▸"} 差异明细 <span class="muted">（按路径列出，点击跳转；对象不区分键顺序，数组按下标对比）</span>
      </button>
      {#if listOpen}
        <div class="rows">
          {#each entries as e, i (e.path + e.kind)}
            <button class="row" class:sel={i === selected} onclick={() => jump(i)}>
              <span class="kind k-{e.kind}">{kindLabel[e.kind]}</span>
              <span class="path">{e.path}</span>
              <span class="vals">
                {#if e.kind === "changed"}
                  <span class="old">{preview(e.left)}</span> → <span class="new">{preview(e.right)}</span>
                {:else if e.kind === "removed"}
                  <span class="old">{preview(e.left)}</span>
                {:else}
                  <span class="new">{preview(e.right)}</span>
                {/if}
              </span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>
