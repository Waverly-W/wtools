# wtools

轻量的 macOS 启动器工具箱。技术栈：Tauri 2 + Rust + Svelte 5，优先级：性能 > 功能 > 外观。

## 开发

```bash
pnpm install
pnpm tauri dev          # 开发模式
pnpm tauri build --bundles app   # 打包 .app（产物在 src-tauri/target/release/bundle/macos/）
pnpm release                     # 打包 → 安装到 /Applications → 重启 wtools
cd src-tauri && cargo test       # Rust 单测（加 -- --ignored --nocapture 可在本机真实扫描应用并打印搜索结果）
node --test src/json/core.test.ts   # JSON 核心库单测（Node 22.6+ 直接跑 TS）
```

浏览器调试界面（不启动 Tauri）：`pnpm dev` 后打开 http://localhost:1420 ，会自动加载 `src/dev/tauriMock.ts` 假数据；
控制台执行 `__mockEmit("launcher:show", "clipboard")` 切到剪贴板视图；JSON 工具打开 http://localhost:1420/json.html ；
截图遮罩打开 http://localhost:1420/shot.html （加 `?mode=ocr` 为截图识别文字，控制台 `__mockShot()` 重新开始）。
模拟层只在开发模式加载，生产包不包含。

默认快捷键 `⌥Space` 呼出搜索框；菜单栏图标 → 设置，可为任意命令绑定全局快捷键。
配置文件：`~/Library/Application Support/com.wtools.app/config.json`
剪贴板数据：`~/Library/Application Support/com.wtools.app/clipboard/`
截图需要「屏幕录制」权限，自动粘贴需要「辅助功能」权限。首次打包前运行一次 `pnpm setup-signing`，
在本机钥匙串创建自签名证书（「wtools Local Signing」），之后 `pnpm release` 自动用它签名，重新打包权限不会失效；
没有证书时用临时签名，每次打包后都要重新授权。

系统增强（设置 → 系统增强，每项单独开关、默认关闭）：系统监控（替代 Stats）、鼠标滚动（替代 Mos）、
输入法自动切换（替代 Input Source Pro / TypeRelay）、窗口切换（替代 AltTab）、阻止「音乐」自动打开（替代 noTunes）。
前四项可一键「从原应用导入设置」（TypeRelay、noTunes 除外）；
原应用仍在运行时设置页会提示，两者同时开启会重复生效（例如滚动反向被反转两次）。
滚动、输入法、窗口切换需要「辅助功能」权限；窗口切换读取窗口标题需要「屏幕录制」权限（截图已申请）。

## 设计规范

所有界面共用一套设计令牌和组件样式（`src/styles/base.css`、`src/styles/components.css`），规则见 [DESIGN.md](DESIGN.md)。新增界面只使用其中的变量和组件类。

## 经验总结

开发过程中的关键决策、踩过的坑、测试方法和资源数据见 [docs/01_wtools复盘.md](docs/01_wtools复盘.md)。

## 结构

```
src-tauri/src/
  lib.rs          启动流程：插件、状态、托盘、快捷键
  config.rs       配置读写（JSON，原子写入）
  hotkey.rs       全局快捷键注册 / 分发 / 冲突检测 / 录制时暂停
  window.rs       搜索框常驻预热（只隐藏不销毁）、工具窗口按需创建
  tray.rs         菜单栏图标
  matcher.rs      模糊匹配 + 拼音（全拼 / 首字母）+ 英文缩写，标题权重高于附加关键词
  usage.rs        使用频率与最近使用加权（usage.json）
  icons.rs        wicon:// 协议：按需渲染应用图标并缓存到 ~/Library/Caches
  commands.rs     暴露给前端的 invoke 命令
  modules/        功能模块（每个功能一个模块）
    mod.rs        Module 协议与注册表
    apps.rs       应用搜索：扫描 .app、读取中文名、FSEvents 监听自动重建
    clipboard/    剪贴板历史
      mod.rs      监听（轮询 changeCount）、多条组合粘贴、粘贴第 N 条、切换数据目录
      store.rs    SQLite：收藏/标签、类型筛选、按条数和天数清理、OCR 结果缓存（可搜索）
      commands.rs 前端命令（耗时操作为 async）
      actions.rs  自定义操作：打开网址 / 运行命令，一键检测
      pasteboard.rs 系统剪贴板读写、模拟 ⌘V（需辅助功能权限）
  ocr.rs          OCR：Vision 识别 + 系统分词，网址/电话/金额/日期保持完整
    json.rs       JSON 工具：命令入口、按需打开窗口、传递要载入的内容（计算都在前端）
    screenshot/   截图：遮罩窗口（首次使用时创建，之后只隐藏）、复制 / 保存 / 识别文字
      capture.rs  CoreGraphics 截取鼠标所在显示器、窗口位置列表（窗口吸附）、屏幕录制权限
    builtin.rs    wtools 自身命令
    system.rs     系统命令
  enhance/        系统增强：常驻后台的系统级功能，配置在 config.json 的 enhance 字段
    mod.rs        配置、按变化启停、设置页命令、原应用冲突检测
    import.rs     从 Stats / Mos / Input Source Pro / AltTab 导入设置（TypeRelay、noTunes 只做冲突提示）
    tap.rs        CGEventTap 封装：每个拦截独立线程 + RunLoop，超时被停用时自动恢复
    ax.rs         辅助功能（AX）接口封装：窗口位置 / 标题 / 最小化、AX 通知
    stats.rs      菜单栏系统监控：mach / sysctl 采样，AppKit 绘制模板图
    scroll.rs     鼠标平滑滚动 + 反向：吞掉滚轮事件，动画线程约 120Hz 补发像素滚动
    input_source.rs  按应用切换输入法（TIS）+ 窗口角落指示器（原生 NSPanel）
    switcher.rs   窗口切换器：按键拦截、窗口列表（可见窗口按前后顺序，最小化 / 隐藏的并行读取）、SkyLight 切换窗口
    switcher_ui.rs  窗口切换器界面（原生 NSPanel，预先创建、不抢焦点、不占网页进程）
    music_block.rs  阻止「音乐」自动打开：监听应用启动通知，是「音乐」就强制退出（替代 noTunes）
src/
  launcher/       搜索框窗口（index.html）：App 负责视图切换，SearchView / ClipboardView / OcrPanel
  settings/       设置窗口（settings.html）
  shot/           截图遮罩（shot.html）：框选、窗口吸附、放大镜取色、标注工具（shapes.ts）
  json/           JSON 工具窗口（json.html）
    core.ts       宽松解析（注释/尾逗号/单引号/转义过的 JSON）、大整数不丢精度、格式化、转义、结构对比
    editor.ts     CodeMirror 配置（只挑需要的扩展）
    FormatTab / DiffTab  格式化页、对比页（对比页按需加载）
  lib/            invoke 封装、快捷键格式化、OcrPanel（剪贴板与截图共用的识别结果面板）
```

## 新增功能模块

1. 在 `src-tauri/src/modules/` 新建文件，实现 `Module`：
   - `commands()` 返回静态命令：自动进入搜索，也自动出现在设置页、可绑定快捷键
   - `search()` 返回动态结果（如应用列表）
   - `execute()` 执行命令
2. 在 `lib.rs` 的 `Registry::new(vec![...])` 中注册。
3. 需要独立界面的功能（如 JSON 工具）：新增一个 `xxx.html` 入口 + `vite.config.ts` 的 input，
   在 Rust 端用 `WebviewWindowBuilder` 按需创建，关闭即销毁，不常驻内存。
