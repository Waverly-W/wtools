// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 子命令模式（如 OCR 子进程）处理完直接退出，不启动界面
    if wtools_lib::run_cli() {
        return;
    }
    wtools_lib::run()
}
