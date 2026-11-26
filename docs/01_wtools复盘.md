# wtools 复盘：轻量复刻 macOS 工具的经验与踩坑

| 项 | 内容 |
|---|---|
| 周期 | 2026-11-19（调研）→ 2026-11-25（窗口切换原生化、集成 TypeRelay / noTunes） |
| 技术栈 | Tauri 2 + Svelte 5 + Vite；Rust 侧 objc2 / objc2-app-kit 直接调用 AppKit、CoreGraphics、AX、SkyLight |
| 复刻对象 | uTools（搜索、剪贴板、截图 OCR、JSON 工具）、Stats、Mos、Input Source Pro、TypeRelay、AltTab、noTunes |
| 依据 | 开发会话日志（含 4 段阶段摘要）、实测数据、[README.md](../README.md)、[DESIGN.md](../DESIGN.md) |
| 事实优先级 | 实测数据 > 会话中核实过的报错原文 > 阶段摘要；标「推测」的未经验证 |
| 配套文档 | [README.md](../README.md)（结构、开发命令）、[DESIGN.md](../DESIGN.md)（设计令牌与组件） |

## 一句话结论

用 Tauri 2 加上 objc2 直接调系统 API，可以用一个进程替代 7 个常驻小工具：wtools 主进程稳态约 34MB，算上 WebKit 辅助进程约 57–64MB；估计原来 7 个工具常驻合计约 200MB。最重要的经验有两条：

1. **网页只用在复杂、按需打开的界面上**。常驻、高频弹出的小界面直接用原生控件写。
2. **每次签名必须稳定**，否则每次打包都要重新授予系统权限。

## 最值得带走的 10 条经验

| # | 经验 | 出处 |
|---|---|---|
| 1 | **先盘点用户真正在用的功能，再动手。** 读原应用的配置（`defaults read <bundle id>`）就知道用户开了哪些功能。只复刻这些，并提供「从原应用导入设置」 | 集成 Stats / Mos / AltTab 等 |
| 2 | **按场景选界面技术**：复杂、按需打开的界面（JSON 对比、设置）用 WebView；常驻、频繁弹出的小界面（窗口切换、输入法指示器）用原生 NSPanel。窗口切换器改成原生后少了一个 WebContent 进程，合计内存从约 100MB 降到约 57MB | 窗口切换器 |
| 3 | **先固定签名身份再做功能**：ad-hoc 签名每次编译都会变，辅助功能、屏幕录制权限就得重新授予。在登录钥匙串放一张自签名证书就能解决，不用管理员密码，也不用设置信任 | 打包流程 |
| 4 | **常驻窗口只隐藏、不销毁**：启动器预热常驻；工具窗口用时才创建；截图遮罩第一次使用时创建，之后只隐藏 | window.rs |
| 5 | **重的系统框架放到子进程里**：Vision OCR 放在主进程会让常驻内存从 29MB 涨到 70MB；改成用子进程执行 `wtools --ocr`，主进程回到 25MB，代价是每次识别多约 0.8s | ocr.rs |
| 6 | **后台线程要能真正睡着**：事件拦截线程改为无限阻塞，停止时再唤醒；tokio 工作线程限制为 2；轮询和拦截回调都包一层 autoreleasepool。空闲唤醒从每 10 秒 131 次降到每 15 秒约 5 次 | tap.rs、lib.rs |
| 7 | **测内存要测稳态**：刚启动 27MB，约 7 分钟后回到 34MB。只报刚启动的数字会夸大优化效果 | 资源优化 |
| 8 | **系统 API 慢就拆开并行**：窗口列表原来逐个应用串行查询 AX，要 559ms；改成屏幕上的窗口走 CGWindowList、最小化和隐藏的窗口并行查 AX，降到 18–21ms | switcher.rs |
| 9 | **替代某个工具时，同时检测原工具是否还在运行**：两边同时运行会重复生效（例如反向滚动被反转两次、输入法被切两次）。设置页提示冲突，并提供「退出原应用」按钮 | enhance/mod.rs |
| 10 | **从第一天定好设计令牌**：颜色、字号、间距、圆角都做成令牌，每个令牌有浅色、深色两套值，页面只管布局。原生界面按同一组数值手动对齐 | DESIGN.md |

## 时间线

| 日期 | 阶段 | 交付 |
|---|---|---|
| 11-19 | 调研 + 基座 | 选定 Tauri 2；做完基座（包 4.9MB，空闲约 54MB）、应用搜索（拼音、中文名）、剪贴板历史（OCR、标签、自定义操作） |
| 11-24 上午 | 剪贴板 UI 改版 | 窗口改为 1080×780；单栏 / 双栏；去掉详情栏，改用右键菜单 |
| 11-24 下午 | 工具 + 工程化 | JSON 工具（CodeMirror 6）、截图标注 + OCR；`pnpm release`、开机自启、自签名证书；设置页改为侧栏 Tab |
| 11-24 晚 | 系统增强 | 替代 Stats、Mos、Input Source Pro、AltTab；用 mole 卸载原应用 |
| 11-25 | 规范 + 减法 | DESIGN.md 设计规范；删除调试日志、降低资源占用；窗口切换器改为原生 NSPanel；集成 TypeRelay 冲突提示、noTunes |

