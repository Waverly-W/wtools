<script lang="ts">
  import { onMount } from "svelte";
  import { Compartment, EditorState, Prec } from "@codemirror/state";
  import { EditorView, keymap } from "@codemirror/view";
  import { api } from "../lib/api";
  import {
    decodeUnicode,
    encodeUnicode,
    escapeText,
    expandNested,
    parse,
    smartParse,
    stringify,
    summary,
    unescapeText,
    type ParseResult,
  } from "./core";
  import {
    baseExtensions,
    debounce,
    formatSize,
    jumpTo,
    pastedWhole,
    setDoc,
    showError,
    store,
  } from "./editor";

  let { notify }: { notify: (msg: string, error?: boolean) => void } = $props();

  let host: HTMLDivElement;
  let view: EditorView;
  const wrapConf = new Compartment();

  let indent = $state(store.get("indent", "2"));
  let sortKeys = $state(store.get("sortKeys") === "1");
  let wrap = $state(store.get("wrap") === "1");
  let result = $state<ParseResult | null>(null);
  let size = $state(0);
  let cursor = $state({ line: 1, col: 1 });

  $effect(() => store.set("indent", indent));
  $effect(() => store.set("sortKeys", sortKeys ? "1" : "0"));
  $effect(() => {
    store.set("wrap", wrap ? "1" : "0");
    view?.dispatch({ effects: wrapConf.reconfigure(wrap ? EditorView.lineWrapping : []) });
  });

  const indentStr = () => (indent === "tab" ? "\t" : " ".repeat(Number(indent)));
  const doc = () => view.state.doc.toString();
  const pretty = (r: Extract<ParseResult, { ok: true }>, compact = false) =>
    stringify(r.node, { indent: compact ? "" : indentStr(), sortKeys });

  const check = debounce(() => {
    const text = doc();
    size = text.length;
    result = text.trim() ? smartParse(text) : null;
    showError(view, result);
  }, 250);
  const save = debounce(() => store.set("format", doc()), 600);

  /** 解析当前内容，失败时定位到出错位置 */
  function parsed(): Extract<ParseResult, { ok: true }> | null {
    const text = doc();
    if (!text.trim()) {
      notify("请先粘贴 JSON");
      return null;
    }
    const r = smartParse(text);
    if (!r.ok) {
      result = r;
      showError(view, r);
      jumpTo(view, r.error.offset);
      notify(`第 ${r.error.line} 行第 ${r.error.col} 列：${r.error.message}`, true);
      return null;
    }
    return r;
  }

  const fixedMsg = (fixes: string[]) => (fixes.length ? `已自动修正：${fixes.join("、")}` : "");

  function format(compact = false) {
    const r = parsed();
    if (!r) return;
    setDoc(view, pretty(r, compact));
    const msg = fixedMsg(r.fixes);
    if (msg) notify(msg);
  }

  function expand() {
    const r = parsed();
    if (!r) return;
    const { node, count } = expandNested(r.node);
    if (!count) return notify("没有找到值为 JSON 文本的字符串");
    setDoc(view, stringify(node, { indent: indentStr(), sortKeys }));
    notify(`已展开 ${count} 处嵌套 JSON`);
  }

  function escape() {
    const text = doc();
    if (!text) return;
    const r = smartParse(text);
    setDoc(view, escapeText(r.ok ? pretty(r, true) : text));
    notify(r.ok ? "已压缩并转义" : "已转义");
  }

  function unescape() {
    const text = doc();
    if (!text) return;
    const u = unescapeText(text);
    const r = parse(u);
    setDoc(view, r.ok ? pretty(r) : u);
    notify(r.ok ? "已去转义并格式化" : "已去转义");
  }

  function unicode(decode: boolean) {
    const text = doc();
    const out = decode ? decodeUnicode(text) : encodeUnicode(text);
    if (out === text) return notify(decode ? "没有 \\uXXXX 编码的字符" : "没有非 ASCII 字符");
    setDoc(view, out);
  }

  async function copy() {
    const text = doc();
    if (!text) return;
    await api.clipPasteText(text, false);
    notify("已复制");
  }

  /** 载入外部内容。`explicit` 为 false 表示来自剪贴板自动读取 */
  export function load(text: string, explicit: boolean) {
    const r = smartParse(text);
    const next = r.ok ? pretty(r) : text;
    if (!explicit && (next === doc() || text === doc())) return;
    setDoc(view, next);
    if (!explicit) notify("已读取剪贴板中的 JSON（⌘Z 撤销）");
    else if (r.ok) notify(fixedMsg(r.fixes) || "已载入");
  }

  export function focus() {
    view?.focus();
  }

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: store.get("format"),
        extensions: [
          ...baseExtensions(),
          wrapConf.of(wrap ? EditorView.lineWrapping : []),
          Prec.high(
            keymap.of([
              { key: "Mod-Enter", run: () => (format(), true) },
              { key: "Shift-Mod-Enter", run: () => (format(true), true) },
            ]),
          ),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) {
              check();
              save();
              // 粘贴替换了全部内容且能解析：自动格式化（⌘Z 可回到粘贴的原文）
              if (pastedWhole(u)) {
                setTimeout(() => {
                  const r = smartParse(doc());
                  if (r.ok) {
                    setDoc(view, pretty(r));
                    const msg = fixedMsg(r.fixes);
                    if (msg) notify(msg);
                  }
                });
              }
            }
            if (u.docChanged || u.selectionSet) {
              const head = u.state.selection.main.head;
              const line = u.state.doc.lineAt(head);
              cursor = { line: line.number, col: head - line.from + 1 };
            }
          }),
        ],
      }),
    });
    check();
    view.focus();
    return () => view.destroy();
  });
