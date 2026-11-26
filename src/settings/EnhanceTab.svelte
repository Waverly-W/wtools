<script lang="ts">
  import { onMount } from "svelte";
  import { api, type EnhanceConfig, type EnhanceFeature, type EnhanceState, type RunningApp } from "../lib/api";

  let st = $state<EnhanceState | null>(null);
  let cfg = $state<EnhanceConfig | null>(null);
  let msg = $state<Partial<Record<EnhanceFeature, string>>>({});
  let apps = $state<RunningApp[]>([]);

  const NEEDS_AX: EnhanceFeature[] = ["scroll", "input_source", "switcher"];
  const CORNERS = [
    ["bottom-right", "右下角"],
    ["bottom-left", "左下角"],
    ["top-right", "右上角"],
    ["top-left", "左上角"],
  ] as const;
  const MODIFIERS = [
    ["ctrl", "⌃ Tab"],
    ["alt", "⌥ Tab"],
    ["cmd", "⌘ Tab（替换系统的应用切换）"],
  ] as const;

  function take(s: EnhanceState) {
    st = s;
    cfg = s.config;
  }

  async function refresh() {
    take(await api.enhanceGet());
  }

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  /** 保存；滑块拖动时稍等再保存 */
  function save(delay = 0) {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      if (!cfg) return;
      try {
        const s = await api.enhanceSet($state.snapshot(cfg) as EnhanceConfig);
        // 保留正在编辑的配置，只更新状态信息
        st = { ...s, config: cfg };
      } catch (e) {
        msg = { ...msg, stats: String(e) };
      }
    }, delay);
  }

  async function importFrom(f: EnhanceFeature) {
    try {
      const r = await api.enhanceImport(f);
      take(r.state);
      msg = { ...msg, [f]: r.message };
    } catch (e) {
      msg = { ...msg, [f]: String(e) };
    }
  }

  async function quit(bundleId: string) {
    await api.enhanceQuitOriginal(bundleId);
    setTimeout(refresh, 800);
  }

  /** 同一功能可能对应多个原应用（如 Input Source Pro、TypeRelay） */
  const originals = (f: EnhanceFeature) => st?.originals.filter((o) => o.feature === f) ?? [];
  const running = (f: EnhanceFeature) => (cfg?.[f].enabled ? originals(f).find((o) => o.running) : undefined);
  const sourceName = (id: string | null) => st?.sources.find((s) => s.id === id)?.name ?? id ?? "";
  const appName = (bundle: string) => apps.find((a) => a.bundle_id === bundle)?.name ?? bundle;
  const needsPermission = $derived(
    !!st && !st.accessibility && !!cfg && NEEDS_AX.some((f) => cfg![f].enabled),
  );

  let ruleApp = $state("");
  function addRule() {
    if (!cfg || !ruleApp) return;
    const a = apps.find((x) => x.bundle_id === ruleApp);
    if (!a || cfg.input_source.rules.some((r) => r.bundle_id === a.bundle_id)) return;
    cfg.input_source.rules = [...cfg.input_source.rules, { bundle_id: a.bundle_id, name: a.name, source: null }];
    ruleApp = "";
    save();
  }

  function removeRule(bundle: string) {
    if (!cfg) return;
    cfg.input_source.rules = cfg.input_source.rules.filter((r) => r.bundle_id !== bundle);
    save();
  }

  function setColor(id: string, hex: string) {
    if (!cfg) return;
    cfg.input_source.colors = { ...cfg.input_source.colors, [id]: hex };
    save(300);
  }

  let excludeApp = $state("");
  function addExcluded() {
    if (!cfg || !excludeApp || cfg.switcher.excluded.includes(excludeApp)) return;
    cfg.switcher.excluded = [...cfg.switcher.excluded, excludeApp];
    excludeApp = "";
    save();
  }

  function removeExcluded(bundle: string) {
    if (!cfg) return;
    cfg.switcher.excluded = cfg.switcher.excluded.filter((b) => b !== bundle);
    save();
  }

  onMount(() => {
    refresh();
    api.enhanceRunningApps().then((a) => (apps = a));
    // 用户可能刚在系统设置里授权、或退出了原应用
    const onFocus = async () => {
      const wasTrusted = st?.accessibility;
      const s = await api.enhanceGet();
      take(!wasTrusted && s.accessibility ? await api.enhanceRetry() : s);
      apps = await api.enhanceRunningApps();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });
</script>

{#snippet featureHead(f: EnhanceFeature, title: string)}
  {@const o = originals(f).find((x) => x.importable)}
  <div class="feature-head">
    <h2>{title}</h2>
    {#if o?.importable}
      <button class="link" onclick={() => importFrom(f)}>从 {o.name} 导入设置</button>
    {/if}
  </div>
{/snippet}

{#snippet status(f: EnhanceFeature)}
  {@const o = running(f)}
  {#if o}
    <span class="err">
      {o.name} 仍在运行，两者同时开启会重复生效
      <button class="link" onclick={() => quit(o.bundle_id)}>退出 {o.name}</button>
    </span>
  {:else if st?.errors[f]}
    <span class="err">{st.errors[f]}</span>
  {:else if msg[f]}
    <span class="hint">{msg[f]}</span>
  {/if}
{/snippet}

<header>
  <h1>系统增强</h1>
  <p>常驻后台的系统小功能，可替代 Stats、Mos、Input Source Pro / TypeRelay、AltTab、noTunes；每项单独开关，修改后立即生效。</p>
</header>

{#if needsPermission}
  <div class="banner">
    <span>滚动增强、输入法切换和窗口切换需要「辅助功能」权限。授权后回到此窗口即可自动生效。</span>
    <button class="btn" onclick={() => api.enhanceOpenAccessibility()}>打开系统设置</button>
  </div>
{/if}

{#if cfg && st}
  <section>
    {@render featureHead("stats", "系统监控")}
    <div class="card">
      <div class="row">
        <div class="label">
          <span>在菜单栏显示 CPU、内存、磁盘占用</span>
          {@render status("stats")}
        </div>
        <input type="checkbox" class="switch" bind:checked={cfg.stats.enabled} onchange={() => save()} />
      </div>
      {#if cfg.stats.enabled}
        <div class="row sub">
          <div class="label"><span>显示项目</span></div>
          <div class="checks">
            <label class="check"><input type="checkbox" bind:checked={cfg.stats.cpu} onchange={() => save()} />CPU</label>
            <label class="check"><input type="checkbox" bind:checked={cfg.stats.memory} onchange={() => save()} />内存</label>
            <label class="check"><input type="checkbox" bind:checked={cfg.stats.disk} onchange={() => save()} />磁盘</label>
          </div>
        </div>
        <div class="row sub">
          <div class="label"><span>刷新间隔</span></div>
          <select class="select" bind:value={cfg.stats.interval} onchange={() => save()}>
            {#each [1, 2, 3, 5, 10] as n (n)}<option value={n}>{n} 秒</option>{/each}
          </select>
        </div>
      {/if}
    </div>
  </section>

  <section>
    {@render featureHead("scroll", "鼠标滚动")}
    <div class="card">
      <div class="row">
        <div class="label">
          <span>鼠标滚轮增强</span>
          {@render status("scroll")}
          {#if !st.errors.scroll && !msg.scroll && !running("scroll")}
            <span class="hint">只作用于鼠标滚轮，触控板和妙控鼠标不受影响</span>
          {/if}
        </div>
        <input type="checkbox" class="switch" bind:checked={cfg.scroll.enabled} onchange={() => save()} />
      </div>
      {#if cfg.scroll.enabled}
        <div class="row sub">
          <div class="label"><span>平滑滚动</span></div>
          <input type="checkbox" class="switch" bind:checked={cfg.scroll.smooth} onchange={() => save()} />
        </div>
        <div class="row sub">
          <div class="label"><span>反向滚动</span><span class="hint">与触控板方向分开设置</span></div>
          <input type="checkbox" class="switch" bind:checked={cfg.scroll.reverse} onchange={() => save()} />
        </div>
        {#if cfg.scroll.smooth}
          <div class="row sub">
            <div class="label"><span>步长</span><span class="hint">每次滚动的最小距离</span></div>
            <div class="range">
              <input type="range" min="0" max="80" step="1" bind:value={cfg.scroll.step} oninput={() => save(250)} />
              <output>{cfg.scroll.step}</output>
            </div>
          </div>
          <div class="row sub">
            <div class="label"><span>速度</span><span class="hint">滚轮转动距离的放大倍数</span></div>
            <div class="range">
              <input type="range" min="0.5" max="8" step="0.1" bind:value={cfg.scroll.speed} oninput={() => save(250)} />
              <output>{cfg.scroll.speed.toFixed(1)}</output>
            </div>
          </div>
          <div class="row sub">
            <div class="label"><span>过渡</span><span class="hint">越大越顺滑，停下得越慢</span></div>
            <div class="range">
              <input type="range" min="1" max="10" step="0.1" bind:value={cfg.scroll.duration} oninput={() => save(250)} />
              <output>{cfg.scroll.duration.toFixed(1)}</output>
            </div>
          </div>
        {/if}
      {/if}
    </div>
  </section>

  <section>
    {@render featureHead("input_source", "输入法自动切换")}
    <div class="card">
      <div class="row">
        <div class="label">
          <span>切换应用时自动切换输入法</span>
          {@render status("input_source")}
        </div>
        <input type="checkbox" class="switch" bind:checked={cfg.input_source.enabled} onchange={() => save()} />
      </div>
      {#if cfg.input_source.enabled}
        <div class="row sub">
          <div class="label"><span>默认输入法</span><span class="hint">没有单独设置的应用使用</span></div>
          <select class="select" bind:value={cfg.input_source.default_source} onchange={() => save()}>
            <option value={null}>不切换</option>
            {#each st.sources as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
          </select>
        </div>
        <div class="row sub">
          <div class="label"><span>记住每个应用上次的输入法</span><span class="hint">优先于默认输入法</span></div>
          <input type="checkbox" class="switch" bind:checked={cfg.input_source.remember_last} onchange={() => save()} />
        </div>
        {#each cfg.input_source.rules as r (r.bundle_id)}
          <div class="row sub">
            <div class="label"><span>{r.name}</span><span class="hint mono">{r.bundle_id}</span></div>
            <div class="btns">
              <select class="select" bind:value={r.source} onchange={() => save()}>
                <option value={null}>跟随默认</option>
                {#each st.sources as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
                {#if r.source && !st.sources.some((s) => s.id === r.source)}
                  <option value={r.source}>{r.source}（未启用）</option>
                {/if}
              </select>
              <button class="btn plain icon sm remove" title="删除" onclick={() => removeRule(r.bundle_id)}>×</button>
            </div>
          </div>
        {/each}
        <div class="row sub">
          <div class="label"><span class="hint">为指定应用单独设置输入法</span></div>
          <div class="btns">
            <select class="select" bind:value={ruleApp}>
              <option value="">选择正在运行的应用…</option>
              {#each apps.filter((a) => !cfg!.input_source.rules.some((r) => r.bundle_id === a.bundle_id)) as a (a.bundle_id)}
                <option value={a.bundle_id}>{a.name}</option>
              {/each}
            </select>
            <button class="btn" disabled={!ruleApp} onclick={addRule}>添加</button>
          </div>
        </div>
        <div class="row sub">
          <div class="label"><span>输入法指示器</span><span class="hint">在当前窗口角落显示正在使用的输入法</span></div>
          <div class="btns">
            <select class="select" bind:value={cfg.input_source.indicator} onchange={() => save()}>
              <option value="off">不显示</option>
              <option value="switch">切换时显示</option>
              <option value="always">常驻显示</option>
            </select>
            {#if cfg.input_source.indicator !== "off"}
              <select class="select" bind:value={cfg.input_source.corner} onchange={() => save()}>
                {#each CORNERS as [v, label] (v)}<option value={v}>{label}</option>{/each}
              </select>
            {/if}
          </div>
        </div>
        {#if cfg.input_source.indicator !== "off"}
          {#each st.sources as s (s.id)}
            <div class="row sub">
              <div class="label"><span class="hint">「{s.name}」指示器颜色</span></div>
              <input
                type="color"
                value={(cfg.input_source.colors[s.id] ?? "#333333").slice(0, 7)}
                oninput={(e) => setColor(s.id, (e.currentTarget as HTMLInputElement).value)}
              />
            </div>
          {/each}
        {/if}
      {/if}
    </div>
  </section>

  <section>
    {@render featureHead("switcher", "窗口切换")}
    <div class="card">
      <div class="row">
        <div class="label">
          <span>按住修饰键 + Tab 在窗口间切换</span>
          {@render status("switcher")}
          {#if !st.errors.switcher && !msg.switcher && !running("switcher")}
            <span class="hint">松开修饰键切换到选中的窗口；⇧ 反向，Esc 取消</span>
          {/if}
        </div>
        <input type="checkbox" class="switch" bind:checked={cfg.switcher.enabled} onchange={() => save()} />
      </div>
      {#if cfg.switcher.enabled}
        <div class="row sub">
          <div class="label"><span>快捷键</span></div>
          <select class="select" bind:value={cfg.switcher.modifier} onchange={() => save()}>
            {#each MODIFIERS as [v, label] (v)}<option value={v}>{label}</option>{/each}
          </select>
        </div>
        <div class="row sub">
          <div class="label"><span>只显示当前屏幕的窗口</span></div>
          <input type="checkbox" class="switch" bind:checked={cfg.switcher.current_screen_only} onchange={() => save()} />
        </div>
        <div class="row sub">
          <div class="label"><span>显示最小化的窗口</span><span class="hint">排在列表最后</span></div>
          <input type="checkbox" class="switch" bind:checked={cfg.switcher.show_minimized} onchange={() => save()} />
        </div>
        <div class="row sub">
          <div class="label"><span>显示已隐藏应用的窗口</span><span class="hint">排在列表最后</span></div>
          <input type="checkbox" class="switch" bind:checked={cfg.switcher.show_hidden} onchange={() => save()} />
        </div>
        {#each cfg.switcher.excluded as b (b)}
          <div class="row sub">
            <div class="label"><span>不显示：{appName(b)}</span>{#if appName(b) !== b}<span class="hint mono">{b}</span>{/if}</div>
            <button class="btn plain icon sm remove" title="删除" onclick={() => removeExcluded(b)}>×</button>
          </div>
        {/each}
        <div class="row sub">
          <div class="label"><span class="hint">不在切换列表中显示的应用</span></div>
          <div class="btns">
            <select class="select" bind:value={excludeApp}>
              <option value="">选择正在运行的应用…</option>
              {#each apps.filter((a) => !cfg!.switcher.excluded.includes(a.bundle_id)) as a (a.bundle_id)}
                <option value={a.bundle_id}>{a.name}</option>
              {/each}
            </select>
            <button class="btn" disabled={!excludeApp} onclick={addExcluded}>添加</button>
          </div>
        </div>
      {/if}
    </div>
  </section>

  <section>
    {@render featureHead("music_block", "音乐")}
    <div class="card">
      <div class="row">
        <div class="label">
          <span>阻止「音乐」自动打开</span>
          {@render status("music_block")}
          {#if !msg.music_block && !running("music_block")}
            <span class="hint">连接耳机、按播放键时不再拉起「音乐」；手动打开也会被关掉</span>
          {/if}
        </div>
        <input type="checkbox" class="switch" bind:checked={cfg.music_block.enabled} onchange={() => save()} />
      </div>
    </div>
  </section>
{/if}
