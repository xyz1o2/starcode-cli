//! StarLens 漂移检测：对比 baseline + 现有 entry 集合与磁盘现状。
//!
//! 纯函数式分类（对标 docs/starlens-design.md §6）：
//! - `Stale`：对象在 baseline、磁盘也在，但 sha 变了 → entry 过期待重算；
//! - `Missing`：baseline 有、磁盘已无；
//! - `Orphan`：有语义 entry，但现在已不是 Index 角色（文件没了或降级）→ 待清理；
//! - `Unbaselined`：磁盘有且在 scope 内（Index/Observe），但 baseline 没记 → 待补。

use super::lens::ScopeRole;
use super::lens_build::Baseline;
use super::lens_scope::{evaluate, ScopeProfile};
use crate::core::utils::paths::sha256_hex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriftReport {
    pub stale: Vec<String>,
    pub missing: Vec<String>,
    pub orphan: Vec<String>,
    pub unbaselined: Vec<String>,
}

impl DriftReport {
    pub fn is_aligned(&self) -> bool {
        self.stale.is_empty()
            && self.missing.is_empty()
            && self.orphan.is_empty()
            && self.unbaselined.is_empty()
    }

    pub fn total(&self) -> usize {
        self.stale.len() + self.missing.len() + self.orphan.len() + self.unbaselined.len()
    }
}

/// 扫描磁盘当前状态：非 Exclude 文件 → (role, sha)。
fn scan_current(root: &Path, profile: ScopeProfile) -> BTreeMap<String, (ScopeRole, String)> {
    let mut map = BTreeMap::new();
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
        let role = evaluate(&rel, profile);
        if role == ScopeRole::Exclude {
            continue;
        }
        if let Ok(bytes) = std::fs::read(path) {
            map.insert(rel, (role, sha256_hex(&bytes)));
        }
    }
    map
}

/// 对比 baseline + 现有 entry 集合与磁盘现状，产出漂移报告。
pub fn detect_drift(
    root: &Path,
    profile: ScopeProfile,
    baseline: &Baseline,
    index_objects: &BTreeSet<String>,
) -> DriftReport {
    let current = scan_current(root, profile);
    let mut report = DriftReport::default();

    // Stale / Unbaselined：以磁盘为准。
    for (obj, (_role, sha)) in &current {
        match baseline.objects.get(obj) {
            None => report.unbaselined.push(obj.clone()),
            Some(base_sha) if base_sha != sha => report.stale.push(obj.clone()),
            Some(_) => {}
        }
    }
    // Missing：baseline 有、磁盘无。
    for obj in baseline.objects.keys() {
        if !current.contains_key(obj) {
            report.missing.push(obj.clone());
        }
    }
    // Orphan：有 entry 但现在已不是 Index 角色（文件没了，或降级为 Observe/Exclude）。
    for obj in index_objects {
        let still_index = matches!(current.get(obj), Some((ScopeRole::Index, _)));
        if !still_index {
            report.orphan.push(obj.clone());
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::lens_build::build_structural_index;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_root() -> std::path::PathBuf {
        let uniq = format!(
            "lens_drift_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(uniq)
    }

    fn index_set(idx: &crate::core::context::lens::WholeIndex) -> BTreeSet<String> {
        idx.entries.iter().map(|e| e.object.clone()).collect()
    }

    #[test]
    fn fresh_build_is_aligned() {
        let root = tmp_root();
        fs::create_dir_all(root.join("src/runtime")).unwrap();
        fs::write(root.join("src/runtime/a.rs"), "pub fn a() {}\n").unwrap();
        let b = build_structural_index(&root, ScopeProfile::Core);
        let report = detect_drift(&root, ScopeProfile::Core, &b.baseline, &index_set(&b.index));
        assert!(report.is_aligned(), "unexpected drift: {report:?}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn detects_stale_missing_orphan_unbaselined() {
        let root = tmp_root();
        fs::create_dir_all(root.join("src/runtime")).unwrap();
        fs::write(root.join("src/runtime/a.rs"), "pub fn a() {}\n").unwrap();
        fs::write(root.join("src/runtime/d.rs"), "pub fn d() {}\n").unwrap();
        let b = build_structural_index(&root, ScopeProfile::Core);
        let objs = index_set(&b.index);

        // 变更磁盘：改 a.rs（stale）、删 d.rs（missing+orphan）、新增 c.rs（unbaselined）。
        fs::write(root.join("src/runtime/a.rs"), "pub fn a() { let x = 1; }\n").unwrap();
        fs::remove_file(root.join("src/runtime/d.rs")).unwrap();
        fs::write(root.join("src/runtime/c.rs"), "pub fn c() {}\n").unwrap();

        let r = detect_drift(&root, ScopeProfile::Core, &b.baseline, &objs);
        assert!(r.stale.contains(&"src/runtime/a.rs".to_string()), "{r:?}");
        assert!(r.missing.contains(&"src/runtime/d.rs".to_string()), "{r:?}");
        assert!(r.orphan.contains(&"src/runtime/d.rs".to_string()), "{r:?}");
        assert!(
            r.unbaselined.contains(&"src/runtime/c.rs".to_string()),
            "{r:?}"
        );
        assert!(!r.is_aligned());

        fs::remove_dir_all(&root).ok();
    }
}
