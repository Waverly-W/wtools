import { invoke } from "@tauri-apps/api/core";

export interface Item {
  id: string;
  title: string;
  subtitle: string;
  icon: string;
}

export interface HotkeyRow {
  id: string;
  title: string;
  module: string;
  accelerator: string | null;
  error: string | null;
}

export type ClipKind = "text" | "image" | "files";
export type ClipFilter = "all" | "text" | "image" | "files" | "favorite";

export interface ClipItem {
  id: number;
  kind: ClipKind;
  title: string;
  meta: string;
  source_app: string | null;
  favorite: boolean;
  tags: string[];
  used_at: number;
  has_ocr: boolean;
  thumb: string | null;
  image: string | null;
  preview: string;
  more: boolean;
}

export interface ClipDetail {
  id: number;
  kind: ClipKind;
  text: string | null;
  truncated: boolean;
  image: string | null;
  files: string[];
}

export interface OcrToken {
  text: string;
  space: boolean;
  atomic: boolean;
}

export interface OcrResult {
  lines: { text: string; tokens: OcrToken[] }[];
}

export interface CustomAction {
  id: string;
  name: string;
  kind: "url" | "shell";
  template: string;
  applies_to: ClipKind[];
  output: "none" | "show" | "copy" | "paste";
}

export interface ClipboardSettings {
  enabled: boolean;
  max_items: number;
  max_days: number;
  actions: CustomAction[];
  layout: "single" | "dual";
  max_lines: number;
  data_dir: string;
  is_default_dir: boolean;
}

export const api = {
  search: (query: string) => invoke<Item[]>("search", { query }),
  execute: (id: string) => invoke<void>("execute", { id }),
  hideLauncher: () => invoke<void>("hide_launcher"),
  listHotkeys: () => invoke<HotkeyRow[]>("list_hotkeys"),
  setHotkey: (id: string, accelerator: string | null) =>
    invoke<void>("set_hotkey", { id, accelerator }),
  suspendHotkeys: (suspend: boolean) => invoke<void>("suspend_hotkeys", { suspend }),
  setLauncherView: (view: string) => invoke<void>("set_launcher_view", { view }),

  clipList: (query: string, filter: ClipFilter, tag: string | null, limit = 300) =>
    invoke<ClipItem[]>("clipboard_list", { query, filter, tag, limit }),
  clipTags: () => invoke<string[]>("clipboard_tags"),
  clipDetail: (id: number) => invoke<ClipDetail>("clipboard_detail", { id }),
  clipPaste: (ids: number[], paste: boolean) => invoke<void>("clipboard_paste", { ids, paste }),
  clipPasteText: (text: string, paste: boolean) =>
    invoke<void>("clipboard_paste_text", { text, paste }),
  clipDelete: (ids: number[]) => invoke<void>("clipboard_delete", { ids }),
  clipFavorite: (ids: number[], favorite: boolean) =>
    invoke<void>("clipboard_favorite", { ids, favorite }),
  clipSetTags: (id: number, tags: string[]) => invoke<void>("clipboard_set_tags", { id, tags }),
  clipClear: () => invoke<void>("clipboard_clear"),
  clipOcr: (id: number) => invoke<OcrResult>("clipboard_ocr", { id }),
  clipReveal: (id: number) => invoke<void>("clipboard_reveal", { id }),
  clipRunAction: (actionId: string, id: number) =>
    invoke<{ output: string | null; message: string | null }>("clipboard_run_action", {
      actionId,
      id,
    }),
  clipTestAction: (action: CustomAction) =>
    invoke<{ ok: boolean; message: string }>("clipboard_test_action", { action }),
  getClipboardSettings: () => invoke<ClipboardSettings>("clipboard_get_settings"),
  setClipboardSettings: (settings: ClipboardSettings) =>
    invoke<void>("clipboard_set_settings", { settings }),
  chooseClipboardDir: () => invoke<string | null>("clipboard_choose_dir"),
  setClipboardDir: (dir: string | null) => invoke<string>("clipboard_set_dir", { dir }),

  autostartStatus: () => invoke<AutostartStatus>("autostart_status"),
  setAutostart: (enabled: boolean) => invoke<AutostartStatus>("autostart_set", { enabled }),
  openLoginItems: () => invoke<void>("autostart_open_settings"),

  enhanceGet: () => invoke<EnhanceState>("enhance_get"),
  enhanceSet: (config: EnhanceConfig) => invoke<EnhanceState>("enhance_set", { config }),
  enhanceRetry: () => invoke<EnhanceState>("enhance_retry"),
  enhanceImport: (feature: EnhanceFeature) =>
    invoke<{ state: EnhanceState; message: string }>("enhance_import", { feature }),
  enhanceRunningApps: () => invoke<RunningApp[]>("enhance_running_apps"),
  enhanceOpenAccessibility: () => invoke<void>("enhance_open_accessibility"),
  enhanceQuitOriginal: (bundleId: string) => invoke<void>("enhance_quit_original", { bundleId }),

  jsonOpen: (mode: JsonMode, text: string | null, right: string | null = null) =>
    invoke<void>("json_open", { mode, text, right }),
  jsonTakePending: () =>
    invoke<{ mode: JsonMode; text: string | null; right: string | null } | null>("json_take_pending"),
  jsonClipboardText: () => invoke<string | null>("json_clipboard_text"),
};

export type JsonMode = "format" | "diff";

export interface AutostartStatus {
  enabled: boolean;
  needs_approval: boolean;
  app_path: string;
}

export type EnhanceFeature = "stats" | "scroll" | "input_source" | "switcher" | "music_block";

export interface EnhanceConfig {
  stats: { enabled: boolean; cpu: boolean; memory: boolean; disk: boolean; interval: number };
  scroll: { enabled: boolean; smooth: boolean; reverse: boolean; step: number; speed: number; duration: number };
  input_source: {
    enabled: boolean;
    default_source: string | null;
    remember_last: boolean;
    rules: { bundle_id: string; name: string; source: string | null }[];
    indicator: "off" | "switch" | "always";
    corner: "top-left" | "top-right" | "bottom-left" | "bottom-right";
    colors: Record<string, string>;
  };
  switcher: {
    enabled: boolean;
    modifier: "ctrl" | "alt" | "cmd";
    current_screen_only: boolean;
    show_minimized: boolean;
    show_hidden: boolean;
    excluded: string[];
  };
  music_block: { enabled: boolean };
}

export interface EnhanceState {
  config: EnhanceConfig;
  originals: { feature: EnhanceFeature; name: string; bundle_id: string; running: boolean; importable: boolean }[];
  accessibility: boolean;
  errors: Partial<Record<EnhanceFeature, string>>;
  sources: { id: string; name: string }[];
}

export interface RunningApp {
  bundle_id: string;
  name: string;
}
