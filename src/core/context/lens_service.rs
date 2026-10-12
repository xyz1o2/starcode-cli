//! StarLens 门面：环境门控 + 加载/重建/漂移检查，把下层模块串成单一入口。
//!
//! 门控默认全关（`STAR_LENS_ENABLED` 未设即 false），渐进开启；真正的注入与
//! PreToolUse nudge 由调用方在确认 `is_enabled()` 后接入。

use super::lens::WholeIndex;
use super::lens_build::{build_structural_index, persist, Baseline, BuildResult, BuildStats};
use super::lens_drift::{detect_drift, DriftReport};
use super::lens_extract::extract_structural;
use super::lens_scope::{evaluate, ScopeProfile};
use crate::core::config::storage::Storage;
use crate::core::context::lens::ScopeRole;
use crate::core::utils::paths::sha256_hex;
use std::collections::BTreeSet;

fn parse_enabled(v: Option<&str>) -> bool {
    matches!(v.map(|s| s.trim()), Some("1" | "true" | "yes" | "on"))
}

fn parse_profile(v: Option<&str>) -> ScopeProfile {
    match v {
        Some(s) if s.trim().eq_ignore_ascii_case("full") => ScopeProfile::Full,
        _ => ScopeProfile::Core,
    }
}

/// 总开关：`STAR_LENS_ENABLED`，默认关。
pub fn is_enabled() -> bool {
    parse_enabled(std::env::var("STAR_LENS_ENABLED").ok().as_deref())
}

/// Scope profile：`STAR_LENS_SCOPE_PROFILE`，`full` → Full，其余 → Core（默认）。
pub fn scope_profile() -> ScopeProfile {
    parse_profile(std::env::var("STAR_LENS_SCOPE_PROFILE").ok().as_deref())
}

/// 读盘加载现有索引与基线。缺任一文件返回 None。
pub fn load(storage: &Storage) -> Option<(WholeIndex, Baseline)> {
    let text = std::fs::read_to_string(storage.lens_index_path()).ok()?;
    let baseline_raw = std::fs::read_to_string(storage.lens_baseline_path()).ok()?;
    Some((
        WholeIndex::from_text(&text),
        Baseline::from_json(&baseline_raw)?,
    ))
}

/// 结构-only 重建并落盘。
pub fn rebuild(storage: &Storage, profile: ScopeProfile) -> std::io::Result<BuildResult> {
    let result = build_structural_index(&storage.project_root(), profile);
    persist(storage, &result)?;
    Ok(result)
}

/// 对已构建 index 逐条做语义增强（读源码 → cheap model 填 F/R/S）。返回成功条数。
pub async fn enrich_index(
    index: &mut WholeIndex,
    root: &std::path::Path,
    client: &crate::llm::client::StarClient,
) -> usize {
    let mut n = 0usize;
    for e in index.entries.iter_mut() {
        if let Ok(content) = std::fs::read_to_string(root.join(&e.object)) {
            if super::lens_enrich::enrich_entry_with_client(e, &content, client).await {
                n += 1;
            }
        }
    }
    n
}

/// 完整重建：结构 build → 语义 enrich → persist。返回 (结果, 增强成功条数)。
pub async fn rebuild_enriched(
    storage: &Storage,
    profile: ScopeProfile,
    client: &crate::llm::client::StarClient,
) -> std::io::Result<(BuildResult, usize)> {
    let mut result = build_structural_index(&storage.project_root(), profile);
    let enriched = enrich_index(&mut result.index, &storage.project_root(), client).await;
    persist(storage, &result)?;
    Ok((result, enriched))
}

/// 对盘上索引做漂移检查。无索引返回 None。
pub fn check_drift(storage: &Storage, profile: ScopeProfile) -> Option<DriftReport> {
    let (index, baseline) = load(storage)?;
    let objs: BTreeSet<String> = index.entries.iter().map(|e| e.object.clone()).collect();
    Some(detect_drift(
        &storage.project_root(),
        profile,
        &baseline,
        &objs,
    ))
}

/// 增量更新：检测漂移 → 只重算受影响对象（结构-only）→ 落盘。完成 §6 的"检测→标脏→重算"闭环。
/// 返回更新前检测到的漂移报告（已对齐则 index/baseline 不动）。无索引返回 None。
pub fn update(storage: &Storage, profile: ScopeProfile) -> Option<DriftReport> {
    let (mut index, mut baseline) = load(storage)?;
    let root = storage.project_root();
    let objs: BTreeSet<String> = index.entries.iter().map(|e| e.object.clone()).collect();
    let drift = detect_drift(&root, profile, &baseline, &objs);
    if drift.is_aligned() {
        return Some(drift);
    }

    // 删除 Missing + Orphan：entry 清掉；Missing 还要从 baseline 移除。
    let gone: BTreeSet<&String> = drift.missing.iter().chain(drift.orphan.iter()).collect();
    index.entries.retain(|e| !gone.contains(&e.object));
    for obj in &drift.missing {
        baseline.objects.remove(obj);
    }

    // 重算 Stale + Unbaselined：按当前角色处理。
    for obj in drift.stale.iter().chain(drift.unbaselined.iter()) {
        let Ok(bytes) = std::fs::read(root.join(obj)) else {
            continue;
        };
        let sha = sha256_hex(&bytes);
        baseline.objects.insert(obj.clone(), sha.clone());
        if evaluate(obj, profile) == ScopeRole::Index {
            if let Ok(content) = String::from_utf8(bytes) {
                let fresh = extract_structural(obj, &content, ScopeRole::Index, sha);
                match index.entries.iter_mut().find(|e| &e.object == obj) {
                    Some(slot) => *slot = fresh,
                    None => index.entries.push(fresh),
                }
            }
        } else {
            // 降级为非 Index：确保不残留旧 entry。
            index.entries.retain(|e| &e.object != obj);
        }
    }

    let result = BuildResult {
        index,
        baseline,
        stats: BuildStats::default(),
    };
    persist(storage, &result).ok()?;
    Some(drift)
}

