use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::Semaphore;
use url::Url;

use crate::config::{self, ServerConfig};
use crate::error::WwError;
use crate::md5_cache::Md5Cache;

const VERIFY_CONCURRENT: usize = 64;
const DOWNLOAD_CONCURRENT: usize = 20; // 并发下载的文件数（原来 8，瓶颈在此）
const DOWNLOAD_CHUNK_TARGET: u64 = 64 * 1024 * 1024; // 大文件分块目标大小
const DOWNLOAD_CHUNK_MIN: usize = 4;
const DOWNLOAD_CHUNK_MAX: usize = 8; // 单个文件最多并发分块数
const RETRY_COUNT: u32 = 3;
const JSON_TIMEOUT_SECS: u64 = 10;
const DOWNLOAD_READ_TIMEOUT_SECS: u64 = 120;
const MIN_CHUNKED_SIZE: u64 = 4 * 1024 * 1024; // 文件 > 4MB 走分块下载

/// 根据文件大小计算分块数（目标块大小固定，文件越大分块越多）。
fn chunk_count_for(size: u64) -> usize {
    let n = (size + DOWNLOAD_CHUNK_TARGET - 1) / DOWNLOAD_CHUNK_TARGET;
    n.clamp(DOWNLOAD_CHUNK_MIN as u64, DOWNLOAD_CHUNK_MAX as u64) as usize
}

#[derive(Clone, Serialize)]
pub struct ProgressEvent {
    #[serde(rename = "eventType")]
    pub event_type: String,
    pub message: String,
    pub current: u64,
    pub total: u64,
    #[serde(rename = "fileName")]
    pub file_name: Option<String>,
}

#[derive(Clone)]
struct SharedCtx {
    game_folder: PathBuf,
    server_type: String,
    server_config: ServerConfig,
    md5_cache: Md5Cache,
    app_handle: tauri::AppHandle,
    /// 全局复用的 HTTP 客户端（连接池 / keep-alive），避免每个文件重建连接。
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct ResourceEntry {
    dest: String,
    md5: String,
    size: i64,
}

#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub url: String,
    pub path: PathBuf,
    pub size: u64,
}

/// 单个 CDN 节点信息（来自 launcher info 的 `default.cdnList`）
#[derive(Clone, Serialize)]
pub struct CdnNode {
    /// 节点基址 URL
    pub url: String,
    /// 优先级（P 字段），数值越大越优先
    pub priority: i64,
    /// K1 标志
    pub k1: i64,
    /// K2 标志
    pub k2: i64,
    /// 是否为自动选择的推荐节点（K1==1 && K2==1 中优先级最高者）
    pub recommended: bool,
}

pub struct GameManager {
    ctx: SharedCtx,
    launcher_info: Option<serde_json::Value>,
    cdn_node: Option<String>,
    preferred_cdn: Option<String>,
    game_index: Option<serde_json::Value>,
    predownload_index_cache: Option<serde_json::Value>,
}

impl GameManager {
    pub fn new(
        game_folder: PathBuf,
        server_type: &str,
        app_handle: tauri::AppHandle,
        cdn_url: Option<String>,
    ) -> Result<Self, WwError> {
        let server_config = config::server_configs()
            .get(server_type)
            .cloned()
            .ok_or_else(|| WwError::Config(format!("无效的服务器类型: {}", server_type)))?;

        let md5_cache =
            Md5Cache::new(game_folder.join("wwm_md5_cache.json"), game_folder.clone());

        // 构建一次全局复用的下载客户端：开启连接池、TCP keep-alive、nodelay，
        // 让大量并发请求复用已有连接，省去重复的 TCP/TLS 握手开销。
        let client = Self::build_download_client()?;

        Ok(Self {
            ctx: SharedCtx {
                game_folder: game_folder.canonicalize().unwrap_or(game_folder),
                server_type: server_type.to_string(),
                server_config,
                md5_cache,
                app_handle,
                client,
            },
            launcher_info: None,
            cdn_node: None,
            preferred_cdn: cdn_url,
            game_index: None,
            predownload_index_cache: None,
        })
    }

    fn emit_progress(&self, event: ProgressEvent) {
        let _ = self.ctx.app_handle.emit("ww:progress", event);
    }

    fn emit_log(&self, message: &str, level: &str) {
        self.emit_progress(ProgressEvent {
            event_type: level.to_string(),
            message: message.to_string(),
            current: 0,
            total: 0,
            file_name: None,
        });
    }

    // ── HTTP helpers (no &self borrow held) ──

