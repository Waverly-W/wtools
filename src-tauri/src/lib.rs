mod autostart;
mod commands;
mod config;
mod enhance;
mod hotkey;
mod icons;
mod matcher;
mod ocr;
mod modules;
mod tray;
mod usage;
mod window;

use tauri::{Manager, RunEvent, WindowEvent};

/// 命令行子命令：`wtools --ocr <图片路径>` 输出识别结果 JSON。
/// 返回 true 表示已作为子命令处理完毕，调用方应直接退出。
pub fn run_cli() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(ocr::CLI_FLAG) {
        let code = match args.get(2).ok_or_else(|| "缺少图片路径".to_string()).and_then(|p| ocr::cli(p)) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
        std::process::exit(code);
    }
    false
}

pub fn run() {
    // 异步命令只负责把耗时工作交给阻塞线程池，2 个工作线程足够（默认按 CPU 核数开）
    if let Ok(rt) = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        tauri::async_runtime::set(rt.handle().clone());
        std::mem::forget(rt);
    }
    let app = tauri::Builder::default()
        // 重复启动时只唤起已有实例的搜索框
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            window::show_launcher(app, window::SEARCH_VIEW);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(hotkey::on_shortcut)
                .build(),
        )
        .register_asynchronous_uri_scheme_protocol(icons::SCHEME, icons::handle)
        .register_asynchronous_uri_scheme_protocol(
            modules::clipboard::SCHEME,
            modules::clipboard::handle_protocol,
        )
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::execute,
            commands::hide_launcher,
            commands::list_hotkeys,
            commands::set_hotkey,
            commands::suspend_hotkeys,
            commands::set_launcher_view,
            autostart::autostart_status,
            autostart::autostart_set,
            autostart::autostart_open_settings,
            enhance::commands::enhance_get,
            enhance::commands::enhance_set,
            enhance::commands::enhance_quit_original,
            enhance::commands::enhance_retry,
            enhance::commands::enhance_import,
            enhance::commands::enhance_running_apps,
            enhance::commands::enhance_open_accessibility,
            modules::clipboard::commands::clipboard_list,
            modules::clipboard::commands::clipboard_detail,
            modules::clipboard::commands::clipboard_tags,
            modules::clipboard::commands::clipboard_paste,
            modules::clipboard::commands::clipboard_paste_text,
            modules::clipboard::commands::clipboard_delete,
            modules::clipboard::commands::clipboard_favorite,
            modules::clipboard::commands::clipboard_set_tags,
            modules::clipboard::commands::clipboard_clear,
            modules::clipboard::commands::clipboard_ocr,
            modules::clipboard::commands::clipboard_reveal,
            modules::clipboard::commands::clipboard_run_action,
            modules::clipboard::commands::clipboard_test_action,
            modules::clipboard::commands::clipboard_get_settings,
            modules::clipboard::commands::clipboard_set_settings,
            modules::clipboard::commands::clipboard_choose_dir,
            modules::clipboard::commands::clipboard_set_dir,
            modules::json::commands::json_take_pending,
            modules::json::commands::json_open,
            modules::json::commands::json_clipboard_text,
            modules::screenshot::commands::shot_take_pending,
            modules::screenshot::commands::shot_image,
            modules::screenshot::commands::shot_show,
            modules::screenshot::commands::shot_cancel,
            modules::screenshot::commands::shot_copy,
            modules::screenshot::commands::shot_save,
            modules::screenshot::commands::shot_ocr,
            modules::screenshot::commands::shot_text,
        ])
        .setup(|app| {
            // 只在菜单栏显示，不占 Dock
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let paths = app.path();
            if let Ok(dir) = paths.app_cache_dir() {
                icons::init(dir);
            }
            let data_dir = paths
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("wtools"));
            let usage_file = data_dir.join("usage.json");
            app.manage(config::ConfigStore::load(&handle));
            let clipboard = modules::clipboard::ClipboardService::open(
                data_dir.join("clipboard"),
                app.state::<config::ConfigStore>().get().clipboard.data_dir,
            )?;
            clipboard.start_watcher(handle.clone());
            app.manage(clipboard);
            app.manage(modules::Registry::new(
                vec![
                    Box::new(modules::apps::Apps::new()),
                    Box::new(modules::clipboard::ClipboardModule),
                    Box::new(modules::json::JsonModule),
                    Box::new(modules::screenshot::ScreenshotModule),
                    Box::new(modules::builtin::Builtin),
                    Box::new(modules::system::System),
                ],
                usage::Usage::load(usage_file),
            ));
            app.manage(hotkey::HotkeyState::default());

            window::setup_launcher(&handle);
            tray::setup(&handle)?;
            hotkey::apply_all(&handle);
            enhance::init(&handle);
            Ok(())
        })
        .on_window_event(|win, event| {
            // 搜索框失焦即隐藏（焦点已在别的应用，无需归还）
            if win.label() == window::LAUNCHER {
                if let WindowEvent::Focused(false) = event {
                    window::hide_launcher(win.app_handle(), false);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building wtools");

    app.run(|app, event| match event {
        // 关闭最后一个可见窗口时不退出，常驻菜单栏；只有显式 app.exit() 才退出
        RunEvent::ExitRequested { api, code, .. } if code.is_none() => api.prevent_exit(),
        // 已在运行时再次点击应用图标（访达 / 启动台 / 程序坞）：系统不会启动新进程，只发送「重新打开」事件
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => window::show_launcher(app, window::SEARCH_VIEW),
        _ => {}
    });
}
