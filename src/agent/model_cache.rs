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
use std::collections::{HashMap, HashSet};
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

/// 同 `load`，但缓存文件路径由调用方指定。
///
/// 给"先用缓存填 UI"这类路径用：那里需要在发请求之前同步读一次，
/// 测试则可以指向临时文件，不必去动 `STAR_MODEL_CACHE_PATH` 这个进程级
/// 环境变量（并行跑测试时会互相串味）。
pub fn load_from_path(path: &Path, provider_id: &str) -> Option<CachedModels> {
    load_from(path, provider_id)
}

/// 同 `save`，但缓存文件路径由调用方指定。
pub fn save_to_path(path: &Path, provider_id: &str, models: &[ModelInfo]) {
    if let Err(e) = save_to(path, provider_id, models) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ModelCache] failed to persist model list: {}",
            e
        ));
    }
}

/// 写入某个 provider 的缓存。失败只记日志 —— 缓存写不进去不该影响模型切换。
pub fn save(provider_id: &str, models: &[ModelInfo]) {
    save_to_path(&cache_path(), provider_id, models);
}

/// 在**所有** provider 桶里查某个模型名归谁，返回 provider id。
///
/// # 为什么必须跨桶查
///
/// 模型列表是按 provider 分桶存的，所以一次全表扫描就能回答"这个名字归谁"。
/// 切换模型时缺了这一步很致命：`model_provider_map` 只装**当前列表**的映射，
/// 面板之外手输的名字（例如 OpenRouter 的 `xiaomi/mimo-v2.6-flash`）查不到就
/// 兜底到"当前 provider"，于是 OpenRouter 的模型名被发给了 Zen，上游回
/// `Model is unavailable` / `is not a valid model ID` —— 表现为"切换模型没反应"。
///
/// `DEFAULT_BUCKET` 不参与：那是"没配 provider 时的兜底桶"，不是真正的归属。
/// 同名模型命中多个 provider 时优先 `preferred`（通常是当前 provider），否则按
/// id 排序取第一个，保证结果稳定、可复现。
/// # `allowed` 为什么必须有
///
/// 磁盘缓存**没有生命周期**：删掉 provider 时程序不会顺手删掉它那个桶（外部工具
/// 直接改配置、进程在删除与冲刷之间崩掉，都会留下残留）。于是全桶扫描会把一个
/// 早已删除的 provider 认领走，调用方拿它去发切换请求，worker 校验时以
/// `Provider '<x>' is not configured.` 拒掉 —— 表现为"模型切不动"。
///
/// `allowed` 是调用方手上"仍然算数的 provider"集合，命中不了它的桶一律跳过。
/// `None` = 不限制（仅供测试与"缓存就是权威"的场景）。
pub fn find_provider_for_model_from(
    path: &Path,
    model: &str,
    preferred: Option<&str>,
    allowed: Option<&HashSet<String>>,
) -> Option<String> {
    let cache = read_all(path);
    let mut candidates: Vec<&String> = cache
        .providers
        .iter()
        .filter(|(provider_id, entry)| {
            provider_id.as_str() != DEFAULT_BUCKET
                && entry.models.iter().any(|m| m.id == model)
                && allowed.is_none_or(|set| set.contains(provider_id.as_str()))
        })
        .map(|(provider_id, _)| provider_id)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort();
    if let Some(preferred) = preferred {
        if let Some(exact) = candidates.iter().find(|id| id.as_str() == preferred) {
            return Some((*exact).clone());
        }
    }
    Some(candidates[0].clone())
}

/// 同 [`find_provider_for_model_from`]，但路径取默认的缓存文件。
pub fn find_provider_for_model(
    model: &str,
    preferred: Option<&str>,
    allowed: Option<&HashSet<String>>,
) -> Option<String> {
    find_provider_for_model_from(&cache_path(), model, preferred, allowed)
}

