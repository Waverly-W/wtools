import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

export const LAUNCHER_WIDTH = 1080;
const win = getCurrentWindow();
let lastHeight = 0;

/** 调整搜索框窗口高度（相同高度不重复调用） */
export function setHeight(height: number) {
  if (height === lastHeight) return;
  lastHeight = height;
  win.setSize(new LogicalSize(LAUNCHER_WIDTH, height));
}
