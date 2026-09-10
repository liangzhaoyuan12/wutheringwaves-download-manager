use std::path::PathBuf;

use tauri::Emitter;

use crate::config;
use crate::manager::{GameManager, ProgressEvent};

fn resolve_game_path(path_arg: Option<String>) -> Result<PathBuf, String> {
    if let Some(p) = path_arg {
        let path = PathBuf::from(&p);
        let mut cfg = config::load_app_config();
        let resolved = path.canonicalize().unwrap_or(path);
        if cfg.default_path.as_deref() != Some(&resolved.to_string_lossy()) {
            cfg.default_path = Some(resolved.to_string_lossy().to_string());
            config::save_app_config(&cfg);
        }
        Ok(resolved)
    } else {
        let cfg = config::load_app_config();
        cfg.default_path
            .map(PathBuf::from)
            .ok_or_else(|| "未设置游戏路径。请在设置中配置或传入 path 参数。".to_string())
    }
}

fn validate_server(server: &str) -> Result<(), String> {
    let valid_servers = ["cn", "global", "bilibili"];
    if !valid_servers.contains(&server) {
        return Err(format!(
            "无效的服务器类型: {}。可选: cn, global, bilibili",
            server
        ));
    }
    Ok(())
}

fn detect_server(game_path: &PathBuf) -> String {
    let cfg_file = game_path.join("launcherDownloadConfig.json");
    if let Ok(content) = std::fs::read_to_string(&cfg_file) {
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(app_id) = data.get("appId").and_then(|v| v.as_str()) {
                if let Some(server) = config::appid_to_server().get(app_id) {
                    return server.to_string();
                }
            }
        }
    }
    "cn".to_string()
}

// ── Commands ──

#[tauri::command]
pub async fn get_status(
    app_handle: tauri::AppHandle,
    game_path: Option<String>,
) -> Result<serde_json::Value, String> {
    let path = resolve_game_path(game_path)?;
    let cfg_file = path.join("launcherDownloadConfig.json");

    if !cfg_file.exists() {
        return Ok(serde_json::json!({
            "path": path.to_string_lossy(),
            "server": "未知",
            "version": "未知",
            "hasConfig": false,
        }));
    }

    let content = std::fs::read_to_string(&cfg_file)
        .map_err(|e| format!("读取配置文件失败: {}", e))?;
    let data: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析配置文件失败: {}", e))?;

    let app_id = data.get("appId").and_then(|v| v.as_str()).unwrap_or("");
    let server = config::appid_to_server()
        .get(app_id)
        .copied()
        .unwrap_or("未知");

    let _ = app_handle.emit(
        "ww:progress",
        ProgressEvent {
            event_type: "log".into(),
            message: format!(
                "目录: {:?}\n服务器: {}\n版本: {}",
                path,
                server,
                data.get("version").and_then(|v| v.as_str()).unwrap_or("未知")
            ),
            current: 0,
            total: 0,
            file_name: None,
        },
    );

    Ok(serde_json::json!({
        "path": path.to_string_lossy(),
        "server": server,
        "version": data.get("version").and_then(|v| v.as_str()).unwrap_or("未知"),
        "appId": app_id,
        "hasConfig": true,
    }))
}

#[tauri::command]
pub async fn start_sync(
    app_handle: tauri::AppHandle,
    game_path: Option<String>,
    server: Option<String>,
    cdn_url: Option<String>,
) -> Result<String, String> {
    let path = resolve_game_path(game_path)?;
    let server = server.unwrap_or_else(|| detect_server(&path));

    let mut mgr = GameManager::new(path, &server, app_handle, cdn_url).map_err(|e| e.to_string())?;
    mgr.sync_files(true).await.map_err(|e| e.to_string())?;

    Ok("同步完成".into())
}

#[tauri::command]
pub async fn start_download(
    app_handle: tauri::AppHandle,
    game_path: Option<String>,
    server: String,
    cdn_url: Option<String>,
) -> Result<String, String> {
    let path = resolve_game_path(game_path)?;
    validate_server(&server)?;

    let mut mgr = GameManager::new(path, &server, app_handle, cdn_url).map_err(|e| e.to_string())?;
    mgr.download_full().await.map_err(|e| e.to_string())?;

    Ok(format!("{} 服下载完成", server))
}

#[tauri::command]
pub async fn start_checkout(
    app_handle: tauri::AppHandle,
    game_path: Option<String>,
    server: String,
    cdn_url: Option<String>,
) -> Result<String, String> {
    let path = resolve_game_path(game_path)?;
    validate_server(&server)?;

    let mut mgr = GameManager::new(path, &server, app_handle, cdn_url).map_err(|e| e.to_string())?;
    mgr.checkout(&server, true)
        .await
        .map_err(|e| e.to_string())?;

    Ok(format!("已切换到 {} 服", server))
}