## 关键设计决策

| 决策 | 理由与数据 | 否决的方案 |
|---|---|---|
| Tauri 2 | 空闲约 30–80MB，Electron 为 150–400MB；实测基座 4.9MB 包、空闲 54MB、CPU 0% | Electron（太重）；纯 Swift / GPUI（只比 Tauri 少约 30MB，但 JSON 对比这类复杂界面开发成本高很多） |
| 不用 tauri-nspanel | 普通窗口加上 collectionBehavior、level 就够用，少一个 git 依赖 | tauri-nspanel |
| 窗口切换器用原生 NSPanel | 每次按 ⌃Tab 都会弹出，常驻；少 1 个 WebContent 进程（约 14–21MB） | 网页版（已移到废纸篓） |
| 截图像素用 `ipc::Response` 传原始 RGBA | 省掉 PNG 编码和解码；导出时前端 `canvas.toBlob` 后以 Raw body 传回 Rust | base64 / PNG 传输 |
| 剪贴板每 400ms 轮询 `changeCount` | macOS 没有剪贴板变化通知；轮询只读一个整数，开销很小 | — |
| 用系统自带 SQLite，图片单独存文件 | 包体只多 0.1MB | 内嵌 SQLite |
| JSON 工具用 CodeMirror 6 + merge | 编辑器代码 390KB，懒加载；3MB JSON 解析加格式化约 160ms | Monaco（太大） |
| JSON 用自写的宽松解析器 | `JSON.parse` 会让 19 位 ID 丢精度（结尾变成 …800），自写解析器保留数字原文 | 原生 JSON.parse |
| 开机自启用 `SMAppService.mainAppService` | 开关状态每次都从系统读取，不另存配置，不会和系统设置不一致 | LaunchAgent plist |
| 应用图标走自定义协议 `wicon://` | 用时才生成、缓存到磁盘（53 个共 420KB） | 启动时生成全部图标 |
| 「阻止音乐自动打开」只监听应用启动通知 | 只靠通知触发，不轮询；实测约 60ms 内关掉「音乐」 | 定时扫描进程 |

## 踩坑清单

### macOS 权限与签名

| 现象 | 根因 | 解决 |
|---|---|---|
| 每次打包后都要重新授予辅助功能、屏幕录制权限 | ad-hoc 签名的 designated requirement 基于 cdhash，每次编译都会变 | `scripts/setup-signing.sh` 用 openssl 生成自签名证书，导入登录钥匙串时加 `-T /usr/bin/codesign`；release 脚本设置 `APPLE_SIGNING_IDENTITY` |
| codesign 报 `no identity found` | 证书所在的钥匙串不在搜索列表里 | 导入登录钥匙串，它默认在搜索列表中，不用设置信任 |
| 换了 app 路径或签名方式后权限失效 | 系统权限按路径 + 签名记录 | 只从 `/Applications` 运行，签名方式固定下来 |
| 截图里只有桌面和菜单栏，看不到其他窗口 | 没有屏幕录制权限 | 先用 `CGPreflightScreenCaptureAccess` 检查，没有就 `CGRequestScreenCaptureAccess` 申请，并直接打开隐私设置页 |

### Tauri 窗口与进程

| 现象 | 根因 | 解决 |
|---|---|---|
| 再点应用图标没反应 | 应用已在运行时，系统只发 `RunEvent::Reopen`，不会触发 single-instance | 在 `app.run` 里处理 Reopen，调用 `show_launcher` |
| ⌘Space 呼出后搜索框立刻消失 | 系统同时切了输入法，搜索框失焦，被「失焦即隐藏」收起 | 换快捷键；设计快捷键时避开系统已占用的组合 |
| 同步 command 让界面卡住 | Tauri 的同步 command 跑在主线程上 | 耗时操作用 async command 加 `spawn_blocking` |
| 新窗口刚创建，前端拿不到数据 | 窗口创建和前端初始化有竞态 | Rust 先把数据放进静态 PENDING，前端在挂载时或收到事件时来取（`*_take_pending`） |
| 托盘图标编译报 `from_bytes` 不存在 | Tauri 2 改了 API | 改用 `tauri::include_image!` |
| 截图遮罩坐标错位 | Cocoa 坐标原点在左下角 | y = 主屏高 − y − h；窗口 level 1000，collectionBehavior 设为 CanJoinAllSpaces \| FullScreenAuxiliary \| Stationary |

