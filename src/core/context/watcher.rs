//! 按规范化根管理索引监听和重建；注册表只登记资源，不持锁执行 I/O。
//! 每根独立 dirty、generation 与 worker 句柄；最多三个监听根和一个重建。
//! worker 退出由守卫公布，panic 不捕获；Indexer 的污染锁错误原样传播。

use crate::core::context::indexer::Indexer;
use crate::core::context::search_cache::shared_search_cache;
use notify::Watcher as _;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, LazyLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(300);
const MAX_ROOTS: usize = 3;
const IDLE_TTL: Duration = Duration::from_secs(300);
const MAX_EVENT_QUEUE: usize = 1024;

static ROOTS: LazyLock<std::sync::Mutex<HashMap<PathBuf, Arc<IndexCoordinator>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
// 包含同步降级路径，所有根共享同一个重建配额；Indexer 仍拥有磁盘 I/O 锁。
static REBUILD_SLOT: LazyLock<std::sync::Mutex<()>> = LazyLock::new(|| std::sync::Mutex::new(()));

struct CoordInner {
    dirty: HashSet<String>,
    last_used: Instant,
    retry_at: Instant,
    failures: u32,
    error: Option<String>,
    worker: Option<JoinHandle<()>>,
}

struct IndexCoordinator {
    inner: Mutex<CoordInner>,
    stop: AtomicBool,
    started: AtomicBool,
    ready: AtomicBool,
    rebuilding: AtomicBool,
    failed: AtomicBool,
    generation: AtomicU64,
    dirty_epoch: AtomicU64,
}

impl IndexCoordinator {
    fn new() -> Self {
        Self {
            inner: Mutex::new(CoordInner {
                dirty: HashSet::new(),
                last_used: Instant::now(),
                retry_at: Instant::now(),
                failures: 0,
                error: None,
                worker: None,
            }),
            stop: AtomicBool::new(false),
            started: AtomicBool::new(true),
            ready: AtomicBool::new(false),
            rebuilding: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            dirty_epoch: AtomicU64::new(0),
        }
    }
}

type IndexError = Box<dyn std::error::Error + Send + Sync>;

/// 现存根解析符号链接；已删除根仍按绝对、词法规范化路径匹配注销记录。
pub fn normalized_root(root: &Path) -> PathBuf {
    if let Ok(root) = root.canonicalize() {
        return root;
    }
    let absolute = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .expect("current directory is required for relative index roots")
            .join(root)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// 将索引工作线程降为后台优先级。
pub fn lower_current_thread_priority() {
    let _ = thread_priority::set_current_thread_priority(thread_priority::ThreadPriority::Min);
}

fn lookup(root: &Path) -> Result<Option<Arc<IndexCoordinator>>, IndexError> {
    let roots = ROOTS
        .lock()
        .map_err(|_| std::io::Error::other("index registry lock poisoned"))?;
    Ok(roots.get(&normalized_root(root)).cloned())
}

fn record_failure(coord: &IndexCoordinator, error: String) {
    coord.failed.store(true, Ordering::SeqCst);
    coord.ready.store(false, Ordering::SeqCst);
    let mut inner = coord.inner.lock();
    inner.failures = inner.failures.saturating_add(1);
    inner.retry_at = Instant::now() + Duration::from_secs(1_u64 << inner.failures.min(6));
    inner.error = Some(error.clone());
    crate::utils::logging::append_debug_log_line(&format!(
        "[Context] index worker failed: {error}"
    ));
}

/// 不捕获 unwind；守卫只发布退出事实，不重用 panic 栈上的资源。
struct WorkerExit(Arc<IndexCoordinator>);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        if !self.0.stop.load(Ordering::SeqCst) && !self.0.failed.load(Ordering::SeqCst) {
            record_failure(&self.0, "index worker exited unexpectedly".into());
        }
        self.0.rebuilding.store(false, Ordering::SeqCst);
        self.0.started.store(false, Ordering::SeqCst);
    }
}

fn retire(coord: &IndexCoordinator) {
    coord.stop.store(true, Ordering::SeqCst);
    coord.ready.store(false, Ordering::SeqCst);
}

