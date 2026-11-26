// 运行：node --test src/json/core.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  decodeUnicode,
  diff,
  encodeUnicode,
  escapeText,
  expandNested,
  parse,
  smartParse,
  stringify,
  unescapeText,
  type JNode,
} from "./core.ts";

const ok = (text: string): JNode => {
  const r = parse(text);
  if (!r.ok) throw new Error(`${r.error.message} @${r.error.line}:${r.error.col}`);
  return r.node;
};
const fmt = (text: string, indent = "  ", sortKeys = false) =>
  stringify(ok(text), { indent, sortKeys });

test("标准 JSON 与 JSON.stringify 输出一致", () => {
  const src = '{"a":[1,2.5,-3.25,{"b":null}],"c":"中文\\n\\"q\\"","d":true,"e":{}, "f":[]}';
  assert.equal(fmt(src), JSON.stringify(JSON.parse(src), null, 2));
  assert.equal(fmt(src, ""), JSON.stringify(JSON.parse(src)));
  assert.equal(fmt(src, "\t"), JSON.stringify(JSON.parse(src), null, "\t"));
});

test("大整数不丢精度", () => {
  assert.equal(fmt('{"id":1234567890123456789}', ""), '{"id":1234567890123456789}');
});

test("宽松语法：注释、尾逗号、单引号、未加引号的键、Python 写法", () => {
  const r = parse(`{
    // 注释
    name: 'wtools', /* 块注释 */
    list: [1, 2,],
    ok: True, none: None,
  }`);
  assert.ok(r.ok);
  if (!r.ok) return;
  assert.equal(stringify(r.node, { indent: "" }), '{"name":"wtools","list":[1,2],"ok":true,"none":null}');
  for (const f of ["注释", "尾逗号", "单引号", "未加引号的键", "Python 写法"]) assert.ok(r.fixes.includes(f), f);
});

test("非标准数字规范化", () => {
  assert.equal(fmt("[+1, .5, 5., 007, 0x1F, -.25]", ""), "[1,0.5,5,7,31,-0.25]");
});

test("错误位置", () => {
  const r = parse('{\n  "a": 1\n  "b": 2\n}');
  assert.ok(!r.ok);
  if (r.ok) return;
  assert.equal(r.error.line, 3);
  assert.equal(r.error.col, 3);
  assert.match(r.error.message, /逗号/);

  const r2 = parse('{"a": "abc');
  assert.ok(!r2.ok && r2.error.message.includes("没有结束"));
  const r3 = parse('{"a": 1} x');
  assert.ok(!r3.ok && r3.error.message.includes("多余"));
  const r4 = parse("{a: hello}");
  assert.ok(!r4.ok && r4.error.message.includes("hello"));
});

test("排序键", () => {
  assert.equal(fmt('{"b":1,"a":{"d":1,"c":2}}', "", true), '{"a":{"c":2,"d":1},"b":1}');
});

test("非 ASCII 输出为 \\u", () => {
  assert.equal(stringify(ok('{"名":"值"}'), { indent: "", asciiOnly: true }), '{"\\u540d":"\\u503c"}');
});

test("自动去转义", () => {
  for (const src of ['"{\\"a\\":1}"', '{\\"a\\":1}']) {
    const r = smartParse(src);
    assert.ok(r.ok, src);
    if (!r.ok) return;
    assert.equal(stringify(r.node, { indent: "" }), '{"a":1}');
    assert.ok(r.fixes.includes("已去转义"));
  }
});

test("展开嵌套 JSON 字符串", () => {
  const { node, count } = expandNested(ok('{"data":"{\\"list\\":\\"[1,2]\\"}","s":"{not json"}'));
  assert.equal(count, 2);
  assert.equal(stringify(node, { indent: "" }), '{"data":{"list":[1,2]},"s":"{not json"}');
});

test("转义 / 去转义 / Unicode", () => {
  const raw = '{"a":"x\ny"}';
  assert.equal(escapeText(raw), '{\\"a\\":\\"x\\ny\\"}');
  assert.equal(unescapeText(escapeText(raw)), raw);
  assert.equal(unescapeText(`"${escapeText(raw)}"`), raw);
  assert.equal(decodeUnicode("\\u4e2d\\u6587ok"), "中文ok");
  assert.equal(encodeUnicode("中文ok"), "\\u4e2d\\u6587ok");
  assert.equal(decodeUnicode(encodeUnicode("😀")), "😀");
});

test("记录路径所在行号", () => {
  const lines = new Map<string, number>();
  stringify(ok('{"a":{"b":[1,{"c d":2}]}}'), { indent: "  " }, lines);
  assert.equal(lines.get("$"), 0);
  assert.equal(lines.get("$.a"), 1);
  assert.equal(lines.get("$.a.b"), 2);
  assert.equal(lines.get("$.a.b[0]"), 3);
  assert.equal(lines.get('$.a.b[1]["c d"]'), 5);
});

test("结构对比", () => {
  const d = diff(
    ok('{"a":1,"b":[1,2,3],"c":{"x":1},"n":1.0,"big":12345678901234567890}'),
    ok('{"c":{"x":2,"y":0},"a":1,"b":[1,3],"n":1,"big":12345678901234567891,"new":true}'),
  );
  const brief = d.map((e) => `${e.kind} ${e.path}`);
  assert.deepEqual(brief, [
    "changed $.b[1]",
    "removed $.b[2]",
    "changed $.c.x",
    "added $.c.y",
    "changed $.big",
    "added $.new",
  ]);
});

test("大文件性能", () => {
  const big = JSON.stringify(
    Array.from({ length: 50000 }, (_, i) => ({ id: i, name: `item ${i}`, tags: ["a", "b"], v: i * 1.5 })),
  );
  const t = performance.now();
  const node = ok(big);
  const out = stringify(node, { indent: "  " });
  const ms = performance.now() - t;
  assert.equal(out, JSON.stringify(JSON.parse(big), null, 2));
  console.log(`  ${(big.length / 1e6).toFixed(1)} MB 解析+格式化 ${ms.toFixed(0)} ms`);
});