    async fn http_client_with_timeout(timeout_secs: u64) -> Result<reqwest::Client, WwError> {
        reqwest::Client::builder()
            .user_agent("WW-Manager/2.0")
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| WwError::Network(e.to_string()))
    }

    /// 构建全局复用的下载客户端。仅在 GameManager::new 中调用一次。
    fn build_download_client() -> Result<reqwest::Client, WwError> {
        reqwest::Client::builder()
            .user_agent("WW-Manager/2.0")
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(DOWNLOAD_READ_TIMEOUT_SECS))
            .pool_max_idle_per_host(32)
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .tcp_nodelay(true)
            .build()
            .map_err(|e| WwError::Network(e.to_string()))
    }

    async fn fetch_json(url: &str, timeout_secs: u64) -> Result<serde_json::Value, WwError> {
        let client = Self::http_client_with_timeout(timeout_secs).await?;
        let resp = client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(WwError::Network(format!("HTTP {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    // ── Lazy CDN properties (return owned values to avoid borrow issues) ──

    async fn ensure_launcher_info(&mut self) -> Result<serde_json::Value, WwError> {
        if self.launcher_info.is_none() {
            let msg = format!("正在获取 {} 服配置...", self.ctx.server_type);
            self.emit_log(&msg, "log");
            let info = Self::fetch_json(&self.ctx.server_config.api_url, JSON_TIMEOUT_SECS).await?;
            self.launcher_info = Some(info);
        }
        Ok(self.launcher_info.clone().unwrap())
    }

    /// 拉取指定服务器的 CDN 节点列表，并标注推荐节点。
    pub async fn fetch_cdn_list(server_type: &str) -> Result<Vec<CdnNode>, WwError> {
        let server_config = config::server_configs()
            .get(server_type)
            .cloned()
            .ok_or_else(|| WwError::Config(format!("无效的服务器类型: {}", server_type)))?;

        let info = Self::fetch_json(&server_config.api_url, JSON_TIMEOUT_SECS).await?;
        let default_info = info
            .get("default")
            .ok_or_else(|| WwError::Network("launcher info 缺少 'default' 字段".into()))?;

        let nodes = default_info
            .get("cdnList")
            .and_then(|v| v.as_array())
            .ok_or_else(|| WwError::Network("CDN 列表为空".into()))?;

        let mut list: Vec<CdnNode> = Vec::new();
        for n in nodes {
            let url = n
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| WwError::Network("CDN 节点缺少 url".into()))?
                .to_string();
            let priority = n.get("P").and_then(|v| v.as_i64()).unwrap_or(0);
            let k1 = n.get("K1").and_then(|v| v.as_i64()).unwrap_or(0);
            let k2 = n.get("K2").and_then(|v| v.as_i64()).unwrap_or(0);
            list.push(CdnNode {
                url,
                priority,
                k1,
                k2,
                recommended: false,
            });
        }

        // 推荐节点：K1==1 且 K2==1 中优先级最高者
        let best_index = list
            .iter()
            .enumerate()
            .filter(|(_, n)| n.k1 == 1 && n.k2 == 1)
            .max_by(|a, b| a.1.priority.cmp(&b.1.priority))
            .map(|(i, _)| i);

        if let Some(i) = best_index {
            list[i].recommended = true;
        }

        Ok(list)
    }

    async fn ensure_cdn_node(&mut self) -> Result<String, WwError> {
        if self.cdn_node.is_none() {
            // 若用户已指定 CDN，则直接使用，否则自动选择推荐节点
            let url = if let Some(ref preferred) = self.preferred_cdn {
                preferred.clone()
            } else {
                let list = Self::fetch_cdn_list(&self.ctx.server_type).await?;
                let best = list
                    .iter()
                    .filter(|n| n.k1 == 1 && n.k2 == 1)
                    .max_by(|a, b| a.priority.cmp(&b.priority))
                    .ok_or_else(|| WwError::Network("没有可用的 CDN 节点".into()))?;
                best.url.clone()
            };

            self.cdn_node = Some(url.to_string());
            log::info!("使用 CDN: {}", url);
        }
        Ok(self.cdn_node.clone().unwrap())
    }

    async fn ensure_game_index(&mut self) -> Result<serde_json::Value, WwError> {
        if self.game_index.is_none() {
            let info = self.ensure_launcher_info().await?;
            let default_info = info.get("default").ok_or_else(|| {
                WwError::Network("launcher info 缺少 'default' 字段".into())
            })?;
            let index_file = default_info
                .get("config")
                .and_then(|c| c.get("indexFile"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| WwError::Network("缺少 indexFile 配置".into()))?;

            let cdn = self.ensure_cdn_node().await?;
            let url = Url::parse(&cdn)?.join(index_file)?;

            self.emit_log("下载文件清单 (Index)...", "log");
            log::info!("下载文件清单 (Index)...");

            let index = Self::fetch_json(url.as_str(), JSON_TIMEOUT_SECS).await?;
            self.game_index = Some(index);
        }
        Ok(self.game_index.clone().unwrap())
    }

    async fn ensure_predownload_index(
        &mut self,
    ) -> Result<Option<serde_json::Value>, WwError> {
        if self.predownload_index_cache.is_none() {
            let info = self.ensure_launcher_info().await?;
            match info.get("predownload") {
                Some(pre_info) => {
                    if let Some(file) = pre_info
                        .get("config")
                        .and_then(|c| c.get("indexFile"))
                        .and_then(|v| v.as_str())
                    {
                        let cdn = self.ensure_cdn_node().await?;
                        let url = Url::parse(&cdn)?.join(file)?;
                        self.emit_log("下载预下载文件清单...", "log");
                        log::info!("下载预下载文件清单...");
                        let index = Self::fetch_json(url.as_str(), JSON_TIMEOUT_SECS).await?;
                        self.predownload_index_cache = Some(index);
                    } else {
                        self.predownload_index_cache = Some(serde_json::Value::Null);
                    }
                }
                None => {
                    self.predownload_index_cache = Some(serde_json::Value::Null);
                }
            }
        }

        match self.predownload_index_cache.as_ref().unwrap() {
            serde_json::Value::Null => Ok(None),
            val => Ok(Some(val.clone())),
        }
    }

    fn get_res_base(info: &serde_json::Value) -> Option<&str> {
        info.get("resourcesBasePath")
            .or_else(|| info.get("baseUrl"))
            .and_then(|v| v.as_str())
    }

    // ── single-file download (retry + resume) ──

    async fn download_file(
        &self,
        url: &str,
        dest: &Path,
        expected_size: u64,
    ) -> Result<bool, WwError> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let ext = dest
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "tmp".to_string());
        let temp_file = dest.with_extension(format!("{}.temp", ext));

        if temp_file.exists() && std::fs::metadata(&temp_file)?.len() == expected_size {
            std::fs::rename(&temp_file, dest)?;
            self.ctx.md5_cache.clear(dest).await;
            return Ok(true);
        }

        if expected_size > MIN_CHUNKED_SIZE {
            return self.download_file_chunked(url, dest, &temp_file, expected_size).await;
        }

        self.download_file_single(url, dest, &temp_file, expected_size).await
    }

    async fn download_file_single(
        &self,
        url: &str,
        dest: &Path,
        temp_file: &Path,
        expected_size: u64,
    ) -> Result<bool, WwError> {
        for attempt in 0..RETRY_COUNT {
            match self.attempt_download(url, temp_file, expected_size).await {
                // Some(md5): 从头下载完成，携带流式计算的 MD5，直接写入缓存
                // None: 断点续传或文件已完整，无法得到完整 MD5，清除缓存待校验阶段重算
                Ok(md5_opt) => {
                    std::fs::rename(temp_file, dest)?;
                    match md5_opt {
                        Some(md5) => self.ctx.md5_cache.set(dest, md5).await,
                        None => self.ctx.md5_cache.clear(dest).await,
                    }
                    return Ok(true);
                }
                Err(e) => {
                    if attempt == RETRY_COUNT - 1 {
                        log::error!("下载失败: {:?}: {}", dest, e);
                        return Err(e);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1 + attempt as u64)).await;
                }
            }
        }
        Ok(false)
    }

    async fn download_file_chunked(
        &self,
        url: &str,
        dest: &Path,
        temp_file: &Path,
        expected_size: u64,
    ) -> Result<bool, WwError> {
        let chunk_count = chunk_count_for(expected_size);
        let chunk_size = (expected_size + chunk_count as u64 - 1) / chunk_count as u64;
        let mut chunk_paths = Vec::new();
        for i in 0..chunk_count {
            let start = i as u64 * chunk_size;
            let end = std::cmp::min(start + chunk_size - 1, expected_size - 1);
            if start >= expected_size {
                break;
            }
            let chunk_path = temp_file.with_extension(format!("chunk_{}", i));
            chunk_paths.push((chunk_path, start, end));
        }

        let concat_paths: Vec<PathBuf> = chunk_paths.iter().map(|(p, _, _)| p.clone()).collect();
        let url = url.to_string();
        let mut handles = Vec::new();

        for (chunk_path, start, end) in chunk_paths {
            let url = url.clone();
            let ctx = self.ctx.clone();
            handles.push(tokio::spawn(async move {
                let mgr = GameManager {
                    ctx,
                    launcher_info: None,
                    cdn_node: None,
                    preferred_cdn: None,
                    game_index: None,
                    predownload_index_cache: None,
                };
                mgr.download_chunk(&url, &chunk_path, start, end).await
            }));
        }

        let mut any_failed = false;
        for h in handles {
            match h.await {
                Ok(Ok(true)) => {}
                Ok(Ok(false)) => {
                    any_failed = true;
                }
                Ok(Err(e)) => {
                    log::error!("分块下载失败: {:?}", e);
                    any_failed = true;
                }
                Err(e) => {
                    log::error!("分块下载 panic: {}", e);
                    any_failed = true;
                }
            }
        }

        if any_failed {
            return Err(WwError::Network("分块下载失败，请重试。".into()));
        }

        // 合并分块 + 流式计算 MD5（复用同一次读取，零额外 I/O）。
        // 放到 spawn_blocking，避免大文件拼接长时间阻塞 async 工作线程。
        let temp_path = temp_file.to_path_buf();
        let md5_hex = tokio::task::spawn_blocking(move || -> Result<String, WwError> {
            let final_file = std::fs::File::create(&temp_path)?;
            let mut final_writer = std::io::BufWriter::with_capacity(1 << 20, final_file);
            let mut md5_ctx = md5::Context::new();
            let mut buf = vec![0u8; 1 << 20];
            for chunk_path in &concat_paths {
                if !chunk_path.exists() {
                    return Err(WwError::Network("分块文件缺失。".into()));
                }
                let mut chunk_file = std::fs::File::open(chunk_path)?;
                loop {
                    let n = chunk_file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    final_writer.write_all(&buf[..n])?;
                    md5_ctx.consume(&buf[..n]);
                }
                drop(chunk_file);
                let _ = std::fs::remove_file(chunk_path);
            }
            final_writer.flush()?;
            Ok(format!("{:x}", md5_ctx.compute()))
        })
        .await
        .map_err(|e| WwError::Generic(format!("拼接任务异常: {}", e)))??;

        // Verify total size
        let final_size = std::fs::metadata(temp_file)?.len();
        if final_size != expected_size {
            let _ = std::fs::remove_file(temp_file);
            return Err(WwError::Network(format!(
                "下载不完整: 期望 {} 字节，实际 {} 字节",
                expected_size, final_size
            )));
        }

        std::fs::rename(temp_file, dest)?;
        self.ctx.md5_cache.set(dest, md5_hex).await;
        Ok(true)
    }

    async fn download_chunk(
        &self,
        url: &str,
        chunk_path: &Path,
        start: u64,
        end: u64,
    ) -> Result<bool, WwError> {
        let expected = end - start + 1;

        // Check if chunk already complete
        if chunk_path.exists() && std::fs::metadata(chunk_path)?.len() == expected {
            return Ok(true);
        }

        let client = self.ctx.client.clone();
        for attempt in 0..RETRY_COUNT {
            let range_header = format!("bytes={}-{}", start, end);

            let resp = match client.get(url).header("Range", &range_header).send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt == RETRY_COUNT - 1 {
                        return Err(WwError::Network(format!("HTTP 请求失败: {}", e)));
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1 + attempt as u64)).await;
                    continue;
                }
            };

            let status = resp.status();
            if status != reqwest::StatusCode::OK
                && status != reqwest::StatusCode::PARTIAL_CONTENT
            {
                if attempt == RETRY_COUNT - 1 {
                    return Err(WwError::Network(format!("HTTP {}", status.as_u16())));
                }
                tokio::time::sleep(std::time::Duration::from_secs(1 + attempt as u64)).await;
                continue;
            }

            let file = std::fs::File::create(chunk_path)?;
            let mut writer = std::io::BufWriter::with_capacity(1 << 20, file);
            use futures_util::StreamExt;
            let mut stream = resp.bytes_stream();
            let mut downloaded: u64 = 0;

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result?;
                writer.write_all(&chunk)?;
                downloaded += chunk.len() as u64;
            }
            writer.flush()?;

            if downloaded == expected {
                return Ok(true);
            }

            if attempt == RETRY_COUNT - 1 {
                let _ = std::fs::remove_file(chunk_path);
                return Err(WwError::Network(format!(
                    "分块 {} 下载不完整: 期望 {} 字节，实际 {} 字节",
                    chunk_path.display(),
                    expected,
                    downloaded
                )));
            }

            let _ = std::fs::remove_file(chunk_path);
            tokio::time::sleep(std::time::Duration::from_secs(1 + attempt as u64)).await;
        }

        Ok(false)
    }

    async fn attempt_download(
        &self,
        url: &str,
        temp_file: &Path,
        expected_size: u64,
    ) -> Result<Option<String>, WwError> {
        let resume_byte = if temp_file.exists() {
            let size = std::fs::metadata(temp_file)?.len();
            if size == expected_size {
                // 文件已完整，无本次流式数据可得，交由校验阶段处理
                return Ok(None);
            }
            size
        } else {
            0
        };

        let client = self.ctx.client.clone();

        let mut req = client.get(url);
        if resume_byte > 0 {
            req = req.header("Range", format!("bytes={}-", resume_byte));
        }

        let resp = req.send().await?;
        let status = resp.status();
        if status != reqwest::StatusCode::OK
            && status != reqwest::StatusCode::PARTIAL_CONTENT
        {
            return Err(WwError::Network(format!("HTTP {}", status.as_u16())));
        }

        let append = resume_byte > 0;
        let file = if append {
            std::fs::OpenOptions::new().append(true).open(temp_file)?
        } else {
            std::fs::File::create(temp_file)?
        };
        let mut writer = std::io::BufWriter::with_capacity(1 << 20, file);

        // 仅在从头下载时才能流式得到完整文件的 MD5
        let mut md5_ctx = if append { None } else { Some(md5::Context::new()) };

        use futures_util::StreamExt;
        let mut downloaded = resume_byte;
        let mut stream = resp.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            writer.write_all(&chunk)?;
            if let Some(ctx) = md5_ctx.as_mut() {
                ctx.consume(&chunk);
            }
            downloaded += chunk.len() as u64;
        }
        writer.flush()?;

        if downloaded < expected_size {
            return Err(WwError::Network("下载不完整".into()));
        }

        // 只有字节数精确匹配时才缓存 MD5，避免异常情况下写入错误指纹
        Ok(if downloaded == expected_size {
            md5_ctx.map(|ctx| format!("{:x}", ctx.compute()))
        } else {
            None
        })
    }

    // ── batch download ──

    async fn batch_download(&self, tasks: &[DownloadTask]) -> Result<(), WwError> {
        if tasks.is_empty() {
            self.emit_log("没有文件需要下载", "log");
            return Ok(());
        }

        let total_size: u64 = tasks.iter().map(|t| t.size).sum();
        let total_count = tasks.len() as u64;
        self.emit_log(
            &format!(
                "准备下载 {} 个文件，总大小: {:.2} MB",
                total_count,
                total_size as f64 / 1024.0 / 1024.0
            ),
            "log",
        );

        let sem = Arc::new(Semaphore::new(DOWNLOAD_CONCURRENT));
        let completed = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut handles = Vec::new();

        // 文件很多时没必要每个文件都发一次 IPC 事件（会拖慢 UI 与主进程），
        // 这里按比例节流：少量文件逐个上报，海量文件每 20 个上报一次。
        let emit_every = if total_count <= 200 { 1 } else { 20 };

        for task in tasks {
            let sem = sem.clone();
            let url = task.url.clone();
            let path = task.path.clone();
            let size = task.size;
            let ctx = self.ctx.clone();
            let completed = completed.clone();
            let failed = failed.clone();
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                let mgr = GameManager {
                    ctx: ctx.clone(),
                    launcher_info: None,
                    cdn_node: None,
                    preferred_cdn: None,
                    game_index: None,
                    predownload_index_cache: None,
                };
                match mgr.download_file(&url, &path, size).await {
                    Ok(true) => {}
                    _ => {
                        failed.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                let n = completed
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                if n % emit_every == 0 || n == total_count {
                    let _ = ctx.app_handle.emit(
                        "ww:progress",
                        ProgressEvent {
                            event_type: "download_file_done".into(),
                            message: format!("[{}] {}", n, file_name),
                            current: n,
                            total: total_count,
                            file_name: Some(file_name),
                        },
                    );
                }
            }));
        }

        for h in handles {
            if let Err(e) = h.await {
                log::error!("下载任务 panic: {}", e);
                return Err(WwError::Generic(format!("下载任务异常: {}", e)));
            }
        }

        if failed.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(WwError::Network(
                "部分文件下载失败，请重试。".into(),
            ));
        }

        Ok(())
    }

    // ── Sync / Verify ──

    pub async fn sync_files(&mut self, force_check_md5: bool) -> Result<(), WwError> {
        // Extract all needed data as owned values FIRST to avoid borrow conflicts
        let launcher_info = self.ensure_launcher_info().await?;
        let default_info = launcher_info
            .get("default")
            .ok_or_else(|| {
                WwError::Network("launcher info 缺少 'default' 字段".into())
            })?;
        let res_base = Self::get_res_base(default_info)
            .ok_or_else(|| {
                WwError::Generic(
                    "无法获取资源路径配置 (resourcesBasePath 和 baseUrl 均为空)".into(),
                )
            })?
            .to_string();

        let cdn = self.ensure_cdn_node().await?;
        let cdn_url = Url::parse(&cdn)?;

        let game_index = self.ensure_game_index().await?;
        let res_list = game_index
            .get("resource")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                WwError::Network("游戏索引中缺少 'resource' 列表".into())
            })?;

        let total = res_list.len() as u64;
        if force_check_md5 {
            self.emit_log("正在校验文件 (可能需要几分钟)...", "log");
        }

        // Build owned entry list
        let entries: Vec<ResourceEntry> = res_list
            .iter()
            .map(|item| ResourceEntry {
                dest: item["dest"].as_str().unwrap_or("").to_string(),
                md5: item["md5"].as_str().unwrap_or("").to_string(),
                size: item["size"].as_i64().unwrap_or(0),
            })
            .collect();

        // All data for spawned tasks — clone before spawning
        let game_folder = self.ctx.game_folder.clone();
        let md5_cache = self.ctx.md5_cache.clone();
        let app_handle = self.ctx.app_handle.clone();

        // Phase 1: parallel verification
        let sem = Arc::new(Semaphore::new(VERIFY_CONCURRENT));
        let verified = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let results: Arc<tokio::sync::Mutex<Vec<Option<DownloadTask>>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::with_capacity(entries.len())));
        let mut handles = Vec::new();

        for entry in &entries {
            let sem = sem.clone();
            let game_folder = game_folder.clone();
            let cdn_url = cdn_url.clone();
            let res_base = res_base.clone();
            let md5_cache = md5_cache.clone();
            let app_handle = app_handle.clone();
            let verified = verified.clone();
            let results = results.clone();
            let entry = entry.clone();
            let total = total;

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                let dest_path = game_folder.join(&entry.dest);

                let need_download = if !dest_path.exists() {
                    true
                } else if force_check_md5 {
                    md5_cache.get(&dest_path).await != Some(entry.md5.clone())
                } else {
                    dest_path
                        .metadata()
                        .map(|m| m.len() as i64)
                        .unwrap_or(-1)
                        != entry.size
                };

                let file_name = dest_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let task = if need_download {
                    let relative = format!("{}/{}", res_base, entry.dest);
                    if let Ok(url) = cdn_url.join(&relative) {
                        Some(DownloadTask {
                            url: url.to_string(),
                            path: dest_path,
                            size: entry.size as u64,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                let n = verified
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                if force_check_md5 && (n % 50 == 0 || n == total) {
                    let _ = app_handle.emit(
                        "ww:progress",
                        ProgressEvent {
                            event_type: "verify_progress".into(),
                            message: format!("校验中: {}/{}", n, total),
                            current: n,
                            total,
                            file_name: Some(file_name),
                        },
                    );
                }

                results.lock().await.push(task);
            }));
        }

        for h in handles {
            if let Err(e) = h.await {
                log::error!("校验任务 panic: {}", e);
                return Err(WwError::Generic(format!("校验任务异常: {}", e)));
            }
        }

        // Phase 2: collect download tasks
        let download_tasks: Vec<DownloadTask> = {
            let lock = results.lock().await;
            lock.iter().filter_map(|t| t.clone()).collect()
        };

        let needed = download_tasks.len();
        if needed > 0 {
            self.batch_download(&download_tasks).await?;
            self.ctx.md5_cache.save().await;
            let _ = self.update_local_config().await;
        } else {
            self.emit_log("所有文件校验通过，无需下载。", "log");
        }

        Ok(())
    }

    // ── Full download ──

    pub async fn download_full(&mut self) -> Result<(), WwError> {
        let msg = format!(
            "准备下载 {} 服完整客户端到: {:?}",
            self.ctx.server_type, self.ctx.game_folder
        );
        self.emit_log(&msg, "log");

        std::fs::create_dir_all(&self.ctx.game_folder)?;
        self.sync_files(false).await?;
        self.emit_log("下载完成，正在检验游戏完整性...", "log");
        self.sync_files(true).await?;

        let msg = format!("{} 服完整客户端下载完毕！", self.ctx.server_type);
        self.emit_log(&msg, "log");
        Ok(())
    }

    // ── Update game ──

    pub async fn update_game(&mut self) -> Result<(), WwError> {
        let info = self.ensure_launcher_info().await?;
        let server_version = info["default"]["version"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        let cfg_path = self.ctx.game_folder.join("launcherDownloadConfig.json");
        let local_version = if cfg_path.exists() {
            let content = std::fs::read_to_string(&cfg_path)?;
            let cfg: serde_json::Value = serde_json::from_str(&content)?;
            cfg.get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string()
        } else {
            "unknown".to_string()
        };

        if local_version == server_version {
            self.emit_log(
                &format!("当前已是最新版本 ({}), 无需更新。", server_version),
                "log",
            );
            return Ok(());
        }

        // Check pre-download
        let predownload_root = self.ctx.game_folder.join(".predownload");
        let version_file = predownload_root.join("predownload_version.json");
        let has_predownload = predownload_root.exists() && version_file.exists();

        if has_predownload {
            let pd_info: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&version_file)?)?;
            let pd_version = pd_info["version"].as_str().unwrap_or("unknown").to_string();
            let pd_server = pd_info["server"].as_str().unwrap_or("");

            if pd_server == self.ctx.server_type && pd_version == server_version {
                // Pre-download matches — apply it directly
                self.emit_log(
                    &format!("检测到预下载内容 (版本: {}), 直接应用...", pd_version),
                    "log",
                );
                self.clear_game_dir_except_predownload().await?;
                self.move_predownload_to_game().await?;
                self.emit_log(
                    &format!("预下载已应用，正在校验完整性..."),
                    "log",
                );
                self.launcher_info = None;
                self.game_index = None;
                self.cdn_node = None;
                self.sync_files(true).await?;
                self.emit_log(
                    &format!("更新完成！当前版本: {}", server_version),
                    "log",
                );
                return Ok(());
            }

            if pd_version < server_version {
                self.emit_log(
                    &format!(
                        "预下载版本 ({}) 已过期，远程版本: {}，将清除所有文件并重新下载。",
                        pd_version, server_version
                    ),
                    "log",
                );
                self.clear_game_dir_all().await?;
                self.emit_log("旧文件已清理，开始下载新版本...", "log");
                self.download_full().await?;
                self.emit_log(
                    &format!("更新完成！当前版本: {}", server_version),
                    "log",
                );
                return Ok(());
            }

            if pd_server != self.ctx.server_type {
                self.emit_log(
                    &format!(
                        "预下载服务器 ({}) 与当前服务器 ({}) 不匹配，将重新下载。",
                        pd_server, self.ctx.server_type
                    ),
                    "log",
                );
                self.clear_game_dir_all().await?;
                self.emit_log("旧文件已清理，开始下载新版本...", "log");
                self.download_full().await?;
                self.emit_log(
                    &format!("更新完成！当前版本: {}", server_version),
                    "log",
                );
                return Ok(());
            }
        }

        // No usable pre-download — fresh download
        self.emit_log(
            &format!("发现新版本: {} -> {}，开始更新...", local_version, server_version),
            "log",
        );
        self.clear_game_dir_except_predownload().await?;
        self.emit_log("旧版本文件已清理，开始下载新版本...", "log");
        self.download_full().await?;
        self.emit_log(
            &format!("更新完成！当前版本: {}", server_version),
            "log",
        );
        Ok(())
    }

    async fn clear_game_dir_except_predownload(&self) -> Result<(), WwError> {
        if !self.ctx.game_folder.exists() {
            return Ok(());
        }
        let entries: Vec<_> = std::fs::read_dir(&self.ctx.game_folder)?
            .filter_map(|e| match e {
                Ok(entry) => Some(entry),
                Err(err) => {
                    log::warn!("无法读取目录项: {}", err);
                    None
                }
            })
            .filter(|e| e.file_name() != ".predownload")
            .collect();
        let total = entries.len() as u64;
        for (i, entry) in entries.iter().enumerate() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
            self.emit_progress(ProgressEvent {
                event_type: "delete_progress".into(),
                message: format!("清理旧版本: {}", name),
                current: (i + 1) as u64,
                total,
                file_name: Some(name),
            });
        }
        Ok(())
    }

    async fn clear_game_dir_all(&self) -> Result<(), WwError> {
        if !self.ctx.game_folder.exists() {
            return Ok(());
        }
        let entries: Vec<_> = std::fs::read_dir(&self.ctx.game_folder)?
            .filter_map(|e| match e {
                Ok(entry) => Some(entry),
                Err(err) => {
                    log::warn!("无法读取目录项: {}", err);
                    None
                }
            })
            .collect();
        let total = entries.len() as u64;
        for (i, entry) in entries.iter().enumerate() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
            self.emit_progress(ProgressEvent {
                event_type: "delete_progress".into(),
                message: format!("清理旧版本: {}", name),
                current: (i + 1) as u64,
                total,
                file_name: Some(name),
            });
        }
        Ok(())
    }

    async fn move_predownload_to_game(&self) -> Result<(), WwError> {
        let predownload_root = self.ctx.game_folder.join(".predownload");
        if !predownload_root.exists() {
            return Err(WwError::Config("预下载目录不存在".into()));
        }

        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(&predownload_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file()
                && entry.file_name() != "predownload_version.json"
            {
                let rel = entry.path().strip_prefix(&predownload_root).unwrap();
                let dest = self.ctx.game_folder.join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                entries.push((entry.path().to_path_buf(), dest));
            }
        }

        let count = entries.len() as u64;
        for (i, (src, dst)) in entries.iter().enumerate() {
            std::fs::rename(src, dst)?;
            self.ctx.md5_cache.clear(dst).await;
            if (i + 1) % 10 == 0 || i + 1 == count as usize {
                self.emit_progress(ProgressEvent {
                    event_type: "delete_progress".into(),
                    message: format!("合并预下载: {}/{}", i + 1, count),
                    current: (i + 1) as u64,
                    total: count,
                    file_name: None,
                });
            }
        }

        self.emit_log(&format!("已合并 {} 个预下载文件。", count), "log");

        // Remove version file and empty .predownload directory
        let version_file = predownload_root.join("predownload_version.json");
        if version_file.exists() {
            let _ = std::fs::remove_file(&version_file);
        }
        if predownload_root.exists() {
            let is_empty = predownload_root
                .read_dir()
                .map(|mut d| d.next().is_none())
                .unwrap_or(true);
            if is_empty {
                let _ = std::fs::remove_dir(&predownload_root);
            }
        }

        Ok(())
    }

    pub async fn delete_all_files(&mut self) -> Result<(), WwError> {
        self.emit_log("正在删除所有游戏文件和预下载内容...", "log");

        if self.ctx.game_folder.exists() {
            let entries: Vec<_> = std::fs::read_dir(&self.ctx.game_folder)?
                .filter_map(|e| match e {
                    Ok(entry) => Some(entry),
                    Err(err) => {
                        log::warn!("无法读取目录项: {}", err);
                        None
                    }
                })
                .collect();
            let total = entries.len() as u64;
            for (i, entry) in entries.iter().enumerate() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    std::fs::remove_dir_all(&path)?;
                } else {
                    std::fs::remove_file(&path)?;
                }
                self.emit_progress(ProgressEvent {
                    event_type: "delete_progress".into(),
                    message: format!("删除: {}", name),
                    current: (i + 1) as u64,
                    total,
                    file_name: Some(name),
                });
            }
        }

        // Remove the game folder itself if empty
        if self.ctx.game_folder.exists() {
            if self.ctx.game_folder.read_dir().map(|mut d| d.next().is_none()).unwrap_or(false) {
                let _ = std::fs::remove_dir(&self.ctx.game_folder);
            }
        }

        self.emit_log("游戏文件已全部删除。", "log");
        Ok(())
    }

    // ── Predownload ──

    pub async fn download_predownload(&mut self) -> Result<(), WwError> {
        let index = match self.ensure_predownload_index().await? {
            Some(idx) => idx,
            None => {
                return Err(WwError::Config(
                    "当前服务器未开放预下载，或未能获取到预下载配置。".into(),
                ));
            }
        };

        let launcher_info = self.ensure_launcher_info().await?;
        let pre_info = launcher_info
            .get("predownload")
            .ok_or_else(|| WwError::Config("无法获取预下载配置".into()))?;

        let pre_config = pre_info
            .get("config")
            .ok_or_else(|| WwError::Config("预下载配置中缺少 'config'".into()))?;

        let res_base = Self::get_res_base(pre_config)
            .ok_or_else(|| {
                WwError::Generic(
                    "无法获取预下载资源路径配置 (resourcesBasePath 和 baseUrl 均为空)".into(),
                )
            })?
            .to_string();

        let res_list = match index.get("resource").and_then(|v| v.as_array()) {
            Some(list) => list,
            None => {
                self.emit_log("预下载资源列表为空。", "log");
                return Ok(());
            }
        };

        let predownload_root = self.ctx.game_folder.join(".predownload");
        std::fs::create_dir_all(&predownload_root)?;

        let target_version = pre_info
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let server = self.ctx.server_type.clone();

        let version_info = serde_json::json!({
            "version": target_version,
            "server": &server,
        });
        std::fs::write(
            predownload_root.join("predownload_version.json"),
            serde_json::to_string_pretty(&version_info).unwrap(),
        )?;

        let msg = format!("开始准备预下载资源 (目标版本: {})...", target_version);
        self.emit_log(&msg, "log");

        let cdn = self.ensure_cdn_node().await?;
        let base_url = Url::parse(&cdn)?;

        let mut tasks = Vec::new();
        for item in res_list {
            let dest_str = item["dest"].as_str().unwrap_or("");
            let dest_path = predownload_root.join(dest_str);
            let expected_size = item["size"].as_i64().unwrap_or(0) as u64;

            let need_download = if !dest_path.exists() {
                true
            } else {
                dest_path
                    .metadata()
                    .map(|m| m.len())
                    .unwrap_or(0)
                    != expected_size
            };

            if need_download {
                let relative = format!("{}/{}", res_base, dest_str);
                if let Ok(url) = base_url.join(&relative) {
                    tasks.push(DownloadTask {
                        url: url.to_string(),
                        path: dest_path,
                        size: expected_size,
                    });
                }
            }
        }

        if !tasks.is_empty() {
            self.batch_download(&tasks).await?;
            self.emit_log("预下载资源下载完成！", "log");
        } else {
            self.emit_log(
                "所有预下载资源均已存在且校验通过，无需重复下载。",
                "log",
            );
        }

        Ok(())
    }

    // ── Checkout (server switching) ──

    pub async fn checkout(
        &mut self,
        target_server: &str,
        force_sync: bool,
    ) -> Result<(), WwError> {
        let diff_files = config::server_diff_files();

        // 1. Disable all server-specific files
        for (_, files) in diff_files.iter() {
            for f_rel in files {
                let f = self.ctx.game_folder.join(f_rel);
                let bak = with_bak_extension(&f);
                if f.exists() {
                    std::fs::rename(&f, &bak)?;
                    self.ctx.md5_cache.clear(&f).await;
                }
            }
        }

        // 2. Enable target server's files
        let mut missing = false;
        if let Some(target_files) = diff_files.get(target_server) {
            for f_rel in target_files {
                let f = self.ctx.game_folder.join(f_rel);
                let bak = with_bak_extension(&f);
                if bak.exists() {
                    std::fs::rename(&bak, &f)?;
                    self.ctx.md5_cache.clear(&f).await;
                } else if !f.exists() {
                    missing = true;
                }
            }
        }

        // 3. Update config
        self.ctx.server_type = target_server.to_string();
        self.ctx.server_config = config::server_configs()
            .get(target_server)
            .cloned()
            .ok_or_else(|| {
                WwError::Config(format!("无效的服务器类型: {}", target_server))
            })?;

        self.launcher_info = None;
        self.cdn_node = None;
        self.game_index = None;

        if let Err(e) = self.update_local_config().await {
            log::warn!("更新本地配置失败: {}", e);
        }

        if missing || force_sync {
            self.emit_log("检测到缺失文件或强制同步，开始同步...", "log");
            self.sync_files(true).await?;
        }

        Ok(())
    }

    // ── Update local config ──

    pub async fn update_local_config(&mut self) -> Result<(), WwError> {
        let info = self.ensure_launcher_info().await?;
        let version = info["default"]["version"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let app_id = self.ctx.server_config.app_id.clone();
        let server_type = self.ctx.server_type.clone();

        let cfg = serde_json::json!({
            "version": version,
            "appId": app_id,
            "group": "default",
        });

        std::fs::create_dir_all(&self.ctx.game_folder)?;
        let cfg_path = self.ctx.game_folder.join("launcherDownloadConfig.json");
        std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg)?)?;

        log::info!("本地配置已更新: {} ({})", server_type, version);
        Ok(())
    }
}

fn with_bak_extension(path: &Path) -> PathBuf {
    match path.extension() {
        Some(ext) => path.with_extension(format!("{}.bak", ext.to_string_lossy())),
        None => path.with_extension("bak"),
    }
}