/// 按 root 幂等启动。容量不足返回 false，由查询方走受控同步降级。
/// 注册和启动占位在同一个锁边界内；目录扫描和 worker 等待都在锁外。
pub fn ensure_started(project_root: &Path) -> Result<bool, IndexError> {
    let root = normalized_root(project_root);
    if !root.is_dir() {
        return Ok(false);
    }
    let coord = {
        let mut roots = ROOTS
            .lock()
            .map_err(|_| std::io::Error::other("index registry lock poisoned"))?;
        // 退出中的线程仍计入容量，不能靠快速注销/启动突破资源上限。
        roots.retain(|path, coord| {
            let inner = coord.inner.lock();
            if !path.is_dir() || inner.last_used.elapsed() > IDLE_TTL {
                retire(coord);
            }
            let finished = inner.worker.as_ref().is_some_and(JoinHandle::is_finished);
            !(coord.stop.load(Ordering::SeqCst) && finished)
        });
        if let Some(coord) = roots.get(&root) {
            let mut inner = coord.inner.lock();
            inner.last_used = Instant::now();
            if coord.started.load(Ordering::SeqCst) {
                return Ok(!coord.stop.load(Ordering::SeqCst));
            }
            if inner
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
                || Instant::now() < inner.retry_at
            {
                return Ok(false);
            }
            // 旧句柄已结束，创建完整的新协调器、监听器和队列，而非复用 panic 状态。
        } else if roots.len() >= MAX_ROOTS {
            return Ok(false);
        }
        let coord = Arc::new(IndexCoordinator::new());
        roots.insert(root.clone(), coord.clone());
        coord
    };
    let worker_coord = coord.clone();
    let worker_root = root.clone();
    let worker = std::thread::Builder::new()
        .name("star-index-worker".into())
        .spawn(move || {
            let _exit = WorkerExit(worker_coord.clone());
            if let Err(error) = run_worker(&worker_root, &worker_coord) {
                record_failure(&worker_coord, error.to_string());
            }
        });
    match worker {
        Ok(worker) => {
            coord.inner.lock().worker = Some(worker);
            Ok(true)
        }
        Err(error) => {
            record_failure(&coord, error.to_string());
            coord.started.store(false, Ordering::SeqCst);
            Err(error.into())
        }
    }
}

/// 注销只发停止信号，不在调用线程等待重建；worker 独占并释放监听器。
pub fn unregister_root(root: &Path) -> Result<(), IndexError> {
    let root = normalized_root(root);
    if let Some(coord) = lookup(&root)? {
        retire(&coord);
    }
    shared_search_cache().remove_root(&root);
    Ok(())
}

/// 查询只对仍有健康 worker 的根使用 stale/warming，失败原因不会被隐藏。
pub fn is_healthy(root: &Path) -> Result<bool, IndexError> {
    let Some(coord) = lookup(root)? else {
        return Ok(false);
    };
    let mut inner = coord.inner.lock();
    inner.last_used = Instant::now();
    Ok(coord.started.load(Ordering::SeqCst)
        && !coord.failed.load(Ordering::SeqCst)
        && !coord.stop.load(Ordering::SeqCst)
        && !inner.worker.as_ref().is_some_and(JoinHandle::is_finished))
}

/// 文件变更显式归属指定根，不从进程 cwd 猜测。
pub fn mark_files_dirty(
    root: &Path,
    paths: impl IntoIterator<Item = String>,
) -> Result<(), IndexError> {
    let root = normalized_root(root);
    shared_search_cache().invalidate_root(&root);
    let _ = ensure_started(&root)?;
    if let Some(coord) = lookup(&root)? {
        let mut inner = coord.inner.lock();
        inner.dirty.extend(paths);
        coord.dirty_epoch.fetch_add(1, Ordering::SeqCst);
    }
    crate::core::context::call_graph_db::invalidate(&root);
    Ok(())
}

/// 为指定根排刷新；不清空失败状态，也不把失败重试无限推迟。
pub fn request_refresh(root: &Path) -> Result<bool, IndexError> {
    if !ensure_started(root)? {
        return Ok(false);
    }
    if let Some(coord) = lookup(root)? {
        coord.inner.lock().dirty.insert("\0refresh-all".into());
    }
    is_healthy(root)
}

/// 同步降级和后台重建共享有界执行配额；污染后明确报错，不恢复锁。
pub(crate) fn with_rebuild_slot<T>(
    work: impl FnOnce() -> Result<T, IndexError>,
) -> Result<T, IndexError> {
    let _slot = REBUILD_SLOT
        .lock()
        .map_err(|_| std::io::Error::other("index rebuild slot lock poisoned"))?;
    work()
}

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

