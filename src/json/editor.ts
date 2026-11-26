// CodeMirror 编辑器配置：只挑需要的扩展，不用 basicSetup（少加载自动补全等用不到的代码）

import { EditorState, type Extension } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  HighlightStyle,
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import { json } from "@codemirror/lang-json";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { lintGutter, setDiagnostics } from "@codemirror/lint";
import { tags } from "@lezer/highlight";
import type { ParseResult } from "./core";

const highlight = HighlightStyle.define([
  { tag: tags.propertyName, color: "var(--j-key)" },
  { tag: tags.string, color: "var(--j-str)" },
  { tag: tags.number, color: "var(--j-num)" },
  { tag: [tags.bool, tags.null], color: "var(--j-kw)" },
  { tag: [tags.brace, tags.squareBracket, tags.separator], color: "var(--text-secondary)" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "var(--fs-md)", backgroundColor: "var(--surface)", color: "var(--text)" },
  ".cm-scroller": {
    fontFamily: "var(--font-mono)",
    lineHeight: "1.6",
  },
  ".cm-content": { caretColor: "var(--text)", userSelect: "text", WebkitUserSelect: "text" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--text)" },
  "&.cm-focused": { outline: "none" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--text-secondary)", border: "none" },
  ".cm-activeLine": { backgroundColor: "var(--j-active-line)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--j-active-line)" },
  ".cm-selectionBackground, &.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground":
    { backgroundColor: "var(--j-selection)" },
  ".cm-selectionMatch": { backgroundColor: "var(--j-match)" },
  ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": { backgroundColor: "var(--j-match)", outline: "none" },
  ".cm-foldPlaceholder": { backgroundColor: "var(--hover)", border: "none", color: "var(--text-secondary)" },
  ".cm-panels": { backgroundColor: "var(--bg)", color: "var(--text)" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--border)" },
  ".cm-textfield": { backgroundColor: "var(--surface)", color: "var(--text)", border: "1px solid var(--border)" },
  ".cm-button": { backgroundImage: "none", backgroundColor: "var(--surface)", color: "var(--text)", border: "1px solid var(--border)" },
  ".cm-tooltip": { backgroundColor: "var(--surface)", border: "1px solid var(--border)" },
  ".cm-diagnostic-error": { borderLeftColor: "var(--danger)" },
});

export function baseExtensions(): Extension[] {
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    foldGutter(),
    lintGutter(),
    highlightSpecialChars(),
    history(),
    drawSelection(),
    indentOnInput(),
    bracketMatching(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    syntaxHighlighting(highlight),
    keymap.of([...defaultKeymap, ...historyKeymap, ...foldKeymap, ...searchKeymap, indentWithTab]),
    EditorState.tabSize.of(2),
    json(),
    theme,
  ];
}

/** 替换全部内容（可 ⌘Z 撤销） */
export function setDoc(view: EditorView, text: string, userEvent = "input.replace") {
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: text },
    selection: { anchor: 0 },
    scrollIntoView: true,
    userEvent,
  });
}

/** 把解析错误显示为红色波浪线 + 行号旁的标记 */
export function showError(view: EditorView, r: ParseResult | null) {
  const len = view.state.doc.length;
  const diags =
    r && !r.ok && len
      ? [
          {
            from: Math.min(r.error.offset, len - 1),
            to: Math.min(r.error.offset + 1, len),
            severity: "error" as const,
            message: r.error.message,
          },
        ]
      : [];
  view.dispatch(setDiagnostics(view.state, diags));
}

export function jumpTo(view: EditorView, offset: number) {
  const pos = Math.min(offset, view.state.doc.length);
  view.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
  view.focus();
}

/** 粘贴时是否替换了全部内容（原来为空或全选后粘贴）——此时自动格式化 */
export function pastedWhole(u: import("@codemirror/view").ViewUpdate): boolean {
  if (!u.transactions.some((tr) => tr.isUserEvent("input.paste"))) return false;
  const sel = u.startState.selection.main;
  return u.startState.doc.length === 0 || (sel.from === 0 && sel.to === u.startState.doc.length);
}

/** 简单的本地存储（隐私模式下可能抛错，失败时忽略） */
export const store = {
  get(key: string, fallback = ""): string {
    try {
      return localStorage.getItem(`wtools.json.${key}`) ?? fallback;
    } catch {
      return fallback;
    }
  },
  set(key: string, value: string) {
    try {
      // 太大的内容不存，避免超出存储配额
      if (value.length > 2_000_000) localStorage.removeItem(`wtools.json.${key}`);
      else localStorage.setItem(`wtools.json.${key}`, value);
    } catch {
      /* 忽略 */
    }
  },
};

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number) {
  let t: ReturnType<typeof setTimeout> | undefined;
  return (...a: A) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
}

export function formatSize(chars: number): string {
  if (chars < 1024) return `${chars} 字符`;
  if (chars < 1024 * 1024) return `${(chars / 1024).toFixed(1)} K 字符`;
  return `${(chars / 1024 / 1024).toFixed(1)} M 字符`;
}
