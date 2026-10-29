//! 后台索引协调器：watcher（notify）+ 防抖 + 低优先级增量重建。
//!
//! 设计目标（对标 Augment ACE / Cursor 的实时索引，且绝不阻塞前台）：
//! - **零阻塞**：所有索引工作跑在 `THREAD_PRIORITY_LOWEST` 后台线程，
//!   前台（首任务、用户输入）永远优先。
//! - **防抖合并**：watcher 事件与 agent 编辑钩子统一汇入脏集合，
//!   300ms 内的连续变更合并为一次重建。
//! - **增量重建**：重建入口是 `Indexer::index_project()` —— 得益于
//!   (size, mtime) 快速跳过，它天然只读变更文件。
//! - **单一事实源**：agent 的 Edit/Write 打脏、codebase_search 发现
//!   引擎过期、watcher 文件事件，三条路径都汇入同一个重建队列。

use crate::core::context::indexer::Indexer;
use notify::Watcher as _;
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
    /// 0 变更空转重建的抑制计数：每轮空转 +1，每 30s 汇总打一次点并清零。
    ///
    /// 只影响日志，不影响任何逻辑 —— 空转本身无害，刷屏有害。
    /// 实测教训：0 变更重建不抑制时，50 Hz 空转两天写满 2.5 GB 日志。
    idle_noop_count: AtomicU64,
    /// 上次空转汇总打点的时刻。
    last_idle_report: Mutex<Option<Instant>>,
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
        idle_noop_count: AtomicU64::new(0),
        last_idle_report: Mutex::new(None),
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

/// 请求一次完整增量刷新（codebase_search 发现引擎过期时调用）。
/// 防抖合并，不会叠加执行。
pub fn request_refresh(project_root: &Path) {
    let coord = coordinator();
    {
        let mut inner = coord.inner.lock();
        if inner.project_root.is_none() {
            inner.project_root = Some(project_root.to_path_buf());
        }
        inner.dirty.insert("\0refresh-all".to_string());
    }
    ensure_started(project_root);

    // 必须在 worker 线程 spawn 之后再 notify —— parking_lot 的 Condvar
    // 不缓存 notify：在线程启动前发出的通知会被静默丢弃，worker 永远
    // 收不到首轮重建信号，索引一直不构建，查询也就一直返回 Warming。
    {
        let inner = coord.inner.lock();
        coord.dirty_signal.notify_one();
    }
}

/// 已注册的目录 watch 集合。
///
/// 以前这里是 `std::mem::forget(watcher)` —— 整个 watcher 被泄漏掉，换取「它
/// 一直活着」，代价是**没人能再碰它**：既不能在新建目录时补 watch，也不能在
/// 目录删除时回收。改成 `Arc<Mutex<…>>` 之后 worker 线程拿得到它，增删目录才
/// 维护得动。
struct DirWatches {
    watcher: notify::RecommendedWatcher,
    /// 已注册目录（绝对路径），用于去重。
    watched: HashSet<PathBuf>,
}

impl DirWatches {
    /// 注册单个目录；已注册过则跳过。返回是否**真的**注册了。
    fn add(&mut self, dir: &Path) -> bool {
        if !self.watched.insert(dir.to_path_buf()) {
            return false;
        }
        if let Err(err) = self.watcher.watch(dir, notify::RecursiveMode::NonRecursive) {
            // 注册失败必须把占位撤掉：否则这个目录永远处于「已注册」状态，
            // 之后同名目录重建出来也不会再试一次。
            self.watched.remove(dir);
            crate::utils::logging::append_debug_log_line(&format!(
                "[Context] watch({}) failed: {}",
                dir.display(),
                err
            ));
            return false;
        }
        true
    }

    /// 目录被删除或改名：撤掉它下面的所有 watch。
    ///
    /// 不撤的话，目录没了而 watch 还在，白占内核预算 ——
    /// `fs.inotify.max_user_watches` 是**按真实 uid 全局共享**的，你的编辑器、
    /// dev server、别的 watch 工具都从这一个池子里取。
    fn remove_subtree(&mut self, dir: &Path) {
        let stale: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|watched| watched.starts_with(dir))
            .cloned()
            .collect();
        for path in stale {
            let _ = self.watcher.unwatch(&path);
            self.watched.remove(&path);
        }
    }
}

