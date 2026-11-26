//! 菜单栏图标

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle,
};

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id("show", "打开搜索框").build(app)?;
    let shot = MenuItemBuilder::with_id("shot", "截图").build(app)?;
    let shot_ocr = MenuItemBuilder::with_id("shot_ocr", "截图识别文字").build(app)?;
    let settings = MenuItemBuilder::with_id("settings", "设置…")
        .accelerator("Cmd+,")
        .build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出 wtools")
        .accelerator("Cmd+Q")
        .build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&show, &shot, &shot_ocr, &settings])
        .separator()
        .item(&quit)
        .build()?;

    TrayIconBuilder::with_id("main")
        .icon(tauri::include_image!("icons/tray.png"))
        .icon_as_template(true)
        .tooltip("wtools")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => crate::window::show_launcher(app, crate::window::SEARCH_VIEW),
            "shot" | "shot_ocr" => {
                let mode = if event.id().as_ref() == "shot" { "capture" } else { "ocr" };
                // 等菜单收起后再截图，避免把菜单截进去
                let app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    if let Err(e) = crate::modules::screenshot::start(&app, mode) {
                        eprintln!("[screenshot] {e}");
                    }
                });
            }
            "settings" => {
                let _ = crate::window::open_settings(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
