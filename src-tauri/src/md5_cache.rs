use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    mtime: i64,
    md5: String,
}

#[derive(Clone)]
pub struct Md5Cache {
    cache_path: PathBuf,
    game_root: PathBuf,
    inner: Arc<RwLock<CacheInner>>,
}

struct CacheInner {
    entries: HashMap<String, CacheEntry>,
    updated: bool,
}

impl Md5Cache {
    pub fn new(cache_path: PathBuf, game_root: PathBuf) -> Self {
        let entries = Self::load_entries(&cache_path);
        Self {
            cache_path,
            game_root,
            inner: Arc::new(RwLock::new(CacheInner {
                entries,
                updated: false,
            })),
        }
    }

    fn load_entries(path: &Path) -> HashMap<String, CacheEntry> {
        if !path.exists() {
            return HashMap::new();
        }
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    pub async fn save(&self) {
        let mut inner = self.inner.write().await;
        if !inner.updated {
            return;
        }
        match serde_json::to_string_pretty(&inner.entries) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&self.cache_path, content) {
                    log::error!("保存 MD5 缓存失败: {}", e);
                } else {
                    log::debug!("MD5 缓存已保存");
                    inner.updated = false;
                }
            }
            Err(e) => {
                log::error!("序列化 MD5 缓存失败: {}", e);
            }
        }
    }

    pub async fn get(&self, file_path: &Path) -> Option<String> {
        if !file_path.exists() {
            return None;
        }

        let rel_path = match file_path.strip_prefix(&self.game_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => file_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
        };

        let mtime = Self::get_mtime(file_path)?;

        {
            let inner = self.inner.read().await;
            if let Some(entry) = inner.entries.get(&rel_path) {
                if entry.mtime == mtime {
                    return Some(entry.md5.clone());
                }
            }
        }

        let path_owned = file_path.to_path_buf();
        let new_md5 = tokio::task::spawn_blocking(move || Self::calculate_md5(&path_owned))
            .await
            .ok()??;

        {
            let mut inner = self.inner.write().await;
            inner.entries.insert(
                rel_path,
                CacheEntry {
                    mtime,
                    md5: new_md5.clone(),
                },
            );
            inner.updated = true;
        }

        Some(new_md5)
    }

    fn get_mtime(file_path: &Path) -> Option<i64> {
        let mtime = std::fs::metadata(file_path).ok()?.modified().ok()?;
        mtime
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs() as i64)
    }

    fn calculate_md5(file_path: &Path) -> Option<String> {
        log::debug!("计算 MD5: {}", file_path.display());
        let mut file = std::fs::File::open(file_path).ok()?;
        let mut ctx = md5::Context::new();
        std::io::copy(&mut file, &mut ctx).ok()?;
        let digest = ctx.compute();
        Some(format!("{:x}", digest))
    }

    pub async fn clear(&self, file_path: &Path) {
        let rel_path = match file_path.strip_prefix(&self.game_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => return,
        };

        let mut inner = self.inner.write().await;
        if inner.entries.remove(&rel_path).is_some() {
            inner.updated = true;
        }
    }
}