/// 只给「能进索引」的目录注册 watch。
///
/// inotify 的 watch 是**按目录**的（`inotify(7)`：*"Inotify monitoring of
/// directories is not recursive: to monitor subdirectories under a directory,
/// additional watches must be created. This can take a significant amount time
/// for large directory trees."*）。`RecursiveMode::Recursive` 于是会把整棵树的
/// 每个目录都注册一遍 —— 包括那些事件随后会被 [`EventFilter`] 丢掉目录。
///
/// 本仓库 `target/` 下有 6,000+ 个目录，而 `Indexer` 根本不索引它们。注册
/// 6,600 次而只有一百多个有用，这就是慢盘（WSL 的 `/mnt/d`）上首次启动那 80
/// 多秒的来源。
///
/// 这里用 `file_walk::walk` 走一遍 —— 和 `Indexer::index_project()` 同一个
/// 函数、同一套 `.gitignore` 口径。所以「能进索引」不是另立的标准，而是构造上
/// 就成立的：walk 不会 descend 进被忽略的目录，我们拿到的必然是 indexer 也会
/// 看到的那些。
fn register_indexed_dirs(watches: &Arc<Mutex<DirWatches>>, root: &Path) -> usize {
    let opts = crate::utils::file_walk::WalkOptions::new();
    let mut guard = watches.lock();
    // root 自己必须注册：直接建在根下的文件，其父目录事件来自 root。
    let mut count = guard.add(root) as usize;
    for entry in crate::utils::file_walk::walk(root, &opts).flatten() {
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            count += guard.add(entry.path()) as usize;
        }
    }
    count
}

/// 新建目录：补注册它和它下面幸存的子目录。
///
/// `inotify(7)` 明确提醒过这个竞态：*"by the time you create a watch for the new
/// subdirectory, new files (and subdirectories) may already exist inside the
/// subdirectory"* —— 补上 watch 之前塞进去的文件不会产生事件。
///
/// 这里能接受那个窗口，是因为重建入口 `Indexer::index_project()` 自己会全量走
/// 一遍树（靠 `(size, mtime)` 跳过未变文件）：漏掉的事件只会让重建**晚**一轮，
/// 不会让索引漏文件。
fn add_watch_tree(watches: &Arc<Mutex<DirWatches>>, dir: &Path) {
    let opts = crate::utils::file_walk::WalkOptions::new();
    let mut guard = watches.lock();
    guard.add(dir);
    for entry in crate::utils::file_walk::walk(dir, &opts).flatten() {
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            guard.add(entry.path());
        }
    }
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
    let watcher_result = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        // channel 满了就丢弃（下一轮全量增量重建会补上），绝不阻塞回调线程
        let _ = tx.send(res);
    });
    let Ok(watcher) = watcher_result else {
        crate::utils::logging::append_debug_log_line(
            "[Context] notify watcher init failed; falling back to query-triggered refresh",
        );
        coord.started.store(false, Ordering::SeqCst);
        return;
    };

    let watches = Arc::new(Mutex::new(DirWatches {
        watcher,
        watched: HashSet::new(),
    }));

    // 注册必须在 spawn worker **之前**做完：worker 一起来就可能收到事件并需要
    // 访问 `watches`，顺序反了会出现「worker 已在跑、watch 却还没建好」的窗口。
    let registered = register_indexed_dirs(&watches, &root);

    // ── 重建 worker：防抖合并 + 低优先级增量重建 ────────────────────────
    let worker_watches = Arc::clone(&watches);
    std::thread::Builder::new()
        .name("star-index-worker".into())
        .spawn(move || rebuild_worker_loop(root, rx, worker_watches))
        .map(|_| ())
        .unwrap_or_else(|e| {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Context] index worker spawn failed: {}",
                e
            ));
            coordinator().started.store(false, Ordering::SeqCst);
        });

    crate::utils::logging::append_debug_log_line(&format!(
        "[Context] index coordinator started (watching {}, {} dirs)",
        project_root.display(),
        registered
    ));
}

