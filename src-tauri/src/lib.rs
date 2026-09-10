mod commands;
mod config;
mod error;
mod manager;
mod md5_cache;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::start_sync,
            commands::start_download,
            commands::start_checkout,
            commands::start_predownload,
            commands::start_update,
            commands::launch_game,
            commands::delete_game,
            commands::get_app_config,
            commands::save_app_config_cmd,
            commands::get_server_list,
            commands::get_cdn_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