/// 取注入用的 Overview 文本，按预算截断。无索引/空返回 None。
/// 门控交给调用方（先查 `is_enabled()`），本函数只负责读取与截断。
pub fn overview_for_injection(storage: &Storage, max_chars: usize) -> Option<String> {
    let text = std::fs::read_to_string(storage.lens_index_path()).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    if text.chars().count() <= max_chars {
        Some(text)
    } else {
        let head: String = text.chars().take(max_chars).collect();
        Some(format!(
            "{head}\n# … (truncated at {max_chars} chars; read .star/lens/lens.txt for the rest)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_root() -> std::path::PathBuf {
        let uniq = format!(
            "lens_service_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(uniq);
        // 必须自带 .star：`Storage::new` 会从传入目录向上找第一个已存在的 .star
        // 并把 project_root 改成它的父目录。临时目录通常挂在 %TEMP% 下，而开发机上
        // ~/（家目录）几乎一定有 .star，于是 project_root 被吸附到家目录，
        // rebuild 就去遍历整个 home —— 本地跑到天荒地老，CI 上因为 runner 干净
        // 才侥幸通过。放一个 .star 进去就能把查找截断在这个临时项目里。
        std::fs::create_dir_all(root.join(".star")).unwrap();
        root
    }

    #[test]
    fn env_parsing_is_pure_and_defaulted() {
        assert!(!parse_enabled(None));
        assert!(!parse_enabled(Some("0")));
        assert!(parse_enabled(Some("1")));
        assert!(parse_enabled(Some(" true ")));
        assert_eq!(parse_profile(None), ScopeProfile::Core);
        assert_eq!(parse_profile(Some("FULL")), ScopeProfile::Full);
        assert_eq!(parse_profile(Some("core")), ScopeProfile::Core);
    }

    #[test]
    fn rebuild_then_load_round_trips_and_is_aligned() {
        let root = tmp_root();
        fs::create_dir_all(root.join("src/runtime")).unwrap();
        fs::write(root.join("src/runtime/a.rs"), "pub fn a() {}\n").unwrap();
        let storage = Storage::new(root.clone());

        let built = rebuild(&storage, ScopeProfile::Core).unwrap();
        assert!(built.stats.indexed >= 1);

        let (index, baseline) = load(&storage).expect("load");
        assert!(index.entries.iter().any(|e| e.object == "src/runtime/a.rs"));
        assert!(baseline.objects.contains_key("src/runtime/a.rs"));

        let drift = check_drift(&storage, ScopeProfile::Core).expect("drift");
        assert!(drift.is_aligned(), "{drift:?}");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn overview_truncates_to_budget() {
        let root = tmp_root();
        fs::create_dir_all(root.join("src/runtime")).unwrap();
        fs::write(root.join("src/runtime/a.rs"), "pub fn a() {}\n").unwrap();
        let storage = Storage::new(root.clone());
        assert_eq!(
            storage.project_root(),
            root,
            "测试必须隔离在临时项目里，不能被上层目录的 .star 改写"
        );
        rebuild(&storage, ScopeProfile::Core).unwrap();

        let full = overview_for_injection(&storage, 100_000).unwrap();
        assert!(full.contains("@src/runtime/a.rs"));
        let clipped = overview_for_injection(&storage, 20).unwrap();
        assert!(clipped.contains("truncated at 20 chars"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn update_refreshes_only_affected_and_realigns() {
        let root = tmp_root();
        fs::create_dir_all(root.join("src/runtime")).unwrap();
        fs::write(root.join("src/runtime/a.rs"), "pub fn a() {}\n").unwrap();
        fs::write(root.join("src/runtime/d.rs"), "pub fn d() {}\n").unwrap();
        let storage = Storage::new(root.clone());
        rebuild(&storage, ScopeProfile::Core).unwrap();

        // 改 a.rs（stale）、删 d.rs（missing+orphan）、增 c.rs（unbaselined）。
        fs::write(root.join("src/runtime/a.rs"), "pub fn a() { let x = 1; }\n").unwrap();
        fs::remove_file(root.join("src/runtime/d.rs")).unwrap();
        fs::write(root.join("src/runtime/c.rs"), "pub fn c() {}\n").unwrap();

        let pre = update(&storage, ScopeProfile::Core).expect("update");
        assert!(!pre.is_aligned(), "更新前应检测到漂移");

        // 更新后重新检测应已对齐。
        assert!(check_drift(&storage, ScopeProfile::Core)
            .unwrap()
            .is_aligned());

        let (idx, base) = load(&storage).unwrap();
        assert!(idx.entries.iter().any(|e| e.object == "src/runtime/c.rs"));
        assert!(!idx.entries.iter().any(|e| e.object == "src/runtime/d.rs"));
        assert!(!base.objects.contains_key("src/runtime/d.rs"));

        fs::remove_dir_all(&root).ok();
    }
}