/// 删掉某个 provider 的桶。
///
/// 桶的寿命必须和 provider 本身一致。留下已删除 provider 的桶，后果不是"多占
/// 一点磁盘"，而是 `find_provider_for_model_from` 的全桶扫描会把里面的模型名
/// 认领给那个已经不存在的 provider，切换请求因此被 worker 以
/// `Provider '<x>' is not configured.` 拒掉 —— 用户看到的就是"模型切不动"。
///
/// 外部工具直接改配置时同样会留下残留，所以 `find_provider_for_model_*` 还带了
/// `allowed` 过滤做第二道防线；这里是第一道，把已知的情况清干净。
pub fn purge_from(path: &Path, provider_id: &str) {
    let mut cache = read_all(path);
    if cache.providers.remove(bucket(provider_id)).is_none() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(&cache) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[ModelCache] failed to purge bucket '{}': {}",
                    bucket(provider_id),
                    e
                ));
            } else {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[ModelCache] purged bucket '{}' for removed provider",
                    bucket(provider_id)
                ));
            }
        }
        Err(e) => crate::utils::logging::append_debug_log_line(&format!(
            "[ModelCache] could not serialise cache while purging '{}': {}",
            bucket(provider_id),
            e
        )),
    }
}

/// 同 [`purge_from`]，但路径取默认的缓存文件。
pub fn purge(provider_id: &str) {
    purge_from(&cache_path(), provider_id);
}

/// 删掉**所有**不在 `live_providers` 里的桶，返回被删掉的 id。
///
/// # 为什么不能只查当前 provider 的桶
///
/// 之前只在"要读当前 provider 的缓存"时顺手清那一个桶。问题是**没被访问到的桶永远
/// 清不到**：`current_provider_id` 是 `local`，那 `apinex` / `dev` / `magpie` 三个桶
/// 就一直躺着。它们的模型会通过 provider 归属混进模型面板，用户看到一个自己早就
/// 删掉、面板里也找不到的 provider 名。
///
/// 实测过的一个完整残留现场（`user-settings.json` 只有 `local` + `opencode`，缓存里
/// 却有 5 个桶）：
///
/// ```text
/// [ORPHAN] apinex  132 models
/// [ORPHAN] dev     158 models
/// [ORPHAN] magpie   88 models
/// [OK    ] local   158 models
/// [OK    ] opencode 84 models
/// ```
///
/// `dev` 和 `local` 模型数一模一样 —— 那是把 `dev` 改名成 `local` 时留下的旧桶。
/// 所以这里做一次**全量清扫**，而不是逐个去碰。
///
/// `live_providers` 应包含内建 provider（它们可能根本没被配置过，只是列在
/// `ALL_PROVIDERS` 里），调用方负责把两者并起来。
pub fn purge_orphans_from(path: &Path, live_providers: &HashSet<String>) -> Vec<String> {
    let mut cache = read_all(path);
    if cache.providers.is_empty() {
        return Vec::new();
    }
    let orphans: Vec<String> = cache
        .providers
        .keys()
        .filter(|id| id.as_str() != DEFAULT_BUCKET && !live_providers.contains(id.as_str()))
        .cloned()
        .collect();
    if orphans.is_empty() {
        return Vec::new();
    }
    for id in &orphans {
        cache.providers.remove(id);
    }
    let serialized = match serde_json::to_string_pretty(&cache) {
        Ok(json) => json,
        Err(e) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ModelCache] could not serialise cache while purging orphans: {}",
                e
            ));
            return Vec::new();
        }
    };
    if let Err(e) = std::fs::write(path, serialized) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ModelCache] failed to purge orphan buckets {:?}: {}",
            orphans, e
        ));
        return Vec::new();
    }
    crate::utils::logging::append_debug_log_line(&format!(
        "[ModelCache] purged {} orphan bucket(s) for removed providers: {:?}",
        orphans.len(),
        orphans
    ));
    orphans
}

