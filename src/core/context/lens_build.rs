//! StarLens 全仓组装：遍历受管范围，产出结构-only 的 `WholeIndex` 与漂移基线。
//!
//! 本层只做确定性工作——walk + scope 评估 + tree-sitter 结构抽取 + 落盘；
//! 语义半扇 F/R/S 由后续 cheap model 管线增强（见 docs/starlens-design.md P1）。

use super::lens::{ScopeRole, WholeIndex};
use super::lens_extract::extract_structural;
use super::lens_scope::{evaluate, ScopeProfile};
use crate::core::utils::paths::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// 单文件读取上限：超过只记指纹、不解析，避免把超大文件读进内存。
const MAX_FILE_BYTES: u64 = 512 * 1024;

/// 漂移基线：受管对象 → 源内容 sha256。(de)serialize 为 `baseline.json`。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub version: u32,
    pub objects: BTreeMap<String, String>,
}

impl Baseline {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Option<Baseline> {
        serde_json::from_str(s).ok()
    }
}

/// 组装统计，供日志与成功指标用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildStats {
    pub indexed: usize,
    pub observed: usize,
    pub excluded: usize,
    pub skipped_large: usize,
}

pub struct BuildResult {
    pub index: WholeIndex,
    pub baseline: Baseline,
    pub stats: BuildStats,
}

/// 遍历 `root`，按 profile 组装结构-only 索引与基线。纯确定性，不调用模型。
pub fn build_structural_index(root: &Path, profile: ScopeProfile) -> BuildResult {
    let mut index = WholeIndex::default();
    let mut baseline = Baseline { version: 1, objects: BTreeMap::new() };
    let mut stats = BuildStats::default();

    let opts = crate::utils::file_walk::WalkOptions::new();
    for entry in crate::utils::file_walk::walk(root, &opts) {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() {
            continue;
        }

        match evaluate(&rel, profile) {
            ScopeRole::Exclude => stats.excluded += 1,
            ScopeRole::Observe => {
                if let Some(sha) = hash_file(path) {
                    baseline.objects.insert(rel, sha);
                    stats.observed += 1;
                }
            }
            ScopeRole::Index => {
                let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                if len > MAX_FILE_BYTES {
                    if let Some(sha) = hash_file(path) {
                        baseline.objects.insert(rel, sha);
                        stats.skipped_large += 1;
                    }
                    continue;
                }
                let Ok(bytes) = std::fs::read(path) else { continue };
                match String::from_utf8(bytes) {
                    Ok(content) => {
                        let sha = sha256_hex(content.as_bytes());
                        let e = extract_structural(&rel, &content, ScopeRole::Index, sha.clone());
                        index.entries.push(e);
                        baseline.objects.insert(rel, sha);
                        stats.indexed += 1;
                    }
                    Err(e) => {
                        // 非 UTF-8：只记指纹（从错误里取回原始字节），当 observe 处理。
                        let sha = sha256_hex(&e.into_bytes());
                        baseline.objects.insert(rel, sha);
                        stats.observed += 1;
                    }
                }
            }
        }
    }
    BuildResult { index, baseline, stats }
}

fn hash_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| sha256_hex(&b))
}

/// 落盘：写 `lens.txt` + `baseline.json` 到 `.star/lens/`（目录自动创建）。
pub fn persist(
    storage: &crate::core::config::storage::Storage,
    result: &BuildResult,
) -> std::io::Result<()> {
    std::fs::create_dir_all(storage.lens_dir())?;
    std::fs::write(storage.lens_index_path(), result.index.to_text())?;
    std::fs::write(storage.lens_baseline_path(), result.baseline.to_json())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_root() -> std::path::PathBuf {
        let uniq = format!(
            "lens_build_{}_{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        );
        std::env::temp_dir().join(uniq)
    }

    #[test]
    fn builds_index_for_core_observes_rest_excludes_junk() {
        let root = tmp_root();
        fs::create_dir_all(root.join("src/runtime")).unwrap();
        fs::create_dir_all(root.join("src/utils")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join("src/runtime/messages.rs"), "pub fn a() {}\n").unwrap();
        fs::write(root.join("src/utils/helper.rs"), "pub fn b() {}\n").unwrap();
        fs::write(root.join("target/debug/junk.rs"), "fn c() {}\n").unwrap();
        fs::write(root.join("assets.png"), "not utf8 image bytes").unwrap();

        let r = build_structural_index(&root, ScopeProfile::Core);

        // 核心子系统文件进 index。
        assert!(r.index.entries.iter().any(|e| e.object == "src/runtime/messages.rs"));
        // 非核心源码只进 baseline（observe），不进 index。
        assert!(r.baseline.objects.contains_key("src/utils/helper.rs"));
        assert!(!r.index.entries.iter().any(|e| e.object == "src/utils/helper.rs"));
        // target/ 被排除，既不进 index 也不进 baseline。
        assert!(!r.baseline.objects.keys().any(|k| k.contains("target/")));
        assert!(r.stats.indexed >= 1 && r.stats.excluded >= 1);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn persist_writes_both_files() {
        let root = tmp_root();
        fs::create_dir_all(root.join("src/runtime")).unwrap();
        fs::write(root.join("src/runtime/messages.rs"), "pub fn a() {}\n").unwrap();
        let r = build_structural_index(&root, ScopeProfile::Core);

        let storage = crate::core::config::storage::Storage::new(root.clone());
        persist(&storage, &r).unwrap();
        assert!(storage.lens_index_path().exists());
        assert!(storage.lens_baseline_path().exists());
        let text = fs::read_to_string(storage.lens_index_path()).unwrap();
        assert!(text.contains("@src/runtime/messages.rs"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn baseline_json_round_trips() {
        let mut b = Baseline { version: 1, objects: BTreeMap::new() };
        b.objects.insert("a.rs".into(), "deadbeef".into());
        assert_eq!(Baseline::from_json(&b.to_json()).unwrap(), b);
    }
}

