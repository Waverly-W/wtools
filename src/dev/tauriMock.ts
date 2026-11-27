/**
 * 仅开发模式：在普通浏览器里打开页面时模拟 Tauri 接口，用假数据调试界面。
 * 在真实应用（存在 __TAURI_INTERNALS__）中不会加载。
 * 控制台可调用 __mockEmit("launcher:show", "clipboard") 切换到剪贴板视图。
 */
export {};

type Fn = (...args: any[]) => any;

const callbacks = new Map<number, Fn>();
const listeners: Record<string, number[]> = {};
let nextId = 1;
const now = Math.floor(Date.now() / 1000);

const svg = (w: number, h: number, color: string, label: string) =>
  `data:image/svg+xml;utf8,${encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><rect width="100%" height="100%" fill="${color}"/><text x="50%" y="50%" font-size="${h / 8}" text-anchor="middle" fill="#fff" font-family="sans-serif">${label}</text></svg>`,
  )}`;

const longText = Array.from({ length: 12 }, (_, i) => `第 ${i + 1} 行：这是一段比较长的示例文本，用来测试按行数截断与展开。`).join("\n");

let clips: any[] = [
  { id: 1, kind: "text", title: "SELECT * FROM orders", meta: "120", source_app: "DataGrip", favorite: true, tags: ["SQL", "常用"], used_at: now - 30, has_ocr: false, thumb: null, image: null, preview: "SELECT o.id, o.amount, u.name\nFROM orders o\nJOIN users u ON u.id = o.user_id\nWHERE o.created_at > '2026-11-01'\n  AND o.status = 'PAID'\nORDER BY o.amount DESC\nLIMIT 100;", more: false },
  { id: 2, kind: "text", title: "长文本", meta: "420", source_app: "备忘录", favorite: false, tags: [], used_at: now - 300, has_ocr: false, thumb: null, image: null, preview: longText, more: false },
  { id: 3, kind: "image", title: "图片 1800×520", meta: "1800×520", source_app: "截图", favorite: false, tags: [], used_at: now - 3600, has_ocr: true, thumb: svg(480, 139, "#4c7bd9", "缩略图"), image: svg(1800, 520, "#2f5fc4", "原图 1800×520"), preview: "", more: false },
  { id: 4, kind: "files", title: "a.pdf 等 3 个文件", meta: "3", source_app: "访达", favorite: true, tags: ["工作"], used_at: now - 7200, has_ocr: false, thumb: null, image: null, preview: "/Users/me/Documents/合同/a.pdf\n/Users/me/Documents/合同/b.pdf\n/Users/me/Desktop/c.png", more: false },
  { id: 5, kind: "text", title: "https://example.com", meta: "28", source_app: "Chrome", favorite: false, tags: [], used_at: now - 86400 * 2, has_ocr: false, thumb: null, image: null, preview: "https://example.com/a?b=1&c=2", more: false },
  { id: 6, kind: "text", title: "{\"ok\":true}", meta: "11", source_app: "VS Code", favorite: true, tags: ["常用"], used_at: now - 86400 * 3, has_ocr: false, thumb: null, image: null, preview: "{\n  \"name\": \"wtools\",\n  \"ok\": true\n}", more: false },
];

// 系统增强（模拟）
let enhance: any = {
  stats: { enabled: true, cpu: true, memory: true, disk: false, interval: 2 },
  scroll: { enabled: true, smooth: true, reverse: true, step: 10, speed: 3, duration: 3.9 },
  input_source: {
    enabled: true,
    default_source: "com.tencent.inputmethod.wetype.pinyin",
    remember_last: false,
    rules: [{ bundle_id: "com.microsoft.edgemac", name: "Microsoft Edge", source: "com.tencent.inputmethod.wetype.pinyin" }],
    indicator: "always",
    corner: "bottom-right",
    colors: { "com.tencent.inputmethod.wetype.pinyin": "#5fc594" },
  },
  switcher: { enabled: true, modifier: "ctrl", current_screen_only: true, show_minimized: true, show_hidden: true, excluded: ["org.yuanli.utools"] },
  music_block: { enabled: true },
};
const enhanceState = () => ({
  config: enhance,
  originals: [
    { feature: "stats", name: "Stats", bundle_id: "eu.exelban.Stats", running: true, importable: true },
    { feature: "scroll", name: "Mos", bundle_id: "com.caldis.Mos", running: false, importable: true },
    { feature: "input_source", name: "Input Source Pro", bundle_id: "com.runjuu.Input-Source-Pro", running: false, importable: true },
    { feature: "input_source", name: "TypeRelay", bundle_id: "com.micky.typerelay", running: true, importable: false },
    { feature: "switcher", name: "AltTab", bundle_id: "com.lwouis.alt-tab-macos", running: false, importable: true },
    { feature: "music_block", name: "noTunes", bundle_id: "digital.twisted.noTunes", running: true, importable: false },
  ],
  accessibility: false,
  errors: { switcher: "需要「辅助功能」权限" },
  sources: [
    { id: "com.apple.keylayout.IrishExtended", name: "爱尔兰 – 扩展" },
    { id: "com.tencent.inputmethod.wetype.pinyin", name: "微信输入法" },
  ],
});

const handlers: Record<string, Fn> = {
  "plugin:event|listen": ({ event, handler }) => ((listeners[event] ||= []).push(handler), handler),
  "plugin:event|unlisten": () => null,
  "plugin:window|set_size": () => null,
  set_launcher_view: () => null,
  hide_launcher: () => console.log("[mock] hide_launcher"),
  search: ({ query }: { query: string }) =>
    query
      ? [
          { id: "app:safari", title: "Safari 浏览器", subtitle: "/Applications/Safari.app", icon: "🧭" },
          { id: "cmd:json", title: "JSON 工具", subtitle: "格式化、转义、对比", icon: "🧩" },
          { id: "cmd:clip", title: "剪贴板历史", subtitle: "", icon: "📋" },
        ]
      : [],
  clipboard_get_settings: () => ({ enabled: true, max_items: 1000, max_days: 0, actions: [{ id: "up", name: "转为大写", kind: "shell", template: "tr", applies_to: ["text"], output: "show" }], layout: "dual", max_lines: 4, data_dir: "/mock", is_default_dir: true }),
  clipboard_tags: () => [...new Set(clips.filter((c) => c.favorite).flatMap((c) => c.tags))].sort(),
  clipboard_list: ({ query, filter, tag }) =>
    clips
      .filter((c) => !query || c.preview.includes(query) || c.title.includes(query))
      .filter((c) => (filter === "favorite" ? c.favorite : filter && filter !== "all" ? c.kind === filter : true))
      .filter((c) => !tag || c.tags.includes(tag)),
  clipboard_detail: ({ id }) => {
    const c = clips.find((x) => x.id === id)!;
    return { id, kind: c.kind, text: c.preview, truncated: false, image: c.image, files: c.preview.split("\n") };
  },
  clipboard_favorite: ({ ids, favorite }) => {
    clips = clips.map((c) => (ids.includes(c.id) ? { ...c, favorite } : c));
  },
  clipboard_set_tags: ({ id, tags }) => {
    clips = clips.map((c) => (c.id === id ? { ...c, tags, favorite: tags.length ? true : c.favorite } : c));
  },
  clipboard_delete: ({ ids }) => {
    clips = clips.filter((c) => !ids.includes(c.id));
  },
  clipboard_paste: (a) => console.log("[mock] paste", a),
  clipboard_paste_text: (a) => console.log("[mock] paste_text", a),
  "plugin:window|close": () => console.log("[mock] close window"),
  json_open: (a) => console.log("[mock] json_open", a),
  list_hotkeys: () => [{ id: "launcher.toggle", title: "呼出 / 隐藏搜索框", module: "wtools", accelerator: "Alt+Space", error: null }],
  autostart_status: () => ({ enabled: false, needs_approval: false, app_path: "/Applications/wtools.app" }),
  enhance_get: () => enhanceState(),
  enhance_set: ({ config }) => ((enhance = config), enhanceState()),
  enhance_retry: () => enhanceState(),
  enhance_import: ({ feature }) => ({ state: enhanceState(), message: `已导入（模拟）：${feature}` }),
  enhance_running_apps: () => [
    { bundle_id: "com.wtools.app", name: "wtools", own: true },
    { bundle_id: "com.google.Chrome", name: "Google Chrome", own: false },
    { bundle_id: "com.apple.Terminal", name: "终端", own: false },
    { bundle_id: "com.microsoft.edgemac", name: "Microsoft Edge", own: false },
  ],
  enhance_open_accessibility: () => null,
  enhance_quit_original: () => null,
  autostart_set: ({ enabled }) => ({ enabled, needs_approval: false, app_path: "/Applications/wtools.app" }),
  shot_take_pending: () => { const p = shotPending; shotPending = null; return p; },
  shot_image: () => fakeDesktop(shotInfo()),
  shot_show: () => null,
  shot_cancel: () => console.log("[mock] shot_cancel"),
  shot_copy: (png) => console.log("[mock] shot_copy", png?.byteLength ?? png?.length, "bytes"),
  shot_save: (png) => console.log("[mock] shot_save", png?.byteLength ?? png?.length, "bytes"),
  shot_text: (a) => console.log("[mock] shot_text", a),
  shot_ocr: async () => {
    await new Promise((r) => setTimeout(r, 600));
    const tok = (text: string, space = false, atomic = false) => ({ text, space, atomic });
    return { lines: [
      { text: "订单号：1234567890123456789", tokens: [tok("订单号"), tok("："), tok("1234567890123456789", false, true)] },
      { text: "联系电话 138-0013-8000", tokens: [tok("联系"), tok("电话"), tok("138-0013-8000", true, true)] },
      { text: "访问 https://example.com/a?b=1", tokens: [tok("访问"), tok("https://example.com/a?b=1", true, true)] },
    ] };
  },
  json_take_pending: () => null,
  json_clipboard_text: () =>
    '{"orderId":1234567890123456789,"user":{"name":"张三","tags":["vip",],},// 注释\n"items":[{"sku":"A1","price":12.5}],"raw":"{\\"nested\\":true}"}',
};

// 截图：生成一张假的桌面截图
const shotInfo = () => {
  const lw = innerWidth, lh = innerHeight, scale = devicePixelRatio || 1;
  return {
    seq: 1, mode: new URLSearchParams(location.search).get("mode") ?? "capture",
    width: Math.round(lw * scale), height: Math.round(lh * scale), lw, lh, scale,
    windows: [
      { x: 0, y: 0, w: lw, h: 26 },
      { x: 80, y: 70, w: Math.min(640, lw * 0.5), h: 420 },
      { x: lw * 0.55, y: 140, w: lw * 0.38, h: 360 },
    ],
  };
};
let shotPending: any = location.pathname.includes("shot") ? shotInfo() : null;
function fakeDesktop(info: any): ArrayBuffer {
  const c = document.createElement("canvas");
  c.width = info.width; c.height = info.height;
  const ctx = c.getContext("2d")!;
  ctx.scale(info.scale, info.scale);
  const g = ctx.createLinearGradient(0, 0, info.lw, info.lh);
  g.addColorStop(0, "#3a6ea5"); g.addColorStop(1, "#8e5ea2");
  ctx.fillStyle = g; ctx.fillRect(0, 0, info.lw, info.lh);
  ctx.fillStyle = "rgba(255,255,255,.85)"; ctx.fillRect(0, 0, info.lw, 26);
  ctx.fillStyle = "#222"; ctx.font = "13px -apple-system"; ctx.fillText("  文件   编辑   显示   窗口   帮助", 10, 17);
  info.windows.slice(1).forEach((w: any, i: number) => {
    ctx.fillStyle = "#fff"; ctx.fillRect(w.x, w.y, w.w, w.h);
    ctx.fillStyle = "#e8e8ea"; ctx.fillRect(w.x, w.y, w.w, 28);
    ["#ff5f57", "#febc2e", "#28c840"].forEach((col, k) => { ctx.fillStyle = col; ctx.beginPath(); ctx.arc(w.x + 16 + k * 20, w.y + 14, 6, 0, 7); ctx.fill(); });
    ctx.fillStyle = "#222"; ctx.font = "15px -apple-system";
    const lines = i === 0
      ? ["订单号：1234567890123456789", "联系电话 138-0013-8000", "访问 https://example.com/a?b=1", "金额 ¥1,234.56 元，日期 2026-11-24"]
      : ["The quick brown fox", "jumps over the lazy dog.", "wtools 截图 OCR 测试"];
    lines.forEach((l, k) => ctx.fillText(l, w.x + 20, w.y + 64 + k * 30));
  });
  return ctx.getImageData(0, 0, c.width, c.height).data.buffer;
}

const page = location.pathname.replace(/^\/|\.html$/g, "");
const label = page && page !== "index" ? page : "launcher";

(window as any).__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label }, currentWebview: { windowLabel: label, label } },
  transformCallback(cb: Fn) {
    const id = nextId++;
    callbacks.set(id, cb);
    return id;
  },
  unregisterCallback(id: number) {
    callbacks.delete(id);
  },
  convertFileSrc: (p: string) => p,
  async invoke(cmd: string, args: any) {
    const h = handlers[cmd];
    if (!h) {
      console.warn("[mock] 未模拟的命令", cmd, args);
      return null;
    }
    return h(args ?? {});
  },
};

(window as any).__mockShot = () => { shotPending = shotInfo(); (window as any).__mockEmit("shot:start", null); };
(window as any).__mockEmit = (event: string, payload: unknown) =>
  (listeners[event] ?? []).forEach((id) => callbacks.get(id)?.({ event, id, payload }));

// 取消事件监听时 @tauri-apps/api 会调用它
(window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener(event: string, id: number) {
    listeners[event] = (listeners[event] ?? []).filter((x) => x !== id);
    callbacks.delete(id);
  },
};