/// worker 主循环：收集事件 → 防抖 → 增量重建。
/// notify 事件的接收超时就是防抖窗口 —— 窗口内新事件持续刷新，
/// 静默满 300ms 才执行一次重建。
fn rebuild_worker_loop(
    root: PathBuf,
    rx: Receiver<notify::Result<notify::Event>>,
    watches: Arc<Mutex<DirWatches>>,
) {
    lower_current_thread_priority();
    let coord = coordinator();
    let filter = build_event_filter(&root);

    loop {
        // 收集一批事件
        let mut got_event = false;
        loop {
            match rx.try_recv() {
                Ok(Ok(event)) => {
                    got_event = true;
                    // 目录增删要维护 watch 集合：注册是 NonRecursive 的，
                    // 所以没人会替我们递归下去。
                    match event.kind {
                        notify::EventKind::Create(_) => {
                            for path in &event.paths {
                                if path.is_dir() && !filter.is_ignored(path) {
                                    add_watch_tree(&watches, path);
                                }
                            }
                        }
                        notify::EventKind::Remove(_) => {
                            for path in &event.paths {
                                watches.lock().remove_subtree(path);
                            }
                        }
                        _ => {}
                    }
                     let mut inner = coord.inner.lock();
                    for path in event.paths {
                        if let Ok(rel) = path.strip_prefix(&root) {
                            let rel = rel.to_string_lossy().replace('\\', "/");
                            // 索引自身的持久化文件不进脏集合，否则重建→写盘→事件→重建 死循环
                            //
                            // 注意 `.star` 目录**本身**也算：Windows 上 notify 会把「watch
                            // 根目录自己的元数据变更」直接以 watch 根路径上报，rel == ".star"
                            // 和 rel == "" 都不指向任何文件，放进来只会引发「0 变更重建
                            // 空转 50 Hz + 刷爆 debug 日志」（实测把 2.5 GB 日志灌满的就是它）。
                            // `.star` 内部的真实写入反正由 `(size, mtime)` 全量增量兜底，
                            // 下一轮真实事件自会把变化补进来。
                            if rel.is_empty() || rel == ".star" || rel.starts_with(".star/") {
                                continue;
                            }
                            // 事件过滤：没有这一层，跑一次 `cargo build` 会灌进
                            // 几千个 `target/` 事件，每轮防抖窗口都被它们刷新，
                            // 重建被无限推迟（甚至永远不触发）。
                            if filter.is_ignored(&path) {
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
                let _ = coord.dirty_signal.wait_for(&mut inner, DEBOUNCE);
                !inner.dirty.is_empty()
            }
        };

        if has_pending {
            let mut snapshot: Vec<String> = {
                let mut inner = coord.inner.lock();
                std::mem::take(&mut inner.dirty).into_iter().collect()
            };
            // HashSet 迭代序是随机的：日志里 dirty.first() 一会 `Some("")`
            // 一会 `Some(".star")`，排查时误导。排序一次，日志稳定可读。
            snapshot.sort();
            run_incremental_rebuild(&root, &snapshot);
        }
    }
}

/// 增量重建：indexer（(size,mtime) 快速跳过）→ 语义引擎增量更新 → 状态翻转。
fn run_incremental_rebuild(root: &Path, dirty: &[String]) {
    let coord = coordinator();
    if coord.rebuilding.swap(true, Ordering::SeqCst) {
        // 上一轮还在跑：把这一轮的脏路径原样放回去，下一轮再处理。
        requeue_dirty_paths(dirty);
        return;
    }

    let started_at = Instant::now();
    lower_current_thread_priority();

    // 1. 文件索引增量更新（(size,mtime) 未变的文件零读取）
    let indexer = Indexer::new(root);
    let index_result = indexer.index_project();

    // 0 变更 = 本轮没有任何文件内容变化（自指事件残留等），是纯空转：
    // 日志照打会把 debug 日志刷爆（实测 50 Hz 空转两天写满 2.5 GB）。
    // 抑制为「每 30s 汇总一次」—— 空转次数和耗时仍然可观测，不再刷屏。
    let index_noop =
        matches!(&index_result, Ok(r) if r.new_blobs.is_empty() && r.removed_blobs.is_empty());

    match &index_result {
        Ok(result) => {
            if index_noop {
                let count = coord.idle_noop_count.fetch_add(1, Ordering::SeqCst) + 1;
                let mut last = coord.last_idle_report.lock();
                if last.as_ref().is_none_or(|t| t.elapsed() >= Duration::from_secs(30)) {
                    *last = Some(Instant::now());
                    coord.idle_noop_count.store(0, Ordering::SeqCst);
                    drop(last);
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[Context] incremental reindex: 0 changes (no-op ×{}) — suppressing for 30s (last {:?})",
                        count,
                        started_at.elapsed()
                    ));
                }
            } else {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[Context] incremental reindex: {} changed, {} removed, {} total ({:?}) in {:?}",
                    result.new_blobs.len(),
                    result.removed_blobs.len(),
                    result.total_files,
                    dirty.first(),
                    started_at.elapsed()
                ));
            }
        }
        Err(err) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Context] incremental reindex failed: {}",
                err
            ));
        }
    }

    // 2. 语义搜索引擎增量更新（有缓存管理器时）
    //
    // 锁纪律：先把 `Arc` clone 出来、**让 guard 落出作用域**，再调
    // `update_engine_in_cache`。那条路径内部要回调 `engine_meta` /
    // `patch_engine`，两者取的是同一把锁 —— 攥着 guard 去调就是自死锁。
    let cache = coord.engine_cache.lock().clone();
    if let Some(cache) = cache {
        match &index_result {
            Ok(result) => {
                match crate::core::tools::codebase_search::update_engine_in_cache(
                    root, &cache, result, None,
                ) {
                    Ok(outcome) => {
                        // `FullRebuild` 是**成功**，不是失败 —— 变化太大时
                        // 逐条补不如重建，这是预期路径。误报成失败会让
                        // resolve_engine 平白禁用 Warming 快路径。
                        let detail = match &outcome {
                            crate::core::tools::codebase_search::UpdateOutcome::Patched {
                                changed,
                            } => format!("patched {} ops", changed),
                            crate::core::tools::codebase_search::UpdateOutcome::FullRebuild {
                                files,
                            } => format!("full rebuild, {} files", files),
                        };
                        // 索引 0 变更 + 补丁 0 ops：纯空转，不复打点（同上面
                        // reindex 的 30s 汇总口径）—— 这一条本身就是 2.5 GB
                        // 日志里最大的单一来源（1,159 万行 / 1.1 GB）。
                        // FullRebuild 和真实补丁照常打点，可观测性不受影响。
                        let boring = index_noop
                            && matches!(
                                &outcome,
                                crate::core::tools::codebase_search::UpdateOutcome::Patched {
                                    changed,
                                } if *changed == 0
                            );
                        if !boring {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[Context] semantic engine updated in background: {} in {:?}",
                                detail,
                                started_at.elapsed()
                            ));
                        }
                        coord.last_rebuild_failed.store(false, Ordering::SeqCst);
                    }
                    Err(err) => {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[Context] background engine update failed: {}",
                            err
                        ));
                        coord.last_rebuild_failed.store(true, Ordering::SeqCst);
                    }
                }
            }
            Err(err) => {
                // 文件索引都没跑成功，拿不到权威变更集 —— 不能凭脏集合
                // 去猜引擎要改什么。下一轮事件会重新触发。
                crate::utils::logging::append_debug_log_line(&format!(
                    "[Context] semantic engine update skipped (index failed): {}",
                    err
                ));
            }
        }
    }

    coord.generation.fetch_add(1, Ordering::SeqCst);
    coord.ready.store(true, Ordering::SeqCst);
    coord.rebuilding.store(false, Ordering::SeqCst);
    coord.dirty_signal.notify_one();
}

