//! 模型列表的磁盘缓存。
//!
//! # 为什么需要它
//!
//! `model_list` 原来只有一个进程内的 60s TTL 缓存（`MODEL_CACHE`）。进程一退出
//! 缓存就没了，于是每次启动后第一次打开 `/model` 都要重新全量拉取：活动 provider
//! 那次 `list_models()` 连超时都没有，其余每个已配置 provider 再各开一个 3s 超时的
//! 并发请求，并且要 `join_all` 等最慢的那个。用户看到的就是"每次自动获取太慢"。
//!
//! 这里把拉取结果按 provider 落盘，冷启动直接命中，打开面板不再等网络；真正需要
//! 更新列表时走显式刷新（面板里的 `⟳` 项，`force = true`）。
//!
//! # 两个刻意的选择
//!
//! - **不设过期。** 模型列表变动很少，静默失效只会让"慢"随机回来。缓存里记下拉取
//!   时间，面板上显示"多久之前拉的"，要不要刷新交给用户。
//! - **按 provider 分桶。** 切换 provider 时只清内存缓存（`clear_model_cache`），
//!   磁盘上各家的列表都留着，来回切换都是秒开。

use crate::types::ModelInfo;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 磁盘缓存文件格式。`version` 用于以后改结构时直接丢弃旧文件。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct DiskCache {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    providers: HashMap<String, ProviderEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ProviderEntry {
    /// 拉取时刻的 Unix 秒。用 wall clock 而不是 `Instant`，因为要跨进程。
    fetched_at: u64,
    models: Vec<ModelInfo>,
}

const CACHE_VERSION: u32 = 1;

/// provider_id 为空时用的桶名（例如只配了环境变量、没有 providers.json 的情况）。
pub(crate) const DEFAULT_BUCKET: &str = "__default";

/// 命中的缓存条目。
pub struct CachedModels {
    pub models: Vec<ModelInfo>,
    /// 距离拉取时刻过了多少秒。时钟回拨时按 0 处理。
    pub age_secs: u64,
}

fn cache_path() -> PathBuf {
    // 允许覆盖，方便测试与多环境隔离；缺省是 ~/.star/model-cache.json。
    if let Ok(p) = std::env::var("STAR_MODEL_CACHE_PATH") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::core::config::storage::Storage::model_cache_path()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn bucket(provider_id: &str) -> &str {
    if provider_id.trim().is_empty() {
        DEFAULT_BUCKET
    } else {
        provider_id
    }
}

/// 读取整个缓存文件。文件不存在、读不动、或 JSON 坏了都当"没有缓存"。
fn read_all(path: &Path) -> DiskCache {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return DiskCache::default();
    };
    match serde_json::from_str::<DiskCache>(&raw) {
        Ok(cache) if cache.version == CACHE_VERSION => cache,
        Ok(_) => {
            crate::utils::logging::append_debug_log_line(
                "[ModelCache] discarding cache written by a different version",
            );
            DiskCache::default()
        }
        Err(e) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ModelCache] unreadable cache file, ignoring: {}",
                e
            ));
            DiskCache::default()
        }
    }
}

fn load_from(path: &Path, provider_id: &str) -> Option<CachedModels> {
    let cache = read_all(path);
    let entry = cache.providers.get(bucket(provider_id))?;
    if entry.models.is_empty() {
        return None;
    }
    Some(CachedModels {
        models: entry.models.clone(),
        age_secs: now_secs().saturating_sub(entry.fetched_at),
    })
}

fn save_to(path: &Path, provider_id: &str, models: &[ModelInfo]) -> std::io::Result<()> {
    if models.is_empty() {
        return Ok(());
    }
    let mut cache = read_all(path);
    cache.version = CACHE_VERSION;
    cache.providers.insert(
        bucket(provider_id).to_string(),
        ProviderEntry {
            fetched_at: now_secs(),
            models: models.to_vec(),
        },
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

/// 取某个 provider 的缓存列表；没有就返回 `None`。
pub fn load(provider_id: &str) -> Option<CachedModels> {
    let hit = load_from(&cache_path(), provider_id);
    if let Some(hit) = &hit {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ModelCache] disk hit for '{}': {} models, {}s old",
            bucket(provider_id),
            hit.models.len(),
            hit.age_secs
        ));
    }
    hit
}

/// 写入某个 provider 的缓存。失败只记日志 —— 缓存写不进去不该影响模型切换。
pub fn save(provider_id: &str, models: &[ModelInfo]) {
    if let Err(e) = save_to(&cache_path(), provider_id, models) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ModelCache] failed to persist model list: {}",
            e
        ));
    }
}

/// 删除整个缓存文件（`/model` 的显式刷新不需要它，留给排查问题时用）。
pub fn clear() {
    let path = cache_path();
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let name = format!("star-model-cache-{}-{}.json", tag, std::process::id());
        std::env::temp_dir().join(name)
    }

    fn sample() -> Vec<ModelInfo> {
        vec![
            ModelInfo::new("gpt-5", "openai"),
            ModelInfo::new("claude-opus-5", "anthropic"),
        ]
    }

    #[test]
    fn saved_models_come_back_with_an_age() {
        let path = temp_path("roundtrip");
        let _ = std::fs::remove_file(&path);

        save_to(&path, "openai", &sample()).expect("save should succeed");
        let hit = load_from(&path, "openai").expect("just-saved entry should load");

        assert_eq!(hit.models.len(), 2);
        assert_eq!(hit.models[0].id, "gpt-5");
        // 刚写完，年龄必然很小；只断言"不是垃圾值"，避免依赖时钟精度。
        assert!(hit.age_secs < 60, "age_secs was {}", hit.age_secs);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn buckets_do_not_bleed_into_each_other() {
        let path = temp_path("buckets");
        let _ = std::fs::remove_file(&path);

        save_to(&path, "openai", &[ModelInfo::new("gpt-5", "openai")]).unwrap();
        save_to(&path, "kimi", &[ModelInfo::new("kimi-k2", "kimi")]).unwrap();

        // 第二次写入不能把第一个 provider 顶掉。
        assert_eq!(load_from(&path, "openai").unwrap().models[0].id, "gpt-5");
        assert_eq!(load_from(&path, "kimi").unwrap().models[0].id, "kimi-k2");
        assert!(load_from(&path, "never-configured").is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_provider_id_gets_its_own_bucket() {
        let path = temp_path("default-bucket");
        let _ = std::fs::remove_file(&path);

        save_to(&path, "", &sample()).unwrap();
        assert_eq!(load_from(&path, "").unwrap().models.len(), 2);
        assert_eq!(load_from(&path, DEFAULT_BUCKET).unwrap().models.len(), 2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_corrupt_or_missing_file_reads_as_no_cache() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ this is not json").unwrap();
        assert!(load_from(&path, "openai").is_none());

        // 坏文件也不能挡住后续写入。
        save_to(&path, "openai", &sample()).unwrap();
        assert!(load_from(&path, "openai").is_some());
        let _ = std::fs::remove_file(&path);

        assert!(load_from(&path, "openai").is_none());
    }

    #[test]
    fn a_future_version_is_ignored_rather_than_deserialized() {
        let path = temp_path("version");
        std::fs::write(
            &path,
            r#"{"version":999,"providers":{"openai":{"fetched_at":0,"models":[]}}}"#,
        )
        .unwrap();
        assert!(load_from(&path, "openai").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_nothing_is_a_no_op() {
        let path = temp_path("empty");
        let _ = std::fs::remove_file(&path);

        save_to(&path, "openai", &[]).unwrap();
        assert!(!path.exists(), "an empty list should not write a file");
    }
}