### 事件拦截（CGEventTap）与 AX

| 现象 | 根因 | 解决 |
|---|---|---|
| 拦截过一段时间后失效 | tap 回调超时，被系统停用 | 每个 tap 单独一个线程跑 RunLoop，收到停用事件后重新启用 |
| 自己补发的事件又被自己拦下来 | 拦截器分不清事件来源 | 补发时在 field 42（`kCGEventSourceUserData`）写入标记 `0x77746F6F`，遇到就放行 |
| 平滑滚动影响了触控板 | 触控板产生的是连续滚动事件 | 只处理 field 88 = 0 的非连续滚动（鼠标滚轮）；1 格滚轮 = 10px，用 `CGEventCreateScrollWheelEvent2` 约 120Hz 补发 |
| 空闲时 CPU 被频繁唤醒 | 线程每 0.5s 醒来检查停止标志 | 改为 `CFRunLoopRunInMode(…, 1e10)` 无限阻塞；Drop 时循环调用 `CFRunLoopStop`，直到 `is_finished()` |
| 窗口列表要 559ms | 逐个应用串行查询 AX，单个应用最慢 112ms | 屏幕上的窗口走 CGWindowList；最小化、隐藏的窗口用 scoped threads 并行查 AX；AX 超时设为 0.25s；启用功能时预热 |
| 普通方法切不到别的应用的指定窗口 | 公开 API 只能激活整个应用 | dlopen SkyLight 调 `_SLPSSetFrontProcessWithOptions` + `SLPSPostEventRecordTo`，再 `AXRaise`；窗口 ID 用 `_AXUIElementGetWindow` 获取 |

### objc2 / AppKit 原生界面

| 现象 | 根因 | 解决 |
|---|---|---|
| 报 `alloc` / `effectiveAppearance` 找不到 | 这些方法定义在 trait 上 | 导入 `MainThreadOnly` / `AnyThread` / `NSAppearanceCustomization` |
| 原生列表第一次显示时，图标是灰色虚线占位框 | `NSWorkspace iconForFile` 在后台异步加载，加载完后 NSImageView 不会自己重绘 | 出现新图标时，在 0.15s 和 0.5s 后各重新设置一次图片（`performSelector:afterDelay:`） |
| 菜单栏图标每次刷新都要经过 PNG 编解码 | tray-icon 的 `set_icon` 内部会 `to_png` | 通过 `with_inner_tray_icon` 拿到 `NSStatusItem`，用 CGBitmapContext 画好后直接设成 NSImage；内容没变就跳过 |
| 内存慢慢上涨 | 后台线程没有 autoreleasepool，系统临时对象一直攒着 | 轮询和拦截回调都包一层 `autoreleasepool` |
| 有一批 352KB 的内存块去不掉 | `CATintedImage` 的 ColorSync 转换缓存，由系统管理、数量有上限 | 接受。定位方法：`footprint` → `vmmap --summary` → 设置 `MallocStackLogging=lite` 后用 `malloc_history` 查 |

### 前端（Svelte 5 / WebKit）

| 现象 | 根因 | 解决 |
|---|---|---|
| effect 反复触发 | effect 体里读到的状态都会被追踪 | 不需要追踪的读取用 `untrack` 包住 |
| 浏览器 mock 没生效 | 静态 import 先于 mock 安装执行 | 改为动态 `import("./App.svelte")` |
| 马赛克没效果 | WebKit 的 drawImage 遇到源矩形超出图片边界时什么都不画 | 先把源矩形裁到图片范围内 |
| canvas 颜色不对 | `fillStyle` 不一定支持 `color-mix` | 把强调色的 hex 转成 rgba |
| 放大镜不显示 | canvas 只在显示时才挂载，挂载前拿不到元素 | 始终挂载，用 class 控制隐藏 |
| 子组件拿不到父组件的 scoped 样式 | Svelte 的样式作用域只限本组件 | 共享样式放到全局 CSS |

### 应用搜索与剪贴板

| 现象 | 根因 | 解决 |
|---|---|---|
| 别的应用显示英文名（WeCom 而不是企业微信） | 系统按「当前应用自己声明的语言」返回别的应用的名称 | Info.plist 声明 `CFBundleLocalizations` [zh_CN, en]，并直接读应用包里 lproj / loctable 中的中文名 |
| 输入「wx」时 App Store 排在企业微信前面 | App Store 的附加关键词里有「玩游戏」，和应用名同等权重 | 以应用名为主，附加关键词得分 ×3/5；连续子串命中加分 |
| 测试时 `pbcopy` 写进空文本 | shell 是 C locale，中文被丢掉 | 设置 `LANG=en_US.UTF-8` |
| 数据目录放网盘后数据库异常 | WAL 模式不适合同步盘；网盘同步会替换文件 | 改用 DELETE journal 模式；检测到文件 inode 变化时重新打开 |

