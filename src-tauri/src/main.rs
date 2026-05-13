// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    unsafe {
            // nv驱动下需设置环境变量才可启动
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            // std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
    wutheringwaves_download_manager_gui_lib::run()
}
