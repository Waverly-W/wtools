<script lang="ts">
  import { onMount } from "svelte";
  import { api, type AutostartStatus, type ClipboardSettings, type CustomAction, type HotkeyRow } from "../lib/api";
  import { formatAccelerator, fromKeyboardEvent } from "../lib/hotkey";
  import EnhanceTab from "./EnhanceTab.svelte";

  const LAUNCHER_TOGGLE = "launcher.toggle";

  let rows = $state<HotkeyRow[]>([]);
  let recording = $state<string | null>(null);

  type Tab = "general" | "clipboard" | "enhance" | "hotkeys";
  const TABS: { id: Tab; name: string; icon: string }[] = [
    { id: "general", name: "通用", icon: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>' },
    { id: "clipboard", name: "剪贴板", icon: '<rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>' },
    { id: "enhance", name: "系统增强", icon: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4M7 12l3-3 2 2 4-4"/>' },
    { id: "hotkeys", name: "快捷键", icon: '<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M8 14h8"/>' },
  ];
  const savedTab = (() => {
    try {
      return localStorage.getItem("wtools.settings.tab");
    } catch {
      return null;
    }
  })();
  let tab = $state<Tab>(TABS.some((t) => t.id === savedTab) ? (savedTab as Tab) : "general");

  function switchTab(t: Tab) {
    if (recording) stopRecording();
    tab = t;
    try {
      localStorage.setItem("wtools.settings.tab", t);
    } catch {
      /* 忽略 */
    }
  }
  let errors = $state<Record<string, string>>({});
  let clip = $state<ClipboardSettings | null>(null);
  let confirmClear = $state(false);
  let clearTimer: ReturnType<typeof setTimeout> | undefined;
  let clipMsg = $state("");
  const MAX_OPTIONS = [200, 500, 1000, 2000, 5000, 10000];
  const DAY_OPTIONS: [number, string][] = [
    [0, "不限"],
    [7, "7 天"],
    [30, "30 天"],
    [90, "90 天"],
    [180, "半年"],
    [365, "一年"],
  ];
  const LINE_OPTIONS = [2, 3, 4, 5, 6, 8, 10];
  const KIND_LABELS = { text: "文本", image: "图片", files: "文件" } as const;
  const OUTPUT_LABELS = { show: "显示结果", copy: "复制结果", paste: "粘贴结果", none: "不处理" } as const;

  // 自定义操作编辑
  let editing = $state<CustomAction | null>(null);
  let editingIsNew = $state(false);
  let testResults = $state<Record<string, { ok: boolean; message: string } | "testing">>({});

  async function saveClip() {
    if (!clip) return;
    try {
      await api.setClipboardSettings(clip);
      clipMsg = "";
    } catch (e) {
      clipMsg = String(e);
    }
  }

  async function chooseDir() {
    const dir = await api.chooseClipboardDir();
    if (dir) await switchDir(dir);
  }

  async function switchDir(dir: string | null) {
    try {
      clipMsg = await api.setClipboardDir(dir);
      clip = await api.getClipboardSettings();
    } catch (e) {
      clipMsg = String(e);
    }
  }

  function newAction() {
    editing = {
      id: `a${Date.now().toString(36)}`,
      name: "",
      kind: "url",
      template: "https://",
      applies_to: ["text"],
      output: "show",
    };
    editingIsNew = true;
  }

  function editAction(a: CustomAction) {
    editing = { ...a, applies_to: [...a.applies_to] };
    editingIsNew = false;
  }

  async function commitAction() {
    if (!clip || !editing) return;
    if (!editing.name.trim()) {
      clipMsg = "操作名称不能为空";
      return;
    }
    const e = editing;
    clip.actions = editingIsNew ? [...clip.actions, e] : clip.actions.map((a) => (a.id === e.id ? e : a));
    editing = null;
    await saveClip();
  }

  async function deleteAction(id: string) {
    if (!clip) return;
    clip.actions = clip.actions.filter((a) => a.id !== id);
    await saveClip();
  }

  async function testAction(a: CustomAction) {
    testResults = { ...testResults, [a.id]: "testing" };
    const r = await api.clipTestAction($state.snapshot(a) as CustomAction);
    testResults = { ...testResults, [a.id]: r };
  }

  function toggleKind(kind: "text" | "image" | "files") {
    if (!editing) return;
    editing.applies_to = editing.applies_to.includes(kind)
      ? editing.applies_to.filter((k) => k !== kind)
      : [...editing.applies_to, kind];
  }

  async function clearHistory() {
    if (!confirmClear) {
      // 二次确认：3 秒内再点一次才清空
      confirmClear = true;
      clearTimer = setTimeout(() => (confirmClear = false), 3000);
      return;
    }
    clearTimeout(clearTimer);
    confirmClear = false;
    await api.clipClear();
  }

  const groups = $derived.by(() => {
    const map = new Map<string, HotkeyRow[]>();
    for (const r of rows) {
      if (!map.has(r.module)) map.set(r.module, []);
      map.get(r.module)!.push(r);
    }
    return [...map];
  });

  async function load() {
    rows = await api.listHotkeys();
  }

  async function startRecording(id: string) {
    if (recording) await stopRecording();
    recording = id;
    errors = { ...errors, [id]: "" };
    // 暂停全局快捷键，否则按下已绑定的组合会被系统直接拦截
    await api.suspendHotkeys(true);
  }

  async function stopRecording() {
    recording = null;
    await api.suspendHotkeys(false);
  }

  async function save(id: string, accel: string | null) {
    recording = null;
    try {
      await api.setHotkey(id, accel);
      errors = { ...errors, [id]: "" };
    } catch (e) {
      errors = { ...errors, [id]: String(e) };
      await api.suspendHotkeys(false);
    }
    await load();
  }

  function onKeydown(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    const id = recording;
    if (e.key === "Escape" && !e.metaKey && !e.altKey && !e.ctrlKey) {
      stopRecording();
      return;
    }
    if ((e.key === "Backspace" || e.key === "Delete") && !e.metaKey && !e.altKey && !e.ctrlKey) {
      if (id === LAUNCHER_TOGGLE) {
        errors = { ...errors, [id]: "呼出搜索框的快捷键不能为空" };
        stopRecording();
      } else {
        save(id, null);
      }
      return;
    }
    const accel = fromKeyboardEvent(e);
    if (accel) save(id, accel);
  }

  let autostart = $state<AutostartStatus | null>(null);
  let autostartError = $state("");

  async function toggleAutostart(e: Event) {
    const want = (e.currentTarget as HTMLInputElement).checked;
    autostartError = "";
    try {
      autostart = await api.setAutostart(want);
    } catch (err) {
      autostartError = String(err);
      autostart = await api.autostartStatus();
    }
  }

  onMount(() => {
    load();
    api.autostartStatus().then((s) => (autostart = s));
    // 用户可能在系统设置里改过登录项，切回窗口时刷新
    const refresh = () => api.autostartStatus().then((s) => (autostart = s));
    window.addEventListener("focus", refresh);
    api.getClipboardSettings().then((c) => (clip = c));
    const cancel = () => recording && stopRecording();
    window.addEventListener("blur", cancel);
    return () => {
      window.removeEventListener("blur", cancel);
      window.removeEventListener("focus", refresh);
    };
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="layout">
<nav class="sidebar">
  {#each TABS as t (t.id)}
    <button class:on={tab === t.id} onclick={() => switchTab(t.id)}>
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{@html t.icon}</svg>
      {t.name}
    </button>
  {/each}
</nav>
<main>
{#if tab === "general"}
  <header>
    <h1>通用</h1>
  </header>
  {#if autostart}
    <section>
      <div class="card">
        <div class="row">
          <div class="label">
            <span>开机自动启动</span>
            {#if autostartError}
              <span class="err">{autostartError}</span>
            {:else if autostart.needs_approval}
              <span class="err">
                需要在系统设置中允许 wtools 登录时打开
                <button class="link" onclick={() => api.openLoginItems()}>打开登录项设置</button>
              </span>
            {:else if !autostart.app_path.startsWith("/Applications/")}
              <span class="err">当前运行的不是「应用程序」里的 wtools，开机将启动：{autostart.app_path}</span>
            {:else}
              <span class="hint">登录后在菜单栏常驻；也可以在「系统设置 → 通用 → 登录项」中管理</span>
            {/if}
          </div>
          <input
            type="checkbox"
            class="switch"
            checked={autostart.enabled || autostart.needs_approval}
            onchange={toggleAutostart}
          />
        </div>
      </div>
    </section>
  {/if}

{:else if tab === "clipboard"}
  <header>
    <h1>剪贴板</h1>
  </header>
  {#if clip}
    <section>
      <h2>显示</h2>
      <div class="card">
        <div class="row">
          <div class="label">
            <span>默认布局</span>
            <span class="hint">双栏：左侧剪贴板历史、右侧收藏；剪贴板窗口右上角可临时切换</span>
          </div>
          <select class="select" bind:value={clip.layout} onchange={saveClip}>
            <option value="dual">双栏（剪贴板 + 收藏）</option>
            <option value="single">单栏</option>
          </select>
        </div>
        <div class="row">
          <div class="label">
            <span>每条最多显示行数</span>
            <span class="hint">超出的内容需点击「展开」或按 ⌘E 查看</span>
          </div>
          <select class="select" bind:value={clip.max_lines} onchange={saveClip}>
            {#each LINE_OPTIONS as n (n)}<option value={n}>{n} 行</option>{/each}
          </select>
        </div>
      </div>
    </section>

    <section>
      <h2>保存规则</h2>
      <div class="card">
        <div class="row">
          <div class="label"><span>记录剪贴板历史</span></div>
          <input type="checkbox" class="switch" bind:checked={clip.enabled} onchange={saveClip} />
        </div>
        <div class="row">
          <div class="label">
            <span>最多保存条数</span>
            <span class="hint">收藏的记录不计入、不会被清理</span>
          </div>
          <select class="select" bind:value={clip.max_items} onchange={saveClip}>
            {#each MAX_OPTIONS as n (n)}<option value={n}>{n.toLocaleString()} 条</option>{/each}
          </select>
        </div>
        <div class="row">
          <div class="label">
            <span>最多保存天数</span>
            <span class="hint">超过天数的记录自动清理，收藏的除外</span>
          </div>
          <select class="select" bind:value={clip.max_days} onchange={saveClip}>
            {#each DAY_OPTIONS as [n, label] (n)}<option value={n}>{label}</option>{/each}
          </select>
        </div>
        <div class="row">
          <div class="label">
            <span>数据目录</span>
            <span class="hint path" title={clip.data_dir}>{clip.data_dir}</span>
            <span class="hint">可选网盘目录在多台电脑间同步；请避免多台电脑同时运行 wtools，以免同步冲突</span>
          </div>
          <div class="btns">
            <button class="btn" onclick={chooseDir}>更改…</button>
            {#if !clip.is_default_dir}
              <button class="btn" onclick={() => switchDir(null)}>恢复默认</button>
            {/if}
          </div>
        </div>
        <div class="row">
          <div class="label">
            <span>清空历史</span>
            <span class="hint">删除全部未收藏的记录，不可恢复</span>
          </div>
          <button class="btn" class:danger={confirmClear} onclick={clearHistory}>
            {confirmClear ? "再次点击确认" : "清空"}
          </button>
        </div>
      </div>
      {#if clipMsg}<p class="note">{clipMsg}</p>{/if}
    </section>

    <section>
      <h2>自定义操作</h2>
      <div class="card">
        {#each clip.actions as a (a.id)}
          {@const r = testResults[a.id]}
          <div class="row">
            <div class="label">
              <span>{a.name} <small class="badge">{a.kind === "url" ? "网址" : "命令"}</small></span>
              <span class="hint mono">{a.template}</span>
              <span class="hint">适用：{a.applies_to.map((k) => KIND_LABELS[k]).join("、") || "无"}</span>
              {#if r === "testing"}
                <span class="hint">检测中…</span>
              {:else if r}
                <span class={r.ok ? "ok" : "err"}>{r.ok ? "✓" : "✗"} {r.message}</span>
              {/if}
            </div>
            <div class="btns">
              <button class="btn" onclick={() => testAction(a)}>检测</button>
              <button class="btn" onclick={() => editAction(a)}>编辑</button>
              <button class="btn" onclick={() => deleteAction(a.id)}>删除</button>
            </div>
          </div>
        {/each}
        <div class="row">
          <div class="label">
            <span class="hint">网址中的 <code>{"{text}"}</code> 替换为内容（自动编码），<code>{"{file}"}</code> 替换为图片/文件路径；命令通过标准输入和环境变量 WTOOLS_TEXT、WTOOLS_FILE 接收内容</span>
          </div>
          <button class="btn" onclick={newAction}>添加操作</button>
        </div>
      </div>

      {#if editing}
        <div class="card editor">
          <label><span>名称</span><input class="input" bind:value={editing.name} placeholder="如：翻译成英文" /></label>
          <label>
            <span>类型</span>
            <select class="select" bind:value={editing.kind}>
              <option value="url">打开网址</option>
              <option value="shell">运行命令</option>
            </select>
          </label>
          <label class="top">
            <span>{editing.kind === "url" ? "网址" : "命令"}</span>
            <textarea
              class="input"
              rows="3"
              bind:value={editing.template}
              spellcheck="false"
              placeholder={editing.kind === "url" ? "https://example.com/search?q={text}" : "tr '[:lower:]' '[:upper:]'"}
            ></textarea>
          </label>
          <div class="field">
            <span>适用于</span>
            {#each ["text", "image", "files"] as const as k (k)}
              <label class="check"><input type="checkbox" checked={editing.applies_to.includes(k)} onchange={() => toggleKind(k)} />{KIND_LABELS[k]}</label>
            {/each}
          </div>
          {#if editing.kind === "shell"}
            <label>
              <span>命令输出</span>
              <select class="select" bind:value={editing.output}>
                {#each Object.entries(OUTPUT_LABELS) as [v, label] (v)}<option value={v}>{label}</option>{/each}
              </select>
            </label>
          {/if}
          {#if testResults[editing.id] && testResults[editing.id] !== "testing"}
            {@const r = testResults[editing.id] as { ok: boolean; message: string }}
            <p class={r.ok ? "ok" : "err"}>{r.ok ? "✓" : "✗"} {r.message}</p>
          {/if}
          <div class="editor-btns">
            <button class="btn" onclick={() => editing && testAction(editing)}>
              {testResults[editing.id] === "testing" ? "检测中…" : "检测是否可用"}
            </button>
            <span class="spacer"></span>
            <button class="btn" onclick={() => (editing = null)}>取消</button>
            <button class="btn primary" onclick={commitAction}>保存</button>
          </div>
        </div>
      {/if}
    </section>
  {/if}

{:else if tab === "enhance"}
  <EnhanceTab />

{:else}
  <header>
    <h1>快捷键</h1>
    <p>点击右侧按钮后按下组合键即可设置；Esc 取消，⌫ 清除。</p>
  </header>

  {#each groups as [module, list] (module)}
    <section>
      <h2>{module}</h2>
      <div class="card">
        {#each list as row (row.id)}
          {@const err = errors[row.id] || row.error}
          <div class="row">
            <div class="label">
              <span>{row.title}</span>
              {#if err}<span class="err">{err}</span>{/if}
            </div>
            <button
              class="btn hotkey"
              class:recording={recording === row.id}
              class:empty={!row.accelerator}
              onclick={() => (recording === row.id ? stopRecording() : startRecording(row.id))}
            >
              {#if recording === row.id}
                按下快捷键…
              {:else if row.accelerator}
                {formatAccelerator(row.accelerator)}
              {:else}
                未设置
              {/if}
            </button>
          </div>
        {/each}
      </div>
    </section>
  {/each}
{/if}
</main>
</div>
