use crate::core::utils::file_utils::read_file_with_encoding;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Blob {
    pub hash: String,
    pub path: String,
    /// 已废弃：不再携带文件全文。旧实现把全文写进 `.star/context/cas/`，
    /// 首跑等于把整个代码库复制一份 —— 这正是后台索引卡启动/卡首任务的
    /// IO 风暴根源之一。字段保留以兼容既有调用方签名，恒为 `None`。
    #[serde(skip)]
    pub content: Option<String>,
}

/// 单文件索引条目：内容 hash + 快速变更检测元数据。
///
/// (size, mtime) 都未变 → 直接沿用旧 hash，**不读文件、不重哈希**
/// （Cursor/Zoekt 式增量检测）。二次索引成本从 O(全库读) 降为 O(目录遍历)。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexEntry {
    pub hash: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mtime_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ProjectIndex {
    /// 相对路径 → 索引条目
    #[serde(deserialize_with = "deserialize_blobs_compat")]
    pub blobs: HashMap<String, IndexEntry>,
}

/// `blobs` 字段的兼容反序列化：新格式 `HashMap<String, IndexEntry>`，
/// 旧格式（v1）`HashMap<String, String>`。旧条目 size/mtime 记 0，
/// 下一轮索引会读一次文件做元数据回填。
fn deserialize_blobs_compat<'de, D>(deserializer: D) -> Result<HashMap<String, IndexEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum LegacyOrNew {
        New(HashMap<String, IndexEntry>),
        Legacy(HashMap<String, String>),
    }

    match LegacyOrNew::deserialize(deserializer)? {
        LegacyOrNew::New(map) => Ok(map),
        LegacyOrNew::Legacy(map) => Ok(map
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    IndexEntry {
                        hash: v,
                        size: 0,
                        mtime_ms: 0,
                    },
                )
            })
            .collect()),
    }
}

#[derive(Debug, Clone)]
pub struct IndexResult {
    /// 相对路径列表：新出现或 (size,mtime) 或 hash 发生变化的文件
    pub new_blobs: Vec<Blob>,
    /// 已删除文件的相对路径
    pub removed_blobs: Vec<String>,
    pub total_files: usize,
}

#[derive(Clone)]
pub struct Indexer {
    project_root: PathBuf,
    index_file: PathBuf,
}

impl Indexer {
    pub fn new(project_root: &Path) -> Self {
        let context_dir = project_root.join(".star").join("context");
        let index_file = context_dir.join("index.json");
        Self {
            project_root: project_root.to_path_buf(),
            index_file,
        }
    }

    fn load_index(&self) -> ProjectIndex {
        if self.index_file.exists() {
            if let Ok(content) = fs::read_to_string(&self.index_file) {
                if let Ok(index) = serde_json::from_str::<ProjectIndex>(&content) {
                    return index;
                }
                // 旧格式（HashMap<String,String>）由自定义 Deserialize 兜底；
                // 走到这里说明文件本身损坏，回退全量重建。
            }
        }
        ProjectIndex::default()
    }

    /// 原子持久化：紧凑 JSON + tmp 文件 + rename 替换。
    /// 旧实现是 pretty JSON 全量重写 —— 大仓上既慢又在写中途崩溃时会截断文件。
    fn save_index(&self, index: &ProjectIndex) -> Result<(), std::io::Error> {
        if let Some(parent) = self.index_file.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string(index)?;
        let tmp = self.index_file.with_extension("json.tmp");
        fs::write(&tmp, content)?;
        fs::rename(&tmp, &self.index_file)?;
        Ok(())
    }

