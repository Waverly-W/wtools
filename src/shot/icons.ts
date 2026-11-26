// 工具栏图标（24×24 描边路径）
export const ICONS: Record<string, string> = {
  rect: '<rect x="4" y="5" width="16" height="14" rx="1.5"/>',
  ellipse: '<ellipse cx="12" cy="12" rx="8.5" ry="7"/>',
  arrow: '<path d="M5 19 19 5M10 5h9v9"/>',
  line: '<path d="M5 19 19 5"/>',
  pen: '<path d="M4 20l4-1 11-11-3-3L5 16l-1 4zM14 7l3 3"/>',
  marker: '<path d="M9 15l-4 4h5l2-2M8 13l7-9 5 4-8 8-4-3zM4 21h16"/>',
  mosaic:
    '<rect x="4" y="4" width="16" height="16" rx="1.5"/><path d="M4 9.3h16M4 14.7h16M9.3 4v16M14.7 4v16" stroke-opacity=".5"/><rect x="4.5" y="4.5" width="4.8" height="4.8" fill="currentColor" stroke="none"/><rect x="14.7" y="9.3" width="4.8" height="5.4" fill="currentColor" stroke="none"/><rect x="9.3" y="14.7" width="5.4" height="4.8" fill="currentColor" stroke="none"/>',
  text: '<path d="M5 6V4.5h14V6M12 4.5V20M9 20h6"/>',
  number: '<circle cx="12" cy="12" r="8.5"/><path d="M10.5 9.5 12.5 8v8"/>',
  undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-3"/>',
  redo: '<path d="m15 14 5-5-5-5"/><path d="M20 9H10a6 6 0 0 0 0 12h3"/>',
  ocr: '<path d="M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3M8 9h8M8 12h8M8 15h5"/>',
  save: '<path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>',
  close: '<path d="M6 6l12 12M18 6 6 18"/>',
  done: '<path d="M5 12.5l4.5 4.5L19 7"/>',
};

export function icon(name: string): string {
  return `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;
}