fn run_worker(root: &Path, coord: &Arc<IndexCoordinator>) -> Result<(), IndexError> {
    lower_current_thread_priority();
    let _ = crate::utils::git_self_exclude::ensure_star_excluded(root);
    let (tx, rx) = mpsc::sync_channel(MAX_EVENT_QUEUE);
    let overflow = Arc::new(AtomicBool::new(false));
    let callback_overflow = overflow.clone();
    let watcher = notify::recommended_watcher(move |event| {
        if tx.try_send(event).is_err() {
            callback_overflow.store(true, Ordering::SeqCst);
        }
    })?;
    let watches = Arc::new(Mutex::new(DirWatches {
        watcher,
        watched: HashSet::new(),
    }));
    register_indexed_dirs(&watches, root);
    if !watches.lock().watched.contains(root) {
        return Err(std::io::Error::other("index root watch registration failed").into());
    }
    let filter = build_event_filter(root);
    let mut pending_since = None;
    loop {
        if coord.stop.load(Ordering::SeqCst) || !root.is_dir() {
            retire(coord);
            shared_search_cache().remove_root(root);
            return Ok(());
        }
        if coord.inner.lock().last_used.elapsed() > IDLE_TTL {
            retire(coord);
            return Ok(());
        }
        collect_events(root, coord, &rx, &watches, &filter)?;
        if overflow.swap(false, Ordering::SeqCst) {
            shared_search_cache().invalidate_root(root);
            let mut inner = coord.inner.lock();
            inner.dirty.insert("\0refresh-all".into());
            coord.dirty_epoch.fetch_add(1, Ordering::SeqCst);
        }
        let pending = !coord.inner.lock().dirty.is_empty();
        if pending && pending_since.is_none() {
            pending_since = Some(Instant::now());
        }
        if pending_since.is_some_and(|since| since.elapsed() >= DEBOUNCE) {
            let can_retry = Instant::now() >= coord.inner.lock().retry_at;
            if can_retry {
                let dirty = std::mem::take(&mut coord.inner.lock().dirty);
                let epoch = coord.dirty_epoch.load(Ordering::SeqCst);
                coord.rebuilding.store(true, Ordering::SeqCst);
                let result = with_rebuild_slot(|| {
                    if coord.stop.load(Ordering::SeqCst) {
                        return Ok(false);
                    }
                    let index = Indexer::new(root).index_project()?;
                    if index.truncated {
                        return Ok(false);
                    }
                    crate::core::tools::codebase_search::update_engine_in_cache(
                        root,
                        &shared_search_cache(),
                        &index,
                        None,
                    )?;
                    Ok(true)
                });
                coord.rebuilding.store(false, Ordering::SeqCst);
                match result {
                    Ok(true) => {
                        let mut inner = coord.inner.lock();
                        inner.failures = 0;
                        inner.error = None;
                        inner.retry_at = Instant::now();
                        coord.failed.store(false, Ordering::SeqCst);
                        coord.ready.store(true, Ordering::SeqCst);
                        coord.generation.fetch_add(1, Ordering::SeqCst);
                        if coord.dirty_epoch.load(Ordering::SeqCst) != epoch {
                            shared_search_cache().invalidate_root(root);
                        }
                    }
                    Ok(false) => {
                        coord.inner.lock().dirty.extend(dirty);
                    }
                    Err(error) => {
                        shared_search_cache().invalidate_root(root);
                        record_failure(coord, error.to_string());
                        coord.inner.lock().dirty.extend(dirty);
                    }
                }
                pending_since = None;
            }
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

fn collect_events(
    root: &Path,
    coord: &IndexCoordinator,
    rx: &Receiver<notify::Result<notify::Event>>,
    watches: &Arc<Mutex<DirWatches>>,
    filter: &EventFilter,
) -> Result<(), IndexError> {
    // 每轮有界排空，持续事件风暴也不能饿死停止检查与重建。
    for _ in 0..MAX_EVENT_QUEUE {
        let event = match rx.try_recv() {
            Ok(event) => event?,
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(std::io::Error::other("index event channel closed").into())
            }
        };
        if !matches!(
            event.kind,
            notify::EventKind::Create(_)
                | notify::EventKind::Modify(_)
                | notify::EventKind::Remove(_)
        ) {
            continue;
        }
        let mut dirty = Vec::new();
        for path in event.paths {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if relative.as_os_str().is_empty() || filter.is_ignored(&path) {
                continue;
            }
            if matches!(event.kind, notify::EventKind::Create(_)) && path.is_dir() {
                add_watch_tree(watches, &path);
            }
            if matches!(event.kind, notify::EventKind::Remove(_)) {
                watches.lock().remove_subtree(&path);
            }
            dirty.push(relative.to_string_lossy().replace('\\', "/"));
        }
        if !dirty.is_empty() {
            shared_search_cache().invalidate_root(root);
            crate::core::context::call_graph_db::invalidate(root);
            coord.inner.lock().dirty.extend(dirty);
            coord.dirty_epoch.fetch_add(1, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// 指定根的可观察状态，包含退出、失败、重建及代数。
pub fn status_text(root: &Path) -> Result<String, IndexError> {
    let Some(coord) = lookup(root)? else {
        return Ok("index: off".into());
    };
    let inner = coord.inner.lock();
    let state = if coord.failed.load(Ordering::SeqCst) {
        "failed"
    } else if !coord.started.load(Ordering::SeqCst) {
        "exited"
    } else if coord.stop.load(Ordering::SeqCst) {
        "stopping"
    } else if coord.rebuilding.load(Ordering::SeqCst) {
        "rebuilding"
    } else if coord.ready.load(Ordering::SeqCst) {
        "ready"
    } else {
        "warming"
    };
    Ok(format!(
        "index: {state} (gen {}, {} dirty){}",
        coord.generation.load(Ordering::SeqCst),
        inner.dirty.len(),
        inner
            .error
            .as_ref()
            .map(|e| format!(" {e}"))
            .unwrap_or_default()
    ))
}

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
    fn root_normalization_collapses_parent_segments_and_resolves_existing_aliases() {
        let dir = tempfile::tempdir().expect("fixture");
        let nested = dir.path().join("src").join("..");
        assert_eq!(normalized_root(&nested), normalized_root(dir.path()));
        #[cfg(unix)]
        {
            let alias = dir.path().join("alias");
            std::os::unix::fs::symlink(dir.path(), &alias).expect("symlink fixture");
            assert_eq!(normalized_root(&alias), normalized_root(dir.path()));
        }
    }

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
