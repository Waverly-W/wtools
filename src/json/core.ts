// JSON 核心：宽松解析、格式化、转义、结构对比
//
// 不用 JSON.parse：
// - 大整数（如 19 位订单号）经过 JSON.parse 会丢精度，这里数字按原文保留
// - 能容忍常见的「不标准 JSON」：注释、尾逗号、单引号、未加引号的键、Python 的 True/False/None
// - 出错时给出准确的行列位置

export type JNode =
  | { t: "obj"; e: [string, JNode][] }
  | { t: "arr"; v: JNode[] }
  | { t: "str"; v: string }
  /** 数字原文（已规范成合法 JSON 写法），不转成 number 以免丢精度 */
  | { t: "num"; v: string }
  | { t: "bool"; v: boolean }
  | { t: "null" };

export interface ParseError {
  message: string;
  offset: number;
  line: number;
  col: number;
}

export type ParseResult =
  | { ok: true; node: JNode; fixes: string[] }
  | { ok: false; error: ParseError };

class Fail {
  message: string;
  offset: number;
  constructor(message: string, offset: number) {
    this.message = message;
    this.offset = offset;
  }
}

const STRICT_NUMBER = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/;
const LOOSE_NUMBER = /[+-]?(?:0[xX][0-9a-fA-F]+|(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)/y;
const IDENT = /[A-Za-z_$][\w$]*/y;

class Parser {
  s: string;
  i = 0;
  fixes = new Set<string>();

  constructor(s: string) {
    this.s = s;
  }

  fail(message: string, at = this.i): never {
    throw new Fail(message, at);
  }

  found(at = this.i): string {
    return at >= this.s.length ? "内容意外结束" : `意外的字符 "${this.s[at]}"`;
  }

  /** 跳过空白和注释 */
  ws() {
    const s = this.s;
    const n = s.length;
    while (this.i < n) {
      const c = s.charCodeAt(this.i);
      if (c === 32 || c === 10 || c === 13 || c === 9 || c === 0xfeff || c === 0xa0) {
        this.i++;
      } else if (c === 47 && s.charCodeAt(this.i + 1) === 47) {
        const end = s.indexOf("\n", this.i);
        this.i = end < 0 ? n : end;
        this.fixes.add("注释");
      } else if (c === 47 && s.charCodeAt(this.i + 1) === 42) {
        const end = s.indexOf("*/", this.i + 2);
        if (end < 0) this.fail("注释没有结束");
        this.i = end + 2;
        this.fixes.add("注释");
      } else {
        break;
      }
    }
  }

  value(): JNode {
    this.ws();
    const c = this.s.charCodeAt(this.i);
    if (c === 123) return this.object();
    if (c === 91) return this.array();
    if (c === 34 || c === 39) return { t: "str", v: this.string() };
    if (c === 45 || c === 43 || c === 46 || (c >= 48 && c <= 57)) return this.number();
    if ((c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === 95 || c === 36) return this.ident();
    this.fail(this.found());
  }

  object(): JNode {
    const s = this.s;
    this.i++;
    const e: [string, JNode][] = [];
    this.ws();
    if (s.charCodeAt(this.i) === 125) {
      this.i++;
      return { t: "obj", e };
    }
    for (;;) {
      this.ws();
      const c = s.charCodeAt(this.i);
      let key: string;
      if (c === 34 || c === 39) {
        key = this.string();
      } else {
        IDENT.lastIndex = this.i;
        const m = IDENT.exec(s) ?? /\d+/y.exec(s.slice(this.i));
        if (!m) this.fail(`需要属性名，${this.found()}`);
        key = m[0];
        this.i += key.length;
        this.fixes.add("未加引号的键");
      }
      this.ws();
      if (s.charCodeAt(this.i) !== 58) this.fail(`属性名后需要冒号，${this.found()}`);
      this.i++;
      e.push([key, this.value()]);
      this.ws();
      const d = s.charCodeAt(this.i);
      if (d === 44) {
        this.i++;
        this.ws();
        if (s.charCodeAt(this.i) === 125) {
          this.fixes.add("尾逗号");
          this.i++;
          return { t: "obj", e };
        }
      } else if (d === 125) {
        this.i++;
        return { t: "obj", e };
      } else {
        this.fail(`需要逗号或 }，${this.found()}`);
      }
    }
  }

  array(): JNode {
    const s = this.s;
    this.i++;
    const v: JNode[] = [];
    this.ws();
    if (s.charCodeAt(this.i) === 93) {
      this.i++;
      return { t: "arr", v };
    }
    for (;;) {
      v.push(this.value());
      this.ws();
      const d = s.charCodeAt(this.i);
      if (d === 44) {
        this.i++;
        this.ws();
        if (s.charCodeAt(this.i) === 93) {
          this.fixes.add("尾逗号");
          this.i++;
          return { t: "arr", v };
        }
      } else if (d === 93) {
        this.i++;
        return { t: "arr", v };
      } else {
        this.fail(`需要逗号或 ]，${this.found()}`);
      }
    }
  }

  string(): string {
    const s = this.s;
    const q = s.charCodeAt(this.i);
    const open = this.i;
    if (q === 39) this.fixes.add("单引号");
    this.i++;
    let out = "";
    let start = this.i;
    for (;;) {
      if (this.i >= s.length) this.fail("字符串没有结束", open);
      const c = s.charCodeAt(this.i);
      if (c === q) {
        out += s.slice(start, this.i);
        this.i++;
        return out;
      }
      if (c === 92) {
        out += s.slice(start, this.i);
        const e = s[this.i + 1];
        switch (e) {
          case '"': case "'": case "\\": case "/": out += e; break;
          case "b": out += "\b"; break;
          case "f": out += "\f"; break;
          case "n": out += "\n"; break;
          case "r": out += "\r"; break;
          case "t": out += "\t"; break;
          case "u": {
            const hex = s.slice(this.i + 2, this.i + 6);
            if (!/^[0-9a-fA-F]{4}$/.test(hex)) this.fail("无效的 \\u 转义");
            out += String.fromCharCode(parseInt(hex, 16));
            this.i += 4;
            break;
          }
          default:
            this.fail(`无效的转义 "\\${e ?? ""}"`);
        }
        this.i += 2;
        start = this.i;
        continue;
      }
      if (c === 10 || c === 13) this.fail("字符串里不能直接换行（需要写成 \\n）");
      if (c < 32) this.fixes.add("字符串中的控制字符");
      this.i++;
    }
  }

  number(): JNode {
    LOOSE_NUMBER.lastIndex = this.i;
    const m = LOOSE_NUMBER.exec(this.s);
    if (!m) this.fail(this.found());
    const raw = m[0];
    const next = this.s.charCodeAt(this.i + raw.length);
    if ((next >= 65 && next <= 90) || (next >= 97 && next <= 122) || next === 95) {
      this.fail(this.found(this.i + raw.length));
    }
    this.i += raw.length;
    if (STRICT_NUMBER.test(raw)) return { t: "num", v: raw };
    this.fixes.add("数字写法");
    return { t: "num", v: normalizeNumber(raw) };
  }

  ident(): JNode {
    IDENT.lastIndex = this.i;
    const word = IDENT.exec(this.s)![0];
    const at = this.i;
    this.i += word.length;
    switch (word) {
      case "true": return { t: "bool", v: true };
      case "false": return { t: "bool", v: false };
      case "null": return { t: "null" };
      case "True": this.fixes.add("Python 写法"); return { t: "bool", v: true };
      case "False": this.fixes.add("Python 写法"); return { t: "bool", v: false };
      case "None": this.fixes.add("Python 写法"); return { t: "null" };
      case "undefined": this.fixes.add("undefined 改为 null"); return { t: "null" };
      case "NaN": case "Infinity": this.fail(`JSON 不支持 ${word}`, at);
    }
    this.fail(`无法识别的内容 "${word}"（字符串需要加引号）`, at);
  }
}

function normalizeNumber(raw: string): string {
  let sign = "";
  let body = raw;
  if (body[0] === "+" || body[0] === "-") {
    sign = body[0] === "-" ? "-" : "";
    body = body.slice(1);
  }
  if (/^0[xX]/.test(body)) return sign + BigInt(body).toString();
  const m = /^(\d*)(?:\.(\d*))?(?:[eE]([+-]?\d+))?$/.exec(body)!;
  const int = (m[1] || "0").replace(/^0+(?=\d)/, "");
  const frac = m[2] ? `.${m[2]}` : "";
  const exp = m[3] ? `e${m[3]}` : "";
  return sign + int + frac + exp;
}

function position(s: string, offset: number) {
  let line = 1;
  let last = -1;
  for (let i = s.indexOf("\n"); i >= 0 && i < offset; i = s.indexOf("\n", i + 1)) {
    line++;
    last = i;
  }
  return { line, col: offset - last };
}

export function parse(text: string): ParseResult {
  const p = new Parser(text);
  try {
    p.ws();
    if (p.i >= text.length) p.fail("内容为空");
    const node = p.value();
    p.ws();
    if (p.i < text.length) p.fail(`JSON 已结束，后面还有多余内容`);
    return { ok: true, node, fixes: [...p.fixes] };
  } catch (e) {
    if (e instanceof Fail) {
      return { ok: false, error: { message: e.message, offset: e.offset, ...position(text, e.offset) } };
    }
    if (e instanceof RangeError) {
      return { ok: false, error: { message: "嵌套层级过深", offset: 0, line: 1, col: 1 } };
    }
    throw e;
  }
}

const looksLikeJson = (s: string) => {
  const t = s.trim();
  return (t[0] === "{" && t.at(-1) === "}") || (t[0] === "[" && t.at(-1) === "]");
};

/**
 * 格式化时用的解析：除了宽松解析，还自动处理被转义过的 JSON
 * - `"{\"a\":1}"`（整体是一个字符串，内容是 JSON）
 * - `{\"a\":1}`（从代码 / 日志里复制出来的转义文本）
 */
export function smartParse(text: string): ParseResult {
  const r = parse(text);
  if (r.ok) {
    if (r.node.t === "str" && looksLikeJson(r.node.v)) {
      const inner = parse(r.node.v);
      if (inner.ok) return { ok: true, node: inner.node, fixes: [...r.fixes, ...inner.fixes, "已去转义"] };
    }
    return r;
  }
  if (text.includes('\\"')) {
    const inner = parse(unescapeText(text));
    if (inner.ok) return { ok: true, node: inner.node, fixes: [...inner.fixes, "已去转义"] };
  }
  return r;
}

// ---------- 输出 ----------

export interface FormatOptions {
  /** 缩进字符串，空字符串表示压缩成一行 */
  indent: string;
  sortKeys?: boolean;
  /** 非 ASCII 字符输出为 \uXXXX */
  asciiOnly?: boolean;
}

const IDENT_KEY = /^[A-Za-z_$][\w$]*$/;

export function childPath(parent: string, key: string | number): string {
  if (typeof key === "number") return `${parent}[${key}]`;
  return IDENT_KEY.test(key) ? `${parent}.${key}` : `${parent}[${JSON.stringify(key)}]`;
}

function quote(s: string, ascii?: boolean): string {
  const q = JSON.stringify(s);
  return ascii ? encodeUnicode(q) : q;
}

function sortedEntries(e: [string, JNode][]): [string, JNode][] {
  return [...e].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
}

/**
 * 序列化。`lines` 不为空时记录每个路径所在的行号（从 0 开始），用于对比结果跳转。
 */
export function stringify(node: JNode, opts: FormatOptions, lines?: Map<string, number>): string {
  const { indent, sortKeys, asciiOnly } = opts;
  const out: string[] = [];
  const pads: string[] = [""];
  const pad = (d: number) => (pads[d] ??= pads[d - 1] + indent);
  let line = 0;

  const write = (n: JNode, path: string, depth: number) => {
    lines?.set(path, line);
    switch (n.t) {
      case "str": out.push(quote(n.v, asciiOnly)); return;
      case "num": out.push(n.v); return;
      case "bool": out.push(n.v ? "true" : "false"); return;
      case "null": out.push("null"); return;
      case "arr": {
        if (!n.v.length) { out.push("[]"); return; }
        if (!indent) {
          out.push("[");
          n.v.forEach((c, i) => {
            if (i) out.push(",");
            write(c, lines ? childPath(path, i) : "", depth + 1);
          });
          out.push("]");
          return;
        }
        out.push("[\n");
        line++;
        const last = n.v.length - 1;
        n.v.forEach((c, i) => {
          out.push(pad(depth + 1));
          write(c, lines ? childPath(path, i) : "", depth + 1);
          out.push(i < last ? ",\n" : "\n");
          line++;
        });
        out.push(pad(depth), "]");
        return;
      }
      case "obj": {
        if (!n.e.length) { out.push("{}"); return; }
        const entries = sortKeys ? sortedEntries(n.e) : n.e;
        if (!indent) {
          out.push("{");
          entries.forEach(([k, c], i) => {
            if (i) out.push(",");
            out.push(quote(k, asciiOnly), ":");
            write(c, lines ? childPath(path, k) : "", depth + 1);
          });
          out.push("}");
          return;
        }
        out.push("{\n");
        line++;
        const last = entries.length - 1;
        entries.forEach(([k, c], i) => {
          out.push(pad(depth + 1), quote(k, asciiOnly), ": ");
          write(c, lines ? childPath(path, k) : "", depth + 1);
          out.push(i < last ? ",\n" : "\n");
          line++;
        });
        out.push(pad(depth), "}");
        return;
      }
    }
  };
  write(node, "$", 0);
  return out.join("");
}

/** 把值为 JSON 文本的字符串展开成真正的对象 / 数组（可多层） */
export function expandNested(node: JNode): { node: JNode; count: number } {
  let count = 0;
  const walk = (n: JNode): JNode => {
    switch (n.t) {
      case "obj": return { t: "obj", e: n.e.map(([k, v]) => [k, walk(v)]) };
      case "arr": return { t: "arr", v: n.v.map(walk) };
      case "str": {
        if (!looksLikeJson(n.v)) return n;
        const r = parse(n.v);
        if (!r.ok) return n;
        count++;
        return walk(r.node);
      }
      default: return n;
    }
  };
  return { node: walk(node), count };
}

export function summary(node: JNode): string {
  switch (node.t) {
    case "obj": return `对象 · ${node.e.length} 个键`;
    case "arr": return `数组 · ${node.v.length} 项`;
    case "str": return "字符串";
    case "num": return "数字";
    case "bool": return "布尔值";
    case "null": return "null";
  }
}

// ---------- 转义 ----------

/** 转义：变成可以放进 JSON / 代码字符串里的文本（不含两端引号） */
export function escapeText(s: string): string {
  return JSON.stringify(s).slice(1, -1);
}

/** 去转义：`\"` → `"`、`\n` → 换行等；两端有引号会一并去掉 */
export function unescapeText(s: string): string {
  let t = s.trim();
  if (t.length >= 2 && (t[0] === '"' || t[0] === "'") && t.at(-1) === t[0]) t = t.slice(1, -1);
  return t.replace(/\\(u[0-9a-fA-F]{4}|[\s\S])/g, (_, e: string) => {
    if (e.length === 5) return String.fromCharCode(parseInt(e.slice(1), 16));
    switch (e) {
      case "n": return "\n";
      case "r": return "\r";
      case "t": return "\t";
      case "b": return "\b";
      case "f": return "\f";
      default: return e;
    }
  });
}

/** `中文` → 中文 */
export function decodeUnicode(s: string): string {
  return s.replace(/\\u([0-9a-fA-F]{4})/g, (_, h: string) => String.fromCharCode(parseInt(h, 16)));
}

/** 中文 → `中文` */
export function encodeUnicode(s: string): string {
  return s.replace(/[^\x00-\x7f]/g, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}`);
}

// ---------- 对比 ----------

export type DiffKind = "added" | "removed" | "changed";

export interface DiffEntry {
  path: string;
  kind: DiffKind;
  left?: JNode;
  right?: JNode;
}

function numEqual(a: string, b: string): boolean {
  if (a === b) return true;
  if (/^-?\d+$/.test(a) && /^-?\d+$/.test(b)) return BigInt(a) === BigInt(b);
  return Number(a) === Number(b);
}

/**
 * 结构对比：对象按键对比（与键的顺序无关），数组按下标对比。
 * 超过 `limit` 条后停止，避免两份完全不同的大文件卡住界面。
 */
export function diff(a: JNode, b: JNode, limit = 2000): DiffEntry[] {
  const out: DiffEntry[] = [];
  const walk = (x: JNode, y: JNode, path: string) => {
    if (out.length >= limit) return;
    if (x.t !== y.t) {
      out.push({ path, kind: "changed", left: x, right: y });
      return;
    }
    switch (x.t) {
      case "obj": {
        const ym = new Map((y as typeof x).e);
        const xm = new Map(x.e);
        for (const [k, v] of xm) {
          const p = childPath(path, k);
          const w = ym.get(k);
          if (w === undefined) out.push({ path: p, kind: "removed", left: v });
          else walk(v, w, p);
        }
        for (const [k, w] of ym) {
          if (!xm.has(k)) out.push({ path: childPath(path, k), kind: "added", right: w });
        }
        return;
      }
      case "arr": {
        const yv = (y as typeof x).v;
        const n = Math.max(x.v.length, yv.length);
        for (let i = 0; i < n && out.length < limit; i++) {
          const p = childPath(path, i);
          if (i >= yv.length) out.push({ path: p, kind: "removed", left: x.v[i] });
          else if (i >= x.v.length) out.push({ path: p, kind: "added", right: yv[i] });
          else walk(x.v[i], yv[i], p);
        }
        return;
      }
      case "str":
        if (x.v !== (y as typeof x).v) out.push({ path, kind: "changed", left: x, right: y });
        return;
      case "num":
        if (!numEqual(x.v, (y as typeof x).v)) out.push({ path, kind: "changed", left: x, right: y });
        return;
      case "bool":
        if (x.v !== (y as typeof x).v) out.push({ path, kind: "changed", left: x, right: y });
        return;
      case "null":
        return;
    }
  };
  walk(a, b, "$");
  return out;
}

/** 对比列表里显示的值预览 */
export function preview(node: JNode | undefined, max = 60): string {
  if (!node) return "";
  if (node.t === "obj" && node.e.length > 3) return `{…${node.e.length} 个键}`;
  if (node.t === "arr" && node.v.length > 5) return `[…${node.v.length} 项]`;
  const s = stringify(node, { indent: "" });
  return s.length > max ? `${s.slice(0, max)}…` : s;
}
