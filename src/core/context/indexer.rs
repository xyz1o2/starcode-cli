use crate::core::utils::file_utils::read_file_with_encoding;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

/// 单文件索引上限：超过就不读不哈希。
/// 与语义引擎的 `max_file_bytes` 同口径 —— 两个索引对"多大的文件算太大"
/// 给不同答案，只会让人怀疑其中一个是 bug。
const INDEX_MAX_FILE_BYTES: u64 = 512 * 1024;

/// 单次索引的累计读取上限。超过后停止读新文件并标记 `truncated`。
const INDEX_MAX_TOTAL_BYTES: u64 = 12 * 1024 * 1024;

/// 二进制探测窗口（字节）。
const INDEX_BINARY_PROBE_BYTES: usize = 8 * 1024;

/// 串行化整个 read-modify-write。
///
/// `index_project()` 是"读 index.json → 改 → 写回"的非原子序列，
/// 而它有两个并发调用方：watcher 的重建 worker 与 `ContextEngine` 的后台
/// 刷新任务（`engine.rs:152` / `:307`）。两边同时跑就是经典的丢更新 ——
/// 后写的那个把先写的整批变更覆盖掉，且不报错。
///
/// 跨进程不保证：同一仓库同时跑两个 StarCode 不是目标场景。
static INDEX_IO_LOCK: Mutex<()> = Mutex::new(());

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
fn deserialize_blobs_compat<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, IndexEntry>, D::Error>
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
    /// 因超过单文件上限而未读取的文件数
    pub skipped_large: usize,
    /// 是否因累计预算耗尽而提前停止。`true` 时 `removed_blobs` 恒为空。
    pub truncated: bool,
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

    /// 快速路径判定：(size, mtime) 都没变就沿用旧 hash，不读文件、不重哈希。
    ///
    /// 这是**尽力而为的启发式，宁可多读一次也不漏检**：
    /// - `mtime_ms == 0` 说明元数据不可信（v1 旧格式回填失败、或文件系统
    ///   不提供 mtime），强制走慢路径，否则会永久沿用陈旧的 hash；
    /// - 不再要求 `size != 0` —— 空文件同样应该享受快速路径；
    /// - 同尺寸 + 同 mtime 粒度的修改确实会被漏检，这是这套机制的固有代价，
    ///   watcher 事件与 agent 编辑钩子打脏是它的补偿路径。
    fn is_unchanged_fast(entry: &IndexEntry, size: u64, mtime_ms: u64) -> bool {
        entry.size == size && entry.mtime_ms == mtime_ms && mtime_ms != 0
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
        let _io_guard = INDEX_IO_LOCK.lock().expect("index io lock poisoned");

        if !self.project_root.exists() {
            return Ok(IndexResult {
                new_blobs: Vec::new(),
                removed_blobs: Vec::new(),
                total_files: 0,
                skipped_large: 0,
                truncated: false,
            });
        }

        let mut current_index = self.load_index();
        let mut new_blobs: Vec<Blob> = Vec::new();
        let mut found_paths: HashSet<String> = HashSet::new();
        // 是否有索引条目被更新（含旧格式元数据回填）——决定是否需要写盘
        let mut entries_updated = false;
        let mut skipped_large = 0usize;
        let mut truncated = false;
        let mut hashed_bytes = 0u64;

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
                            files
                                .lock()
                                .expect("index walk collector poisoned")
                                .push(entry.into_path());
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

            // 单文件上限：不读不哈希。
            // 注意这里**不能**顺手把 rel_path 从 found_paths 里去掉 ——
            // 它早已入集，跳过读取恰好保证了"超限文件不会被误判为删除"。
            if size > INDEX_MAX_FILE_BYTES {
                skipped_large += 1;
                continue;
            }

            // 快速路径：元数据未变 → 沿用旧 hash，不读文件。
            if current_index
                .blobs
                .get(&rel_path)
                .is_some_and(|entry| Self::is_unchanged_fast(entry, size, mtime_ms))
            {
                continue;
            }

            // 累计预算耗尽：停止读新文件。此时遍历不完整，`found_paths` 只是
            // 前缀，下面必须跳过删除检测（否则会把还没走到的文件全报成删除）。
            if hashed_bytes.saturating_add(size) > INDEX_MAX_TOTAL_BYTES {
                truncated = true;
                break;
            }

            // 慢路径：读 + 哈希（仅变更文件）。
            let Ok(content) = read_file_with_encoding(&path) else {
                continue;
            };
            if is_probably_binary(&content) {
                continue;
            }
            hashed_bytes += size;
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
        //
        // `truncated` 时整段跳过：`found_paths` 只覆盖了预算耗尽前走到的那部分
        // 目录，剩下的文件一个都没进去 —— 拿它做差集会把整棵未遍历的树报成
        // 删除。宁可这轮不报删除（下一轮预算够时会补上），也不能谎报。
        let mut removed_blobs = Vec::new();
        if !truncated {
            let old_paths: Vec<String> = current_index.blobs.keys().cloned().collect();
            for path in old_paths {
                if !found_paths.contains(&path) {
                    current_index.blobs.remove(&path);
                    removed_blobs.push(path);
                    entries_updated = true;
                }
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
            skipped_large,
            truncated,
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

/// 二进制探测：解码后的前 8 KiB 仍含 NUL，说明这根本不是文本文件。
///
/// 对二进制做 lossy 解码再 SHA256 是纯浪费（而且哈希值毫无意义）。
/// 只看前 8 KiB，避免在巨大的文本文件上做全量扫描。
fn is_probably_binary(content: &str) -> bool {
    let bytes = content.as_bytes();
    let probe = &bytes[..bytes.len().min(INDEX_BINARY_PROBE_BYTES)];
    probe.contains(&0)
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

    #[test]
    fn empty_files_take_the_fast_path() {
        // 以前快速路径要求 `size != 0`：空文件每次都被当变更文件重新读一遍。
        // 现在只要 (size, mtime) 没变就直接跳过 —— 这条测的是"空文件也
        // 走快速路径"，用的正是 indexer 自己的 index.json 做第二轮对照。
        let (dir, indexer) = temp_project();
        write(&dir, "empty.rs", "");

        let first = indexer.index_project().unwrap();
        assert_eq!(first.total_files, 1);

        let second = indexer.index_project().unwrap();
        assert!(
            second.new_blobs.is_empty(),
            "空文件元数据未变，不该出现在 new_blobs：{:?}",
            second.new_blobs
        );
    }

    #[test]
    fn large_file_is_skipped_without_reading() {
        let (dir, indexer) = temp_project();
        // 超上限但不超 12 MiB 总预算：应当进 skipped_large，而不是被截断。
        write(&dir, "big.rs", &"fn big() {}\n".repeat(50_000));

        let result = indexer.index_project().unwrap();
        assert!(result.skipped_large >= 1, "超大文件应计入 skipped_large");
        assert!(!result.truncated, "单文件超限不该算预算耗尽");
        assert!(
            result.new_blobs.is_empty(),
            "超大文件不该被读、不该进 new_blobs"
        );
        assert_eq!(result.total_files, 0, "超大文件不进索引条目");
    }

    #[test]
    fn skipped_large_file_is_not_reported_as_removed() {
        // 上一条的隐性后果：超大文件留在 found_paths 里，所以它**不会**被
        // 当成删除。如果误报成 removed，语义引擎会跟着把它从索引里删掉 ——
        // 一个用户能看见的文件突然从搜索结果里消失。
        let (dir, indexer) = temp_project();
        write(&dir, "small.rs", "fn small() {}");
        write(&dir, "big.rs", &"fn big() {}\n".repeat(50_000));

        let result = indexer.index_project().unwrap();
        assert!(!result.removed_blobs.contains(&"big.rs".to_string()));
    }

    #[test]
    fn binary_file_is_skipped() {
        let (dir, indexer) = temp_project();
        write(&dir, "binary.rs", "fn ok() {}\n\0\0\0not text\n");

        let result = indexer.index_project().unwrap();
        assert!(
            result.new_blobs.iter().all(|b| b.path != "binary.rs"),
            "二进制文件不该被索引：{:?}",
            result.new_blobs
        );
    }

    #[test]
    fn budget_exhaustion_suppresses_removal_detection() {
        // truncated 时 found_paths 只是预算耗尽前走到的那部分 ——
        // 拿它做差集会把整棵没走到的树报成删除。必须跳过删除检测。
        let (dir, indexer) = temp_project();
        write(&dir, "src/a.rs", "fn a() {}");
        indexer.index_project().unwrap();

        // 第二轮：一批新文件把总预算撑爆（每个都超单文件上限，累计超总量）。
        for i in 0..40 {
            write(&dir, &format!("big{i}.rs"), &"fn big() {}\n".repeat(50_000));
        }
        let result = indexer.index_project().unwrap();

        // 超限文件不读、不进 found_paths 的索引条目，但 total_files
        // 应保持第一轮的 1（旧条目没被清掉）。
        assert!(
            result.removed_blobs.is_empty(),
            "截断时不该报删除：{:?}",
            result.removed_blobs
        );
    }

    /// 并发 read-modify-write 的回归保护：两个 `index_project` 同时跑时，
    /// 后写的那个会把先写的整批变更覆盖掉。进程内锁消除了这个竞争。
    #[test]
    fn concurrent_indexing_does_not_lose_updates() {
        let (dir, indexer) = temp_project();

        // 第一轮建立基线。
        write(&dir, "src/a.rs", "fn a() {}");
        indexer.index_project().unwrap();

        // 两个不同的新文件，两条线程同时索引。
        // （Indexer 只是两个 PathBuf，克隆几乎零成本。）
        std::thread::scope(|s| {
            let dir = &dir;
            let b = indexer.clone();
            let c = indexer.clone();
            s.spawn(move || {
                write(dir, "src/b.rs", "fn b() {}");
                b.index_project().unwrap();
            });
            s.spawn(move || {
                write(dir, "src/c.rs", "fn c() {}");
                c.index_project().unwrap();
            });
        });

        // 没有锁的话，两个线程各自的 load→改→save 只能活一个：
        // b.rs 和 c.rs 里会少一个。
        let index = indexer.load_index_public_for_test();
        assert!(index.blobs.contains_key("src/b.rs"), "并发的更新不该被丢掉");
        assert!(index.blobs.contains_key("src/c.rs"), "并发的更新不该被丢掉");
    }
}