### 卸载原应用（mole / brew）

| 现象 | 根因 | 解决 |
|---|---|---|
| `mo uninstall` 不能指定要卸载的应用 | 它只有交互界面 | source 它的库，直接调用 `batch_uninstall_applications` |
| 报 `files_cleaned: unbound variable` | 脚本在 `set -u` 下运行，全局变量没声明 | 调用前先声明这些变量 |
| 卸载后还有残留（Widgets 扩展进程、plist、Containers 目录） | mole 扫描不全；Containers 受系统保护 | 卸载后手动检查进程、Preferences、Containers、登录项；Containers 目录要「完全磁盘访问」权限才能删，可以不管 |
| brew 里还记着已删除的应用 | 应用是直接删掉的，没走 brew | 用 `brew uninstall --cask` 清理记录；`--zap` 需要 sudo，交给用户执行 |

## 测试与验证方法

| 方法 | 适用 | 注意 |
|---|---|---|
| 浏览器 mock（`src/dev/tauriMock.ts`） | 检查前端界面 | 新增命令时同步补上 mock；mock 要在 App 动态 import 之前安装 |
| osascript 模拟按键 + `screencapture` | 检查原生界面（如按住 ⌃Tab 截图后按 Esc 取消） | **锁屏时系统会拦截全部模拟按键，截图也是全黑**；可以先读 `ioreg` 的 `CGSSessionScreenIsLocked`、`HIDIdleTime` 判断 |
| AX 直接写入输入框 | 测搜索框 | 模拟按键会落到用户正在用的窗口，也会被中文输入法吞掉 |
| 高频采样进程（每 20ms 跑一次 `pgrep`） | 验证「启动后立刻被关掉」这类很短的行为 | 本机 `log show` 查不到其他进程的日志 |
| `footprint` / `top -l` / `vmmap` / `malloc_history` | 内存、唤醒次数、线程数 | 刚启动和稳态（7 分钟以上）都要测；长时间采样放到后台任务里跑 |
| 自定义 CGWindowList 小工具 `wins <pid>` | 确认面板是否真的显示、位置和大小 | — |

## 资源数据对比

| 时点 | 主进程 | WebKit 辅助进程 | 合计 | 空闲唤醒 | 线程 |
|---|---|---|---|---|---|
| 基座（11-19） | 21MB | 3 个 | 约 54MB | — | — |
| 优化前（11-25） | 35.5MB | GPU 16MB + 网络 6.6MB + 2 个 WebContent 共 42MB | 约 100MB | 约 131 次 / 10s | 22 |
| 减法之后 | 稳态约 34MB | 同上 | — | 约 5 次 / 15s | 16 |
| 切换器原生化之后（刚启动） | 27MB | GPU 10MB + 网络 5.9MB + 1 个 WebContent 14MB | 约 57MB | — | — |

## 协作偏好（与用户）

- 全程用中文。有一次回复误用英文，被用户纠正。
- 优先级：**性能 > 功能 > 外观**；先做基座，再逐步加功能，从易到难。
- 先给可行性分析和取舍，用户确认后再按顺序实现。
- 永久删除、卸载、需要 sudo 的操作，先说清楚范围，等用户确认或由用户自己执行。项目不是 git 仓库，删除文件一律移到废纸篓。
- 汇报要如实：没验证的写「未验证」，数据刚启动和稳态分开写。

## 下次开发轻量复刻工具的检查清单

- [ ] 开工前用 `defaults read <bundle id>` 盘点原工具的配置，只复刻实际在用的功能，并规划「导入设置」和「冲突检测」
- [ ] 第一天就配好自签名证书和 release 脚本（打包 → 装到 /Applications → 重启），之后权限不再丢
- [ ] 每个界面先判断：常驻、高频弹出的用原生，复杂、按需打开的用 WebView；常驻窗口只隐藏不销毁
- [ ] 重的系统框架（Vision、大模型等）放子进程
- [ ] 后台线程一律可阻塞、可停止：RunLoop 无限阻塞 + 显式唤醒，轮询包 autoreleasepool，限制 tokio 线程数
- [ ] 第一天定设计令牌（浅色、深色两套），原生界面按同一组数值对齐
- [ ] 每个依赖系统权限的功能，都要给出「没有权限」时的状态和跳转入口
- [ ] 同时提供浏览器 mock，前端改动不必每次打包验证
- [ ] 每轮优化前后都测 `footprint`、唤醒次数、线程数，刚启动和稳态分开记录
- [ ] 替代原工具后：检测冲突 → 退出原工具 → 移除登录项 → 卸载 → 手动清理残留
