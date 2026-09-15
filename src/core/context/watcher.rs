//! 后台索引协调器：watcher（notify）+ 防抖 + 低优先级增量重建。
//!
//! 设计目标（对标 Augment ACE / Cursor 的实时索引，且绝不阻塞前台）：
//! - **零阻塞**：所有索引工作跑在 `THREAD_PRIORITY_LOWEST` 后台线程，
//!   前台（首任务、用户输入）永远优先。
//! - **防抖合并**：watcher 事件与 agent 编辑钩子统一汇入脏集合，
//!   300ms 内的连续变更合并为一次重建。
//! - **增量重建**：重建入口是 `Indexer::index_project()` —— 得益于
//!   (size, mtime) 快速跳过，它天然只读变更文件。
//! - **单一事实源**：agent 的 Edit/Write 打脏、semantic_search 发现
//!   引擎过期、watcher 文件事件，三条路径都汇入同一个重建队列。

use crate::core::context::indexer::Indexer;
use parking_lot::{Condvar, Mutex};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// 重建防抖窗口：窗口内的连续变更合并为一次。
const DEBOUNCE: Duration = Duration::from_millis(300);

/// 把当前线程降为后台优先级（Windows: THREAD_PRIORITY_LOWEST；
/// 其它平台按 thread-priority crate 的映射）。
/// 索引是纯粹的后台工作，必须给前台（首任务、用户交互）让路。
pub fn lower_current_thread_priority() {
    let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Min);
}

struct CoordInner {
    /// 待重建的相对/绝对文件路径（仅供日志与状态展示；重建本身是全量入口的增量执行）
    dirty: HashSet<String>,
    project_root: Option<PathBuf>,
}

/// 全局索引协调器。
pub struct IndexCoordinator {
    inner: Mutex<CoordInner>,
    /// 脏集合变化信号（worker 从 recv_timeout 醒来后检查）
    dirty_signal: Condvar,
    generation: AtomicU64,
    ready: AtomicBool,
    started: AtomicBool,
    rebuilding: AtomicBool,
    last_rebuild_failed: AtomicBool,
    /// 语义搜索引擎缓存（agent_core lazy_init 时注入）
    engine_cache: Mutex<Option<Arc<crate::core::context::search_cache::SearchEngineCacheManager>>>,
}

static COORDINATOR: OnceLock<IndexCoordinator> = OnceLock::new();

fn coordinator() -> &'static IndexCoordinator {
    COORDINATOR.get_or_init(|| IndexCoordinator {
        inner: Mutex::new(CoordInner {
            dirty: HashSet::new(),
            project_root: None,
        }),
        dirty_signal: Condvar::new(),
        generation: AtomicU64::new(0),
        ready: AtomicBool::new(false),
        started: AtomicBool::new(false),
        rebuilding: AtomicBool::new(false),
        last_rebuild_failed: AtomicBool::new(false),
        engine_cache: Mutex::new(None),
    })
}

/// 注入语义搜索引擎缓存（`agent_core::lazy_init` 调用一次）。
pub fn set_engine_cache(cache: Arc<crate::core::context::search_cache::SearchEngineCacheManager>) {
    *coordinator().engine_cache.lock() = Some(cache);
}

/// 标记文件已变更（agent 的 Edit/Write/multi_edit 成功后调用）。
/// 若协调器尚未启动，以文件所在工作目录惰性启动。
pub fn mark_files_dirty(paths: impl IntoIterator<Item = String>) {
    let coord = coordinator();
    let root = {
        let mut inner = coord.inner.lock();
        if inner.project_root.is_none() {
            if let Ok(cwd) = std::env::current_dir() {
                inner.project_root = Some(cwd);
            }
        }
        for p in paths {
            inner.dirty.insert(p);
        }
        coord.dirty_signal.notify_one();
        inner.project_root.clone()
    };
    if let Some(root) = root {
        ensure_started(&root);
    }
}

/// 请求一次完整增量刷新（semantic_search 发现引擎过期时调用）。
/// 防抖合并，不会叠加执行。
pub fn request_refresh(project_root: &Path) {
    let coord = coordinator();
    {
        let mut inner = coord.inner.lock();
        if inner.project_root.is_none() {
            inner.project_root = Some(project_root.to_path_buf());
        }
        inner.dirty.insert("\0refresh-all".to_string());
        coord.dirty_signal.notify_one();
    }
    ensure_started(project_root);
}