/// 重建进行中时，把这一轮的脏路径**原样**放回队列。
///
/// 这里以前插 `"\0refresh-all"` 哨兵。增量补丁上线后那是脚枪：哨兵的含义是
/// "无条件全量重建"，而回队的往往只是补丁窗口内到达的普通编辑 —— 用哨兵会把
/// 它们误升级成一次全量。下一轮本来就会从 `index_project()` 重新推导权威变更集，
/// 把实际路径还回去就够了。
///
/// 哨兵只保留给两处真正需要全量的场景：[`request_refresh`]（引擎已被判定过期）
/// 与 root 之外的事件。
fn requeue_dirty_paths(paths: &[String]) {
    let mut inner = coordinator().inner.lock();
    for path in paths {
        inner.dirty.insert(path.clone());
    }
}

/// watcher 事件过滤器：只有"可能进入索引"的路径才配进脏集合。
///
/// 口径与 `utils::file_walk` 完全一致（`.gitignore` + `.starignore` +
/// `~/.star/ignore` + VCS 目录 + `.star/`），不另起一套 —— 否则"遍历时看不到
/// 的文件"和"能触发重建的文件"会分叉，出现"事件一直在来但索引永远不变"
/// 或反过来的怪状。
struct EventFilter {
    matcher: ignore::gitignore::Gitignore,
}

impl EventFilter {
    /// 该路径是否被忽略（即：不该触发重建）。
    fn is_ignored(&self, path: &Path) -> bool {
        self.matcher
            .matched_path_or_any_parents(path, false)
            .is_ignore()
    }
}