/// 同 [`purge_orphans_from`]，但路径取默认的缓存文件。
pub fn purge_orphans(live_providers: &HashSet<String>) -> Vec<String> {
    purge_orphans_from(&cache_path(), live_providers)
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

    /// 回归：跨网关的模型名必须查到真正拥有它的 provider。
    ///
    /// 真实事故：当前 provider 是 Zen（OpenCode），用户在 `/model` 里敲了
    /// OpenRouter 的 `xiaomi/mimo-v2.6-flash`。只查当前列表就查不到，于是兜底
    /// 留在 Zen，Zen 回 `Model is unavailable` —— 表现为"切换模型没反应"。
    #[test]
    fn finds_the_owning_provider_across_buckets_not_just_the_current_one() {
        let path = temp_path("cross-bucket");
        let _ = std::fs::remove_file(&path);

        save_to(
            &path,
            "openrouter",
            &[ModelInfo::new("xiaomi/mimo-v2.6-flash", "openrouter")],
        )
        .unwrap();
        save_to(
            &path,
            "opencode",
            &[ModelInfo::new("mimo-v2.6-flash-free", "opencode")],
        )
        .unwrap();

        // 当前 provider 是 opencode，但这个名字归 openrouter。
        assert_eq!(
            find_provider_for_model_from(&path, "xiaomi/mimo-v2.6-flash", Some("opencode"), None)
                .as_deref(),
            Some("openrouter"),
            "必须路由到真正拥有该模型的网关，而不是留在当前 provider"
        );
        // 反向同理。
        assert_eq!(
            find_provider_for_model_from(&path, "mimo-v2.6-flash-free", Some("openrouter"), None)
                .as_deref(),
            Some("opencode")
        );
        // 谁都没有的名字才返回 None，交给调用方兜底到当前 provider。
        assert_eq!(
            find_provider_for_model_from(&path, "totally-unknown-model", Some("openrouter"), None),
            None
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_default_bucket_is_not_treated_as_a_real_owner() {
        let path = temp_path("default-owner");
        let _ = std::fs::remove_file(&path);

        save_to(&path, "", &[ModelInfo::new("some-local-model", "local")]).unwrap();

        // `__default` 只是"没配 provider"时的兜底桶，不能当成归属，
        // 否则会把模型名路由到一个根本不存在的 provider。
        assert_eq!(
            find_provider_for_model_from(&path, "some-local-model", None, None),
            None
        );

        let _ = std::fs::remove_file(&path);
    }

    /// 残留桶必须被 `allowed` 挡掉 —— 这正是"刚删掉 provider 就切不动模型"的成因：
    /// 桶还在磁盘上，全桶扫描认领走一个已经不存在的 provider，worker 以
    /// `Provider 'x' is not configured.` 静默拒绝。
    #[test]
    fn buckets_of_removed_providers_are_never_returned() {
        let path = temp_path("removed-owner");
        let _ = std::fs::remove_file(&path);

        save_to(
            &path,
            "magpie",
            &[ModelInfo::new("deepseek-v4.1-flash", "magpie")],
        )
        .unwrap();
        save_to(
            &path,
            "yunjunet",
            &[ModelInfo::new("deepseek-v4.1-pro", "yunjunet")],
        )
        .unwrap();

        let live: HashSet<String> = ["yunjunet".to_string()].into_iter().collect();

        // magpie 已从配置里删除 —— 它的桶不能再认领任何模型。
        assert_eq!(
            find_provider_for_model_from(&path, "deepseek-v4.1-flash", None, Some(&live)),
            None,
            "已删除 provider 的残留桶不得再被认领"
        );
        // 还活着的照常。
        assert_eq!(
            find_provider_for_model_from(&path, "deepseek-v4.1-pro", None, Some(&live)).as_deref(),
            Some("yunjunet")
        );

        let _ = std::fs::remove_file(&path);
    }

    /// `preferred` 也必须在 `allowed` 之内 —— 否则"当前 provider 恰好是那个残留桶"
    /// 会把死 provider 顶上来。
    #[test]
    fn preferred_cannot_resurrect_a_removed_provider() {
        let path = temp_path("removed-preferred");
        let _ = std::fs::remove_file(&path);

        save_to(
            &path,
            "magpie",
            &[ModelInfo::new("deepseek-v4.1-flash", "magpie")],
        )
        .unwrap();
        save_to(
            &path,
            "yunjunet",
            &[ModelInfo::new("deepseek-v4.1-flash", "yunjunet")],
        )
        .unwrap();

        // magpie 和 yunjunet 都提供这个名字，但 magpie 已删除。
        let live: HashSet<String> = ["yunjunet".to_string()].into_iter().collect();
        assert_eq!(
            find_provider_for_model_from(&path, "deepseek-v4.1-flash", Some("magpie"), Some(&live))
                .as_deref(),
            Some("yunjunet"),
            "preferred 指向已删除 provider 时不能把它顶上来"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// 全量清扫必须把**所有**孤儿桶一次清掉 —— 只查当前 provider 那一个是不够的。
    ///
    /// 实测现场：`user-settings.json` 只有 `local` + `opencode`，缓存里却坐着
    /// `apinex` / `dev` / `magpie` 三个桶。而 `current_provider_id` 是 `local`，那三个
    /// 永远不会被访问到，也就永远清不掉。
    #[test]
    fn purge_orphans_removes_every_bucket_not_in_the_live_set() {
        let path = temp_path("purge-orphans");
        let _ = std::fs::remove_file(&path);

        for id in ["local", "opencode", "apinex", "dev", "magpie", "astrdark"] {
            save_to(&path, id, &[ModelInfo::new("some-model", id)]).unwrap();
        }

        let live: HashSet<String> = ["local".to_string(), "opencode".to_string()]
            .into_iter()
            .collect();
        let removed = purge_orphans_from(&path, &live);

        assert_eq!(
            removed.len(),
            4,
            "四个不属于任何存活 provider 的桶都该被清掉，实际清掉 {removed:?}"
        );
        assert!(load_from(&path, "local").is_some(), "local 必须留下");
        assert!(load_from(&path, "opencode").is_some(), "opencode 必须留下");
        for gone in ["apinex", "dev", "magpie", "astrdark"] {
            assert!(
                load_from(&path, gone).is_none(),
                "{gone} 是孤儿桶，必须消失"
            );
        }

        // 再跑一次应当是空操作（幂等）。
        assert!(purge_orphans_from(&path, &live).is_empty());

        let _ = std::fs::remove_file(&path);
    }

    /// 内建 provider 的桶不能被当成孤儿 —— 它们可能压根没进过配置。
    #[test]
    fn purge_orphans_keeps_builtin_buckets_when_the_caller_lists_them_as_live() {
        let path = temp_path("purge-orphans-builtin");
        let _ = std::fs::remove_file(&path);
        save_to(&path, "opencode", &[ModelInfo::new("m", "opencode")]).unwrap();

        // 调用方把内建 id 并进 live 集合 —— 这正是 `list_models_with_mode` 的做法。
        let live: HashSet<String> = ["opencode".to_string()].into_iter().collect();
        assert!(purge_orphans_from(&path, &live).is_empty());
        assert!(load_from(&path, "opencode").is_some());

        let _ = std::fs::remove_file(&path);
    }

    /// `purge` 必须让桶真的消失，而不只是把它从内存里摘掉。
    #[test]
    fn purge_removes_the_bucket_from_disk() {
        let path = temp_path("purge");
        let _ = std::fs::remove_file(&path);

        save_to(&path, "magpie", &[ModelInfo::new("a-model", "magpie")]).unwrap();
        save_to(&path, "yunjunet", &[ModelInfo::new("b-model", "yunjunet")]).unwrap();

        purge_from(&path, "magpie");

        assert!(
            load_from(&path, "magpie").is_none(),
            "删掉 provider 后它的桶不能再被读到"
        );
        assert!(
            load_from(&path, "yunjunet").is_some(),
            "删一个桶不能连带删掉别人的"
        );

        // 再 purge 一次不该出错，也不该清空别人的桶。
        purge_from(&path, "magpie");
        assert!(load_from(&path, "yunjunet").is_some());

        let _ = std::fs::remove_file(&path);
    }
}