/// 启动 watcher + 重建 worker（幂等）。
pub fn ensure_started(project_root: &Path) {
    let coord = coordinator();
    if coord.started.swap(true, Ordering::SeqCst) {
        return;
    }

    {
        let mut inner = coord.inner.lock();
        if inner.project_root.is_none() {
            inner.project_root = Some(project_root.to_path_buf());
        }
    }

    let root = project_root.to_path_buf();

    // ── watcher：文件系统事件 → 脏集合 ─────────────────────────────────
    // notify 的 recommended_watcher 在独立内部线程回调，这里只把
    // 变更路径塞进脏集合，完全不碰索引。
    let (tx, rx): (Sender<notify::Result<notify::Event>>, Receiver<_>) = mpsc::channel();
    let watcher_result = notify::recommended_watcher(
        move |res: notify::Result<notify::Event>| {
            // channel 满了就丢弃（下一轮全量增量重建会补上），绝不阻塞回调线程
            let _ = tx.send(res);
        },
    );
    let Ok(mut watcher) = watcher_result else {
        crate::utils::logging::append_debug_log_line(
            "[Context] notify watcher init failed; falling back to query-triggered refresh",
        );
        coord.started.store(false, Ordering::SeqCst);
        return;
    };
    if let Err(err) = notify::Watcher::watch(
        &mut watcher,
        project_root,
        notify::RecursiveMode::Recursive,
    ) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[Context] notify watch({}) failed: {}; falling back to query-triggered refresh",
            project_root.display(),
            err
        ));
        coord.started.store(false, Ordering::SeqCst);
        return;
    }
    // watcher 实例必须保活
    std::mem::forget(watcher);

    // ── 重建 worker：防抖合并 + 低优先级增量重建 ────────────────────────
    std::thread::Builder::new()
        .name("star-index-worker".into())
        .spawn(move || rebuild_worker_loop(root, rx))
        .map(|_| ())
        .unwrap_or_else(|e| {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Context] index worker spawn failed: {}",
                e
            ));
            coordinator().started.store(false, Ordering::SeqCst);
        });

    crate::utils::logging::append_debug_log_line(&format!(
        "[Context] index coordinator started (watching {})",
        project_root.display()
    ));
}

