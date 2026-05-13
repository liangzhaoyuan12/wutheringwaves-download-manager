use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub api_url: String,
    pub app_id: String,
}

pub fn server_configs() -> HashMap<&'static str, ServerConfig> {
    let mut m = HashMap::new();
    m.insert(
        "cn",
        ServerConfig {
            api_url: "https://prod-cn-alicdn-gamestarter.kurogame.com/launcher/game/G152/10003_Y8xXrXk65DqFHEDgApn3cpK5lfczpFx5/index.json".into(),
            app_id: "10003".into(),
        },
    );
    m.insert(
        "global",
        ServerConfig {
            api_url: "https://prod-alicdn-gamestarter.kurogame.com/launcher/game/G153/50004_obOHXFrFanqsaIEOmuKroCcbZkQRBC7c/index.json".into(),
            app_id: "50004".into(),
        },
    );
    m.insert(
        "bilibili",
        ServerConfig {
            api_url: "https://prod-cn-alicdn-gamestarter.kurogame.com/launcher/game/G152/10004_j5GWFUuFlb8N31Wi2uS3ZAVHcb7ZGN7y/index.json".into(),
            app_id: "10004".into(),
        },
    );
    m
}

pub fn appid_to_server() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("10003", "cn");
    m.insert("50004", "global");
    m.insert("10004", "bilibili");
    m
}

pub fn server_diff_files() -> HashMap<&'static str, Vec<&'static str>> {
    let mut m = HashMap::new();
    m.insert(
        "cn",
        vec![
            "Client/Binaries/Win64/kuro_login.dll",
            "Client/Content/Paks/pakchunk1-Kuro-Win64-Shipping.pak",
        ],
    );
    m.insert(
        "bilibili",
        vec![
            "Client/Binaries/Win64/bilibili_sdk.dll",
            "Client/Content/Paks/pakchunk1-Bilibili-Win64-Shipping.pak",
        ],
    );
    m.insert(
        "global",
        vec![
            "Client/Binaries/Win64/kuro_login.dll",
            "Client/Content/Paks/pakchunk1-Kuro-Win64-Shipping.pak",
        ],
    );
    m
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_available_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dx_mode: Option<String>,
}

fn home_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
}

pub fn get_config_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("ww_manager")
        } else {
            home_dir().join("AppData").join("Roaming").join("ww_manager")
        }
    } else {
        home_dir().join(".config").join("ww_manager")
    }
}

pub fn get_config_file() -> PathBuf {
    get_config_dir().join("config.json")
}

pub fn load_app_config() -> AppConfig {
    let config_file = get_config_file();
    if !config_file.exists() {
        return AppConfig::default();
    }
    match std::fs::read_to_string(&config_file) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(e) => {
            log::warn!("无法加载配置文件 {:?}: {}", config_file, e);
            AppConfig::default()
        }
    }
}

pub fn save_app_config(config: &AppConfig) {
    let config_dir = get_config_dir();
    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        log::error!("无法创建配置目录: {}", e);
        return;
    }
    let config_file = get_config_file();
    match serde_json::to_string_pretty(config) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&config_file, content) {
                log::error!("无法保存配置 {:?}: {}", config_file, e);
            }
        }
        Err(e) => {
            log::error!("序列化配置失败: {}", e);
        }
    }
}