</script>

<div class="tab">
  <div class="bar toolbar">
    <button class="btn sm primary" onclick={() => format()} title="⌘↩">格式化</button>
    <button class="btn sm" onclick={() => format(true)} title="⇧⌘↩">压缩</button>
    <select class="select sm" bind:value={indent} title="缩进">
      <option value="2">2 空格</option>
      <option value="4">4 空格</option>
      <option value="tab">Tab</option>
    </select>
    <label class="check" title="格式化时按键名排序"><input type="checkbox" bind:checked={sortKeys} />键排序</label>
    <span class="vsep"></span>
    <button class="btn sm" onclick={expand} title="把值为 JSON 文本的字符串展开成对象">展开嵌套</button>
    <button class="btn sm" onclick={escape} title="压缩后转义成可以放进字符串的文本">转义</button>
    <button class="btn sm" onclick={unescape} title={'去掉 \\" 等转义并格式化'}>去转义</button>
    <button class="btn sm" onclick={() => unicode(true)} title={"把 \\uXXXX 编码还原成文字"}>Unicode→中文</button>
    <button class="btn sm" onclick={() => unicode(false)} title={"把非 ASCII 字符编码成 \\uXXXX"}>中文→Unicode</button>
    <span class="spacer"></span>
    <label class="check"><input type="checkbox" bind:checked={wrap} />自动换行</label>
    <button class="btn sm" onclick={copy}>复制</button>
    <button class="btn sm" onclick={() => setDoc(view, "")}>清空</button>
  </div>

  <div class="editor" bind:this={host}></div>

  <div class="statusbar status">
    {#if !result}
      <span class="muted">粘贴 JSON 会自动格式化；支持注释、尾逗号、单引号等不标准写法，以及被转义过的 JSON</span>
    {:else if !result.ok}
      {@const e = result.error}
      <button class="link err" onclick={() => jumpTo(view, e.offset)}>
        ✗ 第 {e.line} 行第 {e.col} 列：{e.message}
      </button>
    {:else if result.fixes.length}
      <span class="warn-text">⚠ 不是标准 JSON（{result.fixes.join("、")}），格式化时会自动修正</span>
      <span class="muted">· {summary(result.node)}</span>
    {:else}
      <span class="ok">✓ 合法 JSON</span>
      <span class="muted">· {summary(result.node)}</span>
    {/if}
    <span class="spacer"></span>
    <span class="muted">第 {cursor.line} 行，第 {cursor.col} 列 · {formatSize(size)}</span>
  </div>
</div>