/// worker 主循环：收集事件 → 防抖 → 增量重建。
/// notify 事件的接收超时就是防抖窗口 —— 窗口内新事件持续刷新，
/// 静默满 300ms 才执行一次重建。
fn rebuild_worker_loop(root: PathBuf, rx: Receiver<notify::Result<notify::Event>>) {
    lower_current_thread_priority();
    let coord = coordinator();

    loop {
        // 收集一批事件
        let mut got_event = false;
        loop {
            match rx.try_recv() {
                Ok(Ok(event)) => {
                    got_event = true;
                    let mut inner = coord.inner.lock();
                    for path in event.paths {
                        if let Ok(rel) = path.strip_prefix(&root) {
                            let rel = rel.to_string_lossy().replace('\\', "/");
                            // 索引自身的持久化文件不进脏集合，否则重建→写盘→事件→重建 死循环
                            if rel.starts_with(".star/") {
                                continue;
                            }
                            inner.dirty.insert(rel);
                        } else {
                            inner.dirty.insert("\0refresh-all".to_string());
                        }
                    }
                    if matches!(event.kind, notify::EventKind::Remove(_)) {
                        coord.dirty_signal.notify_one();
                    }
                }
                Ok(Err(err)) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[Context] watch event error: {}",
                        err
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        if got_event {
            coord.dirty_signal.notify_one();
        }

        // 防抖等待：静默满 DEBOUNCE 才动手
        let has_pending = {
            let mut inner = coord.inner.lock();
            if !inner.dirty.is_empty() {
                true
            } else {
                // 等待脏信号或超时（超时后同样检查，watcher 事件由本线程自己收）
                let _ = coord
                    .dirty_signal
                    .wait_for(&mut inner, DEBOUNCE);
                !inner.dirty.is_empty()
            }
        };

        if has_pending {
            let snapshot: Vec<String> = {
                let mut inner = coord.inner.lock();
                std::mem::take(&mut inner.dirty).into_iter().collect()
            };
            run_incremental_rebuild(&root, &snapshot);
        }
    }
}

/// 增量重建：indexer（(size,mtime) 快速跳过）→ 语义引擎后台重建 → 状态翻转。
fn run_incremental_rebuild(root: &Path, dirty: &[String]) {
    let coord = coordinator();
    if coord.rebuilding.swap(true, Ordering::SeqCst) {
        // 上一轮还在跑：把脏集合留下（已由调用方清空，这里重新标记全量）
        let mut inner = coord.inner.lock();
        inner.dirty.insert("\0refresh-all".to_string());
        return;
    }

    let started_at = Instant::now();
    lower_current_thread_priority();

    // 1. 文件索引增量更新（(size,mtime) 未变的文件零读取）
    let indexer = Indexer::new(root);
    let index_result = indexer.index_project();

    match &index_result {
        Ok(result) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Context] incremental reindex: {} changed, {} removed, {} total ({:?}) in {:?}",
                result.new_blobs.len(),
                result.removed_blobs.len(),
                result.total_files,
                dirty.first(),
                started_at.elapsed()
            ));
        }
        Err(err) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Context] incremental reindex failed: {}",
                err
            ));
        }
    }

    // 2. 语义搜索引擎后台重建（有缓存管理器时）
    if let Some(cache) = coord.engine_cache.lock().clone() {
        let built =
            crate::core::tools::semantic_search::build_engine_into_cache(root, &cache, None);
        match built {
            Ok(stats) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[Context] semantic engine rebuilt in background: {} files in {:?}",
                    stats,
                    started_at.elapsed()
                ));
                coord.last_rebuild_failed.store(false, Ordering::SeqCst);
            }
            Err(err) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[Context] background engine rebuild failed: {}",
                    err
                ));
                coord.last_rebuild_failed.store(true, Ordering::SeqCst);
            }
        }
    }

    coord.generation.fetch_add(1, Ordering::SeqCst);
    coord.ready.store(true, Ordering::SeqCst);
    coord.rebuilding.store(false, Ordering::SeqCst);
    coord.dirty_signal.notify_one();
}

/// 协调器是否已启动（查询路径据此决定冷启动策略）。
pub fn is_started() -> bool {
    coordinator().started.load(Ordering::SeqCst)
}

/// 语义引擎是否就绪（供查询路径决定是否提示 warming）。
pub fn is_engine_ready() -> bool {
    coordinator().ready.load(Ordering::SeqCst)
}

/// 是否有后台重建正在进行。
pub fn is_rebuilding() -> bool {
    coordinator().rebuilding.load(Ordering::SeqCst)
}

/// 上次后台重建是否失败（失败时查询路径应回退同步构建，避免死循环 warming）。
pub fn last_rebuild_failed() -> bool {
    coordinator().last_rebuild_failed.load(Ordering::SeqCst)
}

/// 状态摘要（日志/状态行用）。
pub fn status_text() -> String {
    let coord = coordinator();
    let dirty_len = coord.inner.lock().dirty.len();
    let state = if coord.rebuilding.load(Ordering::SeqCst) {
        "rebuilding"
    } else if coord.ready.load(Ordering::SeqCst) {
        "ready"
    } else if coord.started.load(Ordering::SeqCst) {
        "warming"
    } else {
        "off"
    };
    format!(
        "index: {} (gen {}, {} dirty)",
        state,
        coord.generation.load(Ordering::SeqCst),
        dirty_len
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_dirty_starts_coordinator_and_collects_paths() {
        let dir = tempfile::tempdir().unwrap();
        let coord = coordinator();
        // 全局协调器可能被并行测试先播种过 root（engine/semantic 查询路径
        // 也会 request_refresh），因此这里不断言 root 归属，只验证：
        // 1) 幂等启动 2) 脏集合收集到标记文件。
        request_refresh(dir.path());
        coord.inner.lock().dirty.clear();

        mark_files_dirty(vec![dir.path().join("a.rs").to_string_lossy().to_string()]);

        let inner = coord.inner.lock();
        assert!(coord.started.load(Ordering::SeqCst));
        assert!(inner
            .dirty
            .iter()
            .any(|p| p.ends_with("a.rs")), "脏集合应包含标记的文件");
    }

    #[test]
    fn status_text_reflects_state() {
        let s = status_text();
        assert!(s.starts_with("index:"));
    }
}