#[tauri::command]
pub async fn start_predownload(
    app_handle: tauri::AppHandle,
    game_path: Option<String>,
    server: Option<String>,
    cdn_url: Option<String>,
) -> Result<String, String> {
    let path = resolve_game_path(game_path)?;
    let server = server.unwrap_or_else(|| detect_server(&path));

    let mut mgr = GameManager::new(path, &server, app_handle, cdn_url).map_err(|e| e.to_string())?;
    mgr.download_predownload()
        .await
        .map_err(|e| e.to_string())?;

    Ok("预下载完成".into())
}

#[tauri::command]
pub async fn start_update(
    app_handle: tauri::AppHandle,
    game_path: Option<String>,
    server: Option<String>,
    cdn_url: Option<String>,
) -> Result<String, String> {
    let path = resolve_game_path(game_path)?;
    let server = server.unwrap_or_else(|| detect_server(&path));

    let mut mgr = GameManager::new(path, &server, app_handle, cdn_url).map_err(|e| e.to_string())?;
    mgr.update_game().await.map_err(|e| e.to_string())?;

    Ok("更新完成".into())
}

#[tauri::command]
pub async fn launch_game(
    game_path: Option<String>,
    dx_mode: Option<String>,
) -> Result<serde_json::Value, String> {
    let path = resolve_game_path(game_path)?;
    let exe_path = path.join("Client/Binaries/Win64/Client-Win64-Shipping.exe");

    // Persist DX mode
    if let Some(ref mode) = dx_mode {
        let mut cfg = config::load_app_config();
        cfg.dx_mode = Some(mode.clone());
        config::save_app_config(&cfg);
    }

    #[cfg(target_os = "windows")]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                hwnd: *mut std::ffi::c_void,
                lpOperation: *const u16,
                lpFile: *const u16,
                lpParameters: *const u16,
                lpDirectory: *const u16,
                nShowCmd: i32,
            ) -> isize;
        }

        if exe_path.exists() {
            use std::os::windows::ffi::OsStrExt;
            use std::ffi::OsStr;

            let operation: Vec<u16> = OsStr::new("open").encode_wide().chain(std::iter::once(0)).collect();
            let exe_wide: Vec<u16> = exe_path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            let dir_wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();

            let params_str = dx_mode.as_deref().and_then(|mode| {
                if mode == "dx11" {
                    Some("-dx11")
                } else if mode == "dx12" {
                    Some("-dx12")
                } else {
                    None
                }
            }).unwrap_or("");

            let params_wide: Vec<u16> = if params_str.is_empty() {
                Vec::new()
            } else {
                OsStr::new(params_str).encode_wide().chain(std::iter::once(0)).collect()
            };

            let result = unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    operation.as_ptr(),
                    exe_wide.as_ptr(),
                    if params_wide.is_empty() { std::ptr::null() } else { params_wide.as_ptr() },
                    dir_wide.as_ptr(),
                    1, // SW_SHOWNORMAL
                )
            };

            // ShellExecuteW returns a value > 32 on success
            if result as isize <= 32 {
                return Err(format!("无法启动游戏: 错误代码 {}", result));
            }
            return Ok(serde_json::json!({ "platform": "windows", "launched": true }));
        } else {
            return Err(format!("找不到游戏可执行文件: {}", exe_path.display()));
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let exe_exists = exe_path.exists();
        return Ok(serde_json::json!({
            "platform": "linux",
            "exe_exists": exe_exists,
            "exe_path": exe_path.to_string_lossy(),
        }));
    }
}

#[tauri::command]
pub async fn delete_game(
    app_handle: tauri::AppHandle,
    game_path: Option<String>,
) -> Result<String, String> {
    let path = resolve_game_path(game_path)?;
    let mut mgr = GameManager::new(path.clone(), "cn", app_handle, None).map_err(|e| e.to_string())?;
    mgr.delete_all_files().await.map_err(|e| e.to_string())?;
    Ok("游戏已删除".into())
}

#[tauri::command]
pub async fn get_cdn_list(
    server: String,
) -> Result<Vec<crate::manager::CdnNode>, String> {
    validate_server(&server)?;
    GameManager::fetch_cdn_list(&server)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_app_config() -> Result<config::AppConfig, String> {
    Ok(config::load_app_config())
}

#[tauri::command]
pub async fn save_app_config_cmd(config_data: config::AppConfig) -> Result<(), String> {
    config::save_app_config(&config_data);
    Ok(())
}

#[tauri::command]
pub async fn get_server_list() -> Result<Vec<serde_json::Value>, String> {
    let list: Vec<_> = config::server_configs()
        .iter()
        .map(|(key, cfg)| {
            serde_json::json!({
                "key": key,
                "appId": cfg.app_id,
            })
        })
        .collect();
    Ok(list)
}