fn build_event_filter(root: &Path) -> EventFilter {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    // 项目 .gitignore。`add()` 对不存在的文件是 no-op，所以无需先判断存在性。
    let _ = builder.add(root.join(".gitignore"));
    // 本项目扩展的忽略文件，复用 file_walk 的口径。
    if let Some(project) = crate::utils::file_walk::project_ignore_file(root) {
        let _ = builder.add(project);
    }
    if let Some(global) = crate::utils::file_walk::global_ignore_file() {
        let _ = builder.add(global);
    }
    // VCS 元数据目录永远不进。
    //
    // 注意 `add_line` 走的是**标准 gitignore 语义**（与 `file_walk` 的
    // `OverrideBuilder` 相反）：不带 `!` 的 pattern 是"忽略"，带 `!` 是
    // "取消忽略"。写成 `!.git/` 等于把这些目录放出来 —— 必须是裸 pattern。
    for dir in crate::utils::file_walk::VCS_DIRS {
        let _ = builder.add_line(None, &format!("{dir}/"));
    }
    // agent 自身状态目录：索引它纯属噪声，而且是重建死循环的源头。
    let _ = builder.add_line(None, ".star/");

    EventFilter {
        matcher: builder
            .build()
            .unwrap_or_else(|_| ignore::gitignore::Gitignore::empty()),
    }
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

    /// 造一棵带 `target/` 的小树，模拟真实仓库的目录构成。
    fn repo_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        for path in [
            "src",
            "src/nested",
            "target",
            "target/debug",
            "target/debug/deps",
        ] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        std::fs::write(root.join("target/debug/deps/big.bin"), "").unwrap();
        dir
    }

    /// 本地 watcher —— **不**碰全局协调器。
    ///
    /// 用例只验证注册集合本身，所以直接构造一个 `DirWatches`，绕开
    /// `ensure_started`：那会写进 `OnceLock` 单例，污染同进程里其他用例。
    fn local_watches() -> Arc<Mutex<DirWatches>> {
        let (_tx, _rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
        let watcher = notify::recommended_watcher(move |res| {
            let _ = _tx.send(res);
        })
        .expect("watcher");
        Arc::new(Mutex::new(DirWatches {
            watcher,
            watched: HashSet::new(),
        }))
    }

    /// 回归的核心：被 `.gitignore` 忽略的目录**不得**进 watch 集合。
    ///
    /// 以前这里是 `watch(root, Recursive)`：整棵树的每个目录都注册一遍 ——
    /// 本仓库 `target/` 下有 6,000+ 个，而它们的事件随后会被 `EventFilter`
    /// 丢掉。注册量 6,600 而有用的一百多个，慢盘上就是那 80 多秒。
    #[test]
    fn ignored_directories_never_get_a_watch() {
        let dir = repo_fixture();
        let root = dir.path();
        let watches = local_watches();

        register_indexed_dirs(&watches, root);

        let watched: Vec<String> = watches
            .lock()
            .watched
            .iter()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect();

        // 要监视的
        assert!(
            watched.iter().any(|p| p.ends_with("/src")),
            "源目录必须在监视集合里: {watched:?}"
        );
        assert!(
            watched.iter().any(|p| p.ends_with("/src/nested")),
            "嵌套源目录也必须在: {watched:?}"
        );
        assert!(
            watched
                .iter()
                .any(|p| p == &root.to_string_lossy().replace('\\', "/")),
            "根目录必须自己注册，否则直接建在根下的文件收不到事件: {watched:?}"
        );
        // 不该监视的
        assert!(
            !watched.iter().any(|p| p.contains("/target")),
            "target/ 被 gitignore，Indexer 不看它，也不该给它 watch: {watched:?}"
        );
    }

    /// 新建目录必须补上 watch —— 注册是 NonRecursive 的，没人会替我们递归。
    /// 漏了这一步，「新建文件夹里的文件不触发重建」会很难查。
    #[test]
    fn a_newly_created_directory_gets_watched() {
        let dir = repo_fixture();
        let root = dir.path();
        let watches = local_watches();
        register_indexed_dirs(&watches, root);

        let fresh = root.join("src").join("brand-new");
        std::fs::create_dir_all(fresh.join("deep")).unwrap();

        add_watch_tree(&watches, &fresh);

        let watched = watches.lock();
        assert!(watched.watched.contains(&fresh));
        assert!(
            watched.watched.contains(&fresh.join("deep")),
            "新目录下的子目录也要注册"
        );
    }

    /// 目录删除后必须撤 watch：`fs.inotify.max_user_watches` 按真实 uid 全局
    /// 共享，不撤等于白占你编辑器 / dev server 的预算。
    #[test]
    fn deleting_a_directory_releases_its_watches() {
        let dir = repo_fixture();
        let root = dir.path();
        let watches = local_watches();
        register_indexed_dirs(&watches, root);

        let doomed = root.join("src").join("nested");
        assert!(watches.lock().watched.contains(&doomed));

        watches.lock().remove_subtree(&doomed);

        let watched = watches.lock();
        assert!(!watched.watched.contains(&doomed));
        assert!(
            watched.watched.contains(&root.join("src")),
            "只撤子树，兄弟目录不能被误伤"
        );
    }

    /// 注册失败不能留下「已注册」的占位，否则同名目录重建后永远补不上 watch。
    #[test]
    fn a_failed_registration_does_not_leave_a_phantom_entry() {
        let watches = local_watches();
        let missing = PathBuf::from("/definitely/not/a/real/dir");
        assert!(!watches.lock().add(&missing));
        assert!(
            !watches.lock().watched.contains(&missing),
            "失败必须把占位撤掉"
        );
        // 幂等：同一个路径不会重复注册。
        assert!(!watches.lock().add(&missing));
    }

    /// 涉及全局协调器脏集合的测试共用这一把锁。
    ///
    /// 协调器是 `OnceLock` 单例，`request_refresh` / `mark_files_dirty` /
    /// `requeue_dirty_paths` 写的是同一个 `HashSet`。并行测试里只要两个用例
    /// 同时碰它，一个用例合法插入的哨兵就会落进另一个用例的断言里 ——
    /// 那是测试间的竞争，不是被测代码的 bug。串行化这一组用例即可消除它。
    static DIRTY_SET_TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    #[test]
    fn mark_dirty_starts_coordinator_and_collects_paths() {
        let _guard = DIRTY_SET_TEST_LOCK.lock();
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
        assert!(
            inner.dirty.iter().any(|p| p.ends_with("a.rs")),
            "脏集合应包含标记的文件"
        );
    }

    #[test]
    fn status_text_reflects_state() {
        let s = status_text();
        assert!(s.starts_with("index:"));
    }

    /// 重建进行中回队的必须是**实际路径**，不是哨兵。
    ///
    /// 用哨兵会把补丁窗口内到达的普通编辑误升级成一次全量重建 ——
    /// 这是增量补丁上线后才出现的脚枪。
    #[test]
    fn requeue_keeps_paths_verbatim_and_adds_no_sentinel() {
        // 必须和 `mark_dirty_starts_coordinator_and_collects_paths` 共用同一把锁：
        // 那个用例会调 `request_refresh`（合法地插哨兵）。不串行化的话，
        // 它的哨兵会落进本用例的断言里 —— 那是测试间竞争，不是被测代码的 bug。
        let _guard = DIRTY_SET_TEST_LOCK.lock();
        let coord = coordinator();

        coord.inner.lock().dirty.clear();

        requeue_dirty_paths(&["src/a.rs".to_string(), "src/b.rs".to_string()]);

        let inner = coord.inner.lock();
        assert!(inner.dirty.contains("src/a.rs"));
        assert!(inner.dirty.contains("src/b.rs"));
        assert!(
            !inner.dirty.iter().any(|p| p.starts_with('\0')),
            "回队不得注入哨兵，否则普通编辑会被误升级为全量重建：{:?}",
            inner.dirty
        );
    }

    /// 事件过滤必须复用 `utils::file_walk` 的忽略口径 —— 否则"遍历时看不到
    /// 的文件"和"能触发重建的文件"会分叉。
    #[test]
    fn event_filter_honours_gitignore_and_vcs_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "target/\nsecret.txt\n").unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();

        let filter = build_event_filter(root);

        assert!(filter.is_ignored(&root.join("target/debug/build.rs")));
        assert!(filter.is_ignored(&root.join("secret.txt")));
        assert!(!filter.is_ignored(&root.join("src/main.rs")));
        // `.star/` 是 agent 自身状态目录：索引它会造成重建→写盘→事件→重建 死循环。
        assert!(filter.is_ignored(&root.join(".star/context/index.json")));
        assert!(filter.is_ignored(&root.join(".git/HEAD")));
    }

    /// 没有 `.gitignore` 也必须能建出过滤器（空仓库不该 panic）。
    #[test]
    fn event_filter_without_ignore_files_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let filter = build_event_filter(dir.path());
        assert!(!filter.is_ignored(&dir.path().join("main.rs")));
        assert!(filter.is_ignored(&dir.path().join(".star/logs/agent.log")));
    }
}
