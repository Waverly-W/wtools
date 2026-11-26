// 快捷键字符串与 Rust 端 global-hotkey 的解析格式一致，例如 "Alt+Space"、"Command+Shift+J"

const MODIFIER_CODES = new Set([
  "MetaLeft", "MetaRight", "AltLeft", "AltRight",
  "ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "CapsLock", "Fn",
]);

/** 从键盘事件生成快捷键字符串；只按了修饰键或组合无效时返回 null */
export function fromKeyboardEvent(e: KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(e.code)) return null;
  const key = normalizeCode(e.code);
  if (!key) return null;

  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Command");

  // 没有修饰键的全局快捷键会吞掉正常输入，只允许 F1–F20
  const isFn = /^F\d{1,2}$/.test(key);
  if (mods.length === 0 && !isFn) return null;
  // 只有 Shift 也会影响正常输入
  if (mods.length === 1 && mods[0] === "Shift" && !isFn) return null;

  return [...mods, key].join("+");
}

// 用 e.code（物理键位）而不是 e.key，避免 Option 组合键产生特殊字符
function normalizeCode(code: string): string | null {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (/^F\d{1,2}$/.test(code)) return code;
  const map: Record<string, string> = {
    Space: "Space", Enter: "Enter", Tab: "Tab", Backspace: "Backspace", Delete: "Delete",
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
    Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown",
    Backquote: "Backquote", Minus: "Minus", Equal: "Equal",
    BracketLeft: "BracketLeft", BracketRight: "BracketRight", Backslash: "Backslash",
    Semicolon: "Semicolon", Quote: "Quote", Comma: "Comma", Period: "Period", Slash: "Slash",
  };
  return map[code] ?? null;
}

const SYMBOLS: Record<string, string> = {
  Control: "⌃", Ctrl: "⌃", Alt: "⌥", Option: "⌥", Shift: "⇧",
  Command: "⌘", Cmd: "⌘", Super: "⌘", CommandOrControl: "⌘", CmdOrCtrl: "⌘",
  Space: "Space", Enter: "↩", Tab: "⇥", Backspace: "⌫", Delete: "⌦",
  Up: "↑", Down: "↓", Left: "←", Right: "→",
  Backquote: "`", Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]",
  Backslash: "\\", Semicolon: ";", Quote: "'", Comma: ",", Period: ".", Slash: "/",
};

/** 转为 macOS 风格的显示文本，如 ⌥Space、⌘⇧J */
export function formatAccelerator(accel: string): string {
  return accel
    .split("+")
    .map((p) => SYMBOLS[p] ?? p.toUpperCase())
    .join("");
}