    fn calculate_hash(&self, content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content);
        format!("{:x}", hasher.finalize())
    }

    /// 全库索引（天然增量）：并行遍历目录收集文件清单后逐个检测变更。
    /// - (size, mtime) 未变 → 沿用旧 hash，零读取；
    /// - 元数据变了才 read + SHA256；
    /// - 二次索引成本 ≈ 一次目录遍历 + 变更文件的读取。
    pub fn index_project(&self) -> Result<IndexResult, Box<dyn std::error::Error + Send + Sync>> {
        if !self.project_root.exists() {
            return Ok(IndexResult {
                new_blobs: Vec::new(),
                removed_blobs: Vec::new(),
                total_files: 0,
            });
        }

        let mut current_index = self.load_index();
        let mut new_blobs: Vec<Blob> = Vec::new();
        let mut found_paths: HashSet<String> = HashSet::new();
        // 是否有索引条目被更新（含旧格式元数据回填）——决定是否需要写盘
        let mut entries_updated = false;

        // 三层 ignore（`~/.star/ignore` → `.starignore` → `.gitignore`，
        // `require_git(false)`）就是从 `utils::file_walk` 抽出来的，全树共用同一份口径。
        // WalkParallel：多线程并行目录遍历（ignore crate 与 ripgrep 同源）。
        let files: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let walker = crate::utils::file_walk::walk_builder(
            &self.project_root,
            &crate::utils::file_walk::WalkOptions::new(),
        )
        .build_parallel();

        let files_for_walk = Arc::clone(&files);
        walker.run(move || {
            let files = Arc::clone(&files_for_walk);
            Box::new(move |result: Result<ignore::DirEntry, ignore::Error>| {
                match result {
                    Ok(entry) => {
                        if entry.file_type().is_some_and(|t| t.is_file()) {
                            files.lock().expect("index walk collector poisoned").push(entry.into_path());
                        }
                    }
                    Err(err) => crate::utils::logging::append_debug_log_line(&format!(
                        "[Context] Walk error while indexing {}: {}",
                        self.project_root.display(),
                        err
                    )),
                }
                ignore::WalkState::Continue
            })
        });

        let collected = files.lock().expect("index walk collector poisoned").clone();
        for path in collected {
            let rel_path = match path.strip_prefix(&self.project_root) {
                Ok(p) => p.to_string_lossy().replace("\\", "/"),
                Err(_) => continue,
            };
            // 排除工具自身的状态目录与 git 内部目录：
            // 否则 索引→写盘→事件→再索引 的自触发循环，且索引条目被污染。
            if rel_path.starts_with(".star/") || rel_path.starts_with(".git/") || rel_path == ".git"
            {
                continue;
            }
            found_paths.insert(rel_path.clone());

            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            let size = meta.len();
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            // 快速路径：元数据未变 → 沿用旧 hash，不读文件。
            if let Some(entry) = current_index.blobs.get(&rel_path) {
                if entry.size == size && entry.mtime_ms == mtime_ms && entry.size != 0 {
                    continue;
                }
            }

            // 慢路径：读 + 哈希（仅变更文件）。
            let Ok(content) = read_file_with_encoding(&path) else {
                continue;
            };
            let hash = self.calculate_hash(&content);
            let unchanged = current_index
                .blobs
                .get(&rel_path)
                .is_some_and(|e| e.hash == hash);
            current_index.blobs.insert(
                rel_path.clone(),
                IndexEntry {
                    hash: hash.clone(),
                    size,
                    mtime_ms,
                },
            );
            entries_updated = true;
            if !unchanged {
                new_blobs.push(Blob {
                    hash,
                    path: rel_path,
                    content: None,
                });
            }
        }

        // Identify removed files
        let mut removed_blobs = Vec::new();
        let old_paths: Vec<String> = current_index.blobs.keys().cloned().collect();
        for path in old_paths {
            if !found_paths.contains(&path) {
                current_index.blobs.remove(&path);
                removed_blobs.push(path);
                entries_updated = true;
            }
        }

        // Save updated index（仅在有变更时写盘，未变的运行零写入）
        if entries_updated {
            self.save_index(&current_index)?;
        }

        Ok(IndexResult {
            new_blobs,
            removed_blobs,
            total_files: current_index.blobs.len(),
        })
    }

    /// 索引摘要（供状态行/日志使用）
    pub fn summary_json(&self) -> serde_json::Value {
        let index = self.load_index();
        json!({
            "total_files": index.blobs.len(),
        })
    }

    #[cfg(test)]
    pub(crate) fn load_index_public_for_test(&self) -> ProjectIndex {
        self.load_index()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project() -> (tempfile::TempDir, Indexer) {
        let dir = tempfile::tempdir().expect("tempdir");
        let indexer = Indexer::new(dir.path());
        (dir, indexer)
    }

    fn write(dir: &tempfile::TempDir, rel: &str, content: &str) {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn first_run_indexes_all_then_second_run_skips_unchanged() {
        let (dir, indexer) = temp_project();
        write(&dir, "src/a.rs", "fn a() {}");
        write(&dir, "src/b.rs", "fn b() {}");

        let first = indexer.index_project().expect("first index");
        assert_eq!(first.total_files, 2);
        assert_eq!(first.new_blobs.len(), 2);

        // 二次索引：(size,mtime) 未变 → 零读取、零新增。
        let second = indexer.index_project().expect("second index");
        assert_eq!(second.total_files, 2);
        assert!(
            second.new_blobs.is_empty(),
            "未变文件不应出现在 new_blobs: {:?}",
            second.new_blobs
        );
    }

    #[test]
    fn changed_file_is_detected_and_unchanged_is_not() {
        let (dir, indexer) = temp_project();
        write(&dir, "src/a.rs", "fn a() {}");
        write(&dir, "src/b.rs", "fn b() {}");
        indexer.index_project().unwrap();

        // 保证 mtime 有变化（部分文件系统 mtime 粒度较粗）。
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&dir, "src/a.rs", "fn a_changed() {}");

        let result = indexer.index_project().expect("third index");
        assert_eq!(result.new_blobs.len(), 1);
        assert_eq!(result.new_blobs[0].path, "src/a.rs");
        assert!(result.new_blobs[0].content.is_none(), "Blob 不再携带全文");
    }

    #[test]
    fn removed_file_is_reported() {
        let (dir, indexer) = temp_project();
        write(&dir, "src/a.rs", "fn a() {}");
        indexer.index_project().unwrap();

        fs::remove_file(dir.path().join("src/a.rs")).unwrap();
        let result = indexer.index_project().expect("index after removal");
        assert_eq!(result.removed_blobs, vec!["src/a.rs".to_string()]);
        assert_eq!(result.total_files, 0);
    }

    #[test]
    fn legacy_index_format_is_deserialized() {
        let (dir, indexer) = temp_project();
        let legacy = r#"{"blobs": {"src/a.rs": "deadbeef"}}"#;
        fs::create_dir_all(dir.path().join(".star/context")).unwrap();
        fs::write(dir.path().join(".star/context/index.json"), legacy).unwrap();

        let index = indexer.load_index_public_for_test();
        let entry = index.blobs.get("src/a.rs").expect("legacy entry kept");
        assert_eq!(entry.hash, "deadbeef");
        assert_eq!(entry.size, 0, "旧条目无元数据，下一轮会读一次回填");
    }

    #[test]
    fn index_file_is_atomic_and_compact() {
        let (dir, indexer) = temp_project();
        write(&dir, "src/a.rs", "fn a() {}");
        indexer.index_project().unwrap();

        let raw = fs::read_to_string(dir.path().join(".star/context/index.json")).unwrap();
        assert!(!raw.contains("\n  "), "持久化必须是紧凑 JSON，非 pretty");
        assert!(!dir.path().join(".star/context/index.json.tmp").exists());
    }
}
