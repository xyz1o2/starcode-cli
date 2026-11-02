//! CallGraph 持久化（SQLite）。
//!
//! 设计文档：`docs/callgraph-persistence-design.zh-CN.md`。
//!
//! 库只是 `Vec<FileSymbols>` 的物化缓存，不是新的查询引擎：
//! - 写入：按文件哈希增量提取（复用 Indexer 的 size/mtime + sha256 机制）；
//! - 读取：从库组装 `FileSymbols` → 原样走 [`CallGraph::build`]。
//!
//! 因此查询 API 与输出格式和内存版**完全一致**。`SymbolId` 从不入库：
//! 它是全局自增计数器，只在单次加载内有效，每次按 `rel_path` 排序重新分配。

use super::call_graph::CallGraph;
use super::symbol::{self, CallEdge, FileSymbols, SymbolDef, SymbolId, SymbolKind};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 库结构版本。schema 或任何影响读结果的语义变化都必须 bump；
/// 版本不匹配时整库丢弃重建（复刻 `Indexer::load_index` 的版本丢弃逻辑）。
pub const CALLGRAPH_SCHEMA_VERSION: u32 = 1;

/// 每批变更文件的提交粒度：初次全量在大仓上避免数千次 fsync。
const SYNC_BATCH_FILES: usize = 200;

/// `sync` 串行化：watcher 批量打脏时不并发写库。
static SYNC_LOCK: Mutex<()> = Mutex::new(());

/// 各 root 上次成功 sync 时的 `index.json` mtime token。
///
/// 与 CodebaseSearch 引擎缓存使用同一个冷热信号（设计文档 §7）：token 变了
/// 才值得重新 walk + stat；没变时查询直接读库。
static LAST_SYNC_TOKEN: OnceLock<Mutex<HashMap<PathBuf, Option<SystemTime>>>> = OnceLock::new();

fn last_sync_tokens() -> &'static Mutex<HashMap<PathBuf, Option<SystemTime>>> {
    LAST_SYNC_TOKEN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 库文件位置：`.star/context/callgraph.db`（与 `index.json` 同目录）。
pub fn db_path(root: &Path) -> PathBuf {
    root.join(".star").join("context").join("callgraph.db")
}

/// 一次同步的可观测结果（日志与测试共用）。
#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub files_total: usize,
    pub fast_path_skips: usize,
    pub reparsed: usize,
    pub removed: usize,
    pub elapsed_ms: u128,
}

// ── 查询入口 ──────────────────────────────────────────────────────────────────

/// 查询入口：优先从库加载；需要时先增量同步；库彻底不可用时退回内存全量构建。
///
/// `trace_call_chain` 用它替换 `build_call_graph` —— 工具 schema 与输出不变。
pub fn get_call_graph(root: &Path) -> Result<CallGraph, Box<dyn Error + Send + Sync>> {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let token = index_token(&canonical);

    let token_changed = {
        let tokens = last_sync_tokens()
            .lock()
            .expect("callgraph token lock poisoned");
        tokens.get(&canonical).copied() != Some(token)
    };

    if !token_changed {
        if let Ok(Some(graph)) = load_graph(&canonical) {
            return Ok(graph);
        }
        // 库被删或已损坏：落到下面的同步路径重建。
    }

    match sync(&canonical) {
        Ok(report) => {
            log::info!(
                "CallGraph index synced in {}ms ({} files, {} re-parsed, {} removed, {} fast-path)",
                report.elapsed_ms,
                report.files_total,
                report.reparsed,
                report.removed,
                report.fast_path_skips
            );
            last_sync_tokens()
                .lock()
                .expect("callgraph token lock poisoned")
                .insert(canonical.clone(), token);
            if let Ok(Some(graph)) = load_graph(&canonical) {
                return Ok(graph);
            }
        }
        Err(err) => {
            log::warn!(
                "CallGraph persistence unavailable for {}: {err}",
                canonical.display()
            );
        }
    }

    // 终极兜底：磁盘只读、库被占用等情况下，查询绝不能失败。
    crate::core::tools::codebase_search::build_call_graph(root)
}

/// 让某个项目根的缓存 token 失效：下一次 [`get_call_graph`] 会重新走 sync。
///
/// watcher 观察到真实文件变更（或 agent 自己写完文件）时调用。**只清 token，
/// 不做任何 I/O** —— 同步推迟到真正的查询，避免没人用 CodeGraph 时白跑。
///
/// 只对"真实路径变更"调用，不要对 `\0refresh-all` 哨兵调用：哨兵可能由
/// 周期性 refiller 插入，清 token 会让每次查询都重新同步，等于禁用缓存。
pub fn invalidate(root: &Path) {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    last_sync_tokens()
        .lock()
        .expect("callgraph token lock poisoned")
        .remove(&canonical);
}

// ── 同步（写入） ──────────────────────────────────────────────────────────────

/// 全量/增量同步。库损坏时隔离旧文件并重建一次。
pub fn sync(root: &Path) -> Result<SyncReport, Box<dyn Error + Send + Sync>> {
    let _guard = SYNC_LOCK.lock().expect("callgraph sync lock poisoned");
    let started = Instant::now();

    let db = db_path(root);
    if let Some(parent) = db.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut report = match sync_once(root, &db) {
        Ok(report) => report,
        // 只有**确认损坏**才隔离重建。另一个进程正在写库时拿到的是
        // SQLITE_BUSY（有 busy_timeout 兜底），那是可重试的争用，不是数据损坏；
        // 把库改名会把它从对方脚下抽走。
        Err(first) if db_looks_corrupt(&db) => {
            log::warn!(
                "CallGraph db at {} is corrupt ({first}); quarantining and rebuilding",
                db.display()
            );
            quarantine_db(&db);
            sync_once(root, &db)?
        }
        Err(err) => return Err(err),
    };
    report.elapsed_ms = started.elapsed().as_millis();
    Ok(report)
}

/// 文件级变更。`Replace` 是一份完整的 `FileSymbols`（提取时 id 从 0 起，
/// 因此 `caller_id` 直接就是文件内局部下标）。
enum StagedChange {
    Replace {
        rel: String,
        size: u64,
        mtime_ms: u64,
        hash: String,
        file_symbols: FileSymbols,
    },
    /// 哈希未变、只是 size/mtime 元数据变了（touch / 重新 checkout）。
    Touch {
        rel: String,
        size: u64,
        mtime_ms: u64,
    },
    Remove {
        rel: String,
    },
}

fn sync_once(root: &Path, db: &Path) -> Result<SyncReport, Box<dyn Error + Send + Sync>> {
    let mut conn = Connection::open(db)?;
    configure_writer(&conn)?;

    let previous = ensure_schema(&mut conn, root)?;
    let mut report = SyncReport::default();

    // 1. 收集支持语言的文件，统一按 rel_path 排序 —— 符号 id 分配必须确定，
    //    不能受目录遍历顺序（或并行遍历完成顺序）影响。
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in crate::utils::file_walk::walk(root, &walk_options()) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        if symbol::language_for_file(ext).is_none() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        files.push((rel, path.to_path_buf()));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);

    report.files_total = files.len();

    let mut staged: Vec<StagedChange> = Vec::new();
    let mut seen: HashSet<String> = HashSet::with_capacity(files.len());

    for (rel, path) in &files {
        seen.insert(rel.clone());

        let meta = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let size = meta.len();
        let mtime_ms = mtime_millis(&meta);

        // 快路径：size/mtime 都未变 → 零读取、零哈希（同 Indexer::is_unchanged_fast）。
        if let Some(prev) = previous.get(rel) {
            if prev.size == size && prev.mtime_ms == mtime_ms && mtime_ms != 0 {
                report.fast_path_skips += 1;
                continue;
            }
        }

        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            // 与 build_call_graph 一致：读不了（二进制/权限）就不贡献符号。
            Err(_) => continue,
        };
        let hash = crate::core::utils::paths::sha256_hex(content.as_bytes());

        if let Some(prev) = previous.get(rel) {
            if prev.content_hash == hash {
                staged.push(StagedChange::Touch {
                    rel: rel.clone(),
                    size,
                    mtime_ms,
                });
                continue;
            }
        }

        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let Some(language) = symbol::language_for_file(ext) else {
            continue;
        };

        // 传 0 起点：提取器产出的 `caller_id` 天然就是文件内局部下标，
        // 入库时不需要再减基址（设计文档 §6）。内存版透传全局计数器只影响
        // `SymbolId` 的取值，不影响任何查询结果。
        let mut local_base: SymbolId = 0;
        match symbol::extract_symbols(&content, rel, ext, &language, &mut local_base) {
            Some(file_symbols) => {
                staged.push(StagedChange::Replace {
                    rel: rel.clone(),
                    size,
                    mtime_ms,
                    hash,
                    file_symbols,
                });
                report.reparsed += 1;
            }
            None => {
                // 文件曾是支持语言但现在解析失败：旧符号已不可信，移除。
                staged.push(StagedChange::Remove { rel: rel.clone() });
            }
        }

        if staged.len() >= SYNC_BATCH_FILES {
            flush(&mut conn, &mut staged)?;
        }
    }

    // 2. 磁盘上已不存在的文件 → 删除（外键级联清 symbols/call_edges）。
    for rel in previous.keys() {
        if !seen.contains(rel) {
            staged.push(StagedChange::Remove { rel: rel.clone() });
            report.removed += 1;
        }
    }
    flush(&mut conn, &mut staged)?;

    Ok(report)
}

fn flush(
    conn: &mut Connection,
    staged: &mut Vec<StagedChange>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if staged.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    let now = now_millis();

    for change in staged.drain(..) {
        match change {
            StagedChange::Replace {
                rel,
                size,
                mtime_ms,
                hash,
                file_symbols,
            } => {
                // 先 upsert 父行（子表外键指向它），再替换子行。
                tx.execute(
                    "INSERT INTO files
                       (rel_path, content_hash, size, mtime_ms, symbol_count, edge_count, indexed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(rel_path) DO UPDATE SET
                       content_hash = excluded.content_hash,
                       size = excluded.size,
                       mtime_ms = excluded.mtime_ms,
                       symbol_count = excluded.symbol_count,
                       edge_count = excluded.edge_count,
                       indexed_at = excluded.indexed_at",
                    params![
                        rel,
                        hash,
                        size as i64,
                        mtime_ms as i64,
                        file_symbols.symbols.len() as i64,
                        file_symbols.edges.len() as i64,
                        now as i64
                    ],
                )?;
                tx.execute("DELETE FROM symbols WHERE file_rel_path = ?1", params![rel])?;
                tx.execute(
                    "DELETE FROM call_edges WHERE file_rel_path = ?1",
                    params![rel],
                )?;

                for sym in &file_symbols.symbols {
                    tx.execute(
                        "INSERT INTO symbols
                           (file_rel_path, local_idx, name, kind, start_line, end_line, signature)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            rel,
                            sym.id as i64,
                            sym.name,
                            sym.kind.as_str(),
                            sym.start_line as i64,
                            sym.end_line as i64,
                            sym.signature
                        ],
                    )?;
                }
                for edge in &file_symbols.edges {
                    tx.execute(
                        "INSERT INTO call_edges
                           (file_rel_path, caller_local_idx, callee_name, line)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            rel,
                            edge.caller_id as i64,
                            edge.callee_name,
                            edge.line as i64
                        ],
                    )?;
                }
            }
            StagedChange::Touch {
                rel,
                size,
                mtime_ms,
            } => {
                tx.execute(
                    "UPDATE files SET size = ?1, mtime_ms = ?2, indexed_at = ?3 WHERE rel_path = ?4",
                    params![size as i64, mtime_ms as i64, now as i64, rel],
                )?;
            }
            StagedChange::Remove { rel } => {
                // 子表有 ON DELETE CASCADE（PRAGMA foreign_keys = ON 已开）。
                tx.execute("DELETE FROM files WHERE rel_path = ?1", params![rel])?;
            }
        }
    }

    tx.commit()?;
    Ok(())
}

// ── 读取 ──────────────────────────────────────────────────────────────────────

/// 从库组装调用图。
///
/// 库不存在 / 版本不符 → `Ok(None)`；文件损坏 → `Err`。两种都由
/// [`get_call_graph`] 决定是重建还是内存兜底（设计文档 §8）。
pub fn load_graph(root: &Path) -> Result<Option<CallGraph>, Box<dyn Error + Send + Sync>> {
    let root_str = normalized_root(root);
    let db = db_path(root);
    if !db.exists() {
        return Ok(None);
    }

    let conn = Connection::open(&db)?;
    // 读也可能与写者短暂争用（非 WAL 的旧库或 checkpoint 期间）。
    conn.busy_timeout(Duration::from_secs(5))?;
    if !table_exists(&conn, "files")? {
        return Ok(None);
    }

    let expected_version = CALLGRAPH_SCHEMA_VERSION.to_string();
    if read_meta(&conn, "schema_version")?.as_deref() != Some(expected_version.as_str()) {
        return Ok(None);
    }
    // 库被拷到别的仓库时，缓存内容与路径口径都不再可信。
    if read_meta(&conn, "root")?.as_deref() != Some(root_str.as_str()) {
        return Ok(None);
    }

    // 一次拉全表再按文件分组：避免"每文件两条查询"的 N+1。
    let mut grouped: HashMap<String, RawFile> = HashMap::new();

    {
        let mut stmt = conn.prepare(
            "SELECT file_rel_path, local_idx, name, kind, start_line, end_line, signature
             FROM symbols ORDER BY file_rel_path, local_idx",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                RawSymbol {
                    local_idx: row.get::<_, i64>(1)? as usize,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    start_line: row.get::<_, i64>(4)? as usize,
                    end_line: row.get::<_, i64>(5)? as usize,
                    signature: row.get(6)?,
                },
            ))
        })?;
        for row in rows {
            let (rel, sym) = row?;
            grouped.entry(rel).or_default().symbols.push(sym);
        }
    }

    {
        let mut stmt = conn.prepare(
            "SELECT file_rel_path, caller_local_idx, callee_name, line
             FROM call_edges ORDER BY file_rel_path, rowid",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                RawEdge {
                    caller_local_idx: row.get::<_, i64>(1)? as usize,
                    callee_name: row.get(2)?,
                    line: row.get::<_, i64>(3)? as usize,
                },
            ))
        })?;
        for row in rows {
            let (rel, edge) = row?;
            grouped.entry(rel).or_default().edges.push(edge);
        }
    }

    // 按 rel_path 排序组装 —— 与 sync 的写入顺序一致，符号 id 可复现。
    let mut stmt = conn.prepare("SELECT rel_path FROM files ORDER BY rel_path")?;
    let rel_paths: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;

    let mut next_id: SymbolId = 0;
    let mut file_symbols: Vec<FileSymbols> = Vec::with_capacity(rel_paths.len());

    for rel in rel_paths {
        let raw = grouped.remove(&rel).unwrap_or_default();
        let mut local_to_global: HashMap<usize, SymbolId> =
            HashMap::with_capacity(raw.symbols.len());
        let mut symbols = Vec::with_capacity(raw.symbols.len());

        for sym in raw.symbols {
            let id = next_id;
            next_id += 1;
            local_to_global.insert(sym.local_idx, id);
            symbols.push(SymbolDef {
                id,
                name: sym.name,
                kind: parse_kind(&sym.kind),
                file_path: rel.clone(),
                start_line: sym.start_line,
                end_line: sym.end_line,
                signature: sym.signature,
            });
        }

        let edges = raw
            .edges
            .into_iter()
            .filter_map(|edge| {
                // 悬空 caller（理论上不会出现）直接丢弃，不 panic。
                let caller_id = *local_to_global.get(&edge.caller_local_idx)?;
                Some(CallEdge {
                    caller_id,
                    callee_name: edge.callee_name,
                    callee_id: None,
                    file_path: rel.clone(),
                    line: edge.line,
                })
            })
            .collect();

        // 空符号文件也要 push：与 `build_call_graph` 的输入空间保持一致。
        file_symbols.push(FileSymbols {
            file_path: rel,
            symbols,
            edges,
        });
    }

    Ok(Some(CallGraph::build(&file_symbols)))
}

#[derive(Default)]
struct RawFile {
    symbols: Vec<RawSymbol>,
    edges: Vec<RawEdge>,
}

struct RawSymbol {
    local_idx: usize,
    name: String,
    kind: String,
    start_line: usize,
    end_line: usize,
    signature: String,
}

struct RawEdge {
    caller_local_idx: usize,
    callee_name: String,
    line: usize,
}

fn parse_kind(kind: &str) -> SymbolKind {
    match kind {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "trait" => SymbolKind::Trait,
        "interface" => SymbolKind::Interface,
        "enum" => SymbolKind::Enum,
        "module" => SymbolKind::Module,
        // 未知词表（旧库或将来新增 kind）一律降级，绝不让查询炸掉。
        _ => SymbolKind::Unknown,
    }
}

// ── Schema 与连接 ─────────────────────────────────────────────────────────────

struct PreviousFile {
    content_hash: String,
    size: u64,
    mtime_ms: u64,
}

fn configure_writer(conn: &Connection) -> Result<(), Box<dyn Error + Send + Sync>> {
    // 多进程共用一个项目时，写锁是正常现象：等一会儿而不是立刻失败。
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}

/// 只有"这个文件不是 SQLite 库"才算损坏。
///
/// 打不开（权限、路径）或查询因锁失败都返回 `false`：那应当走内存兜底，
/// 而不是把库改名重建。
fn db_looks_corrupt(db: &Path) -> bool {
    if !db.exists() {
        return false;
    }
    let Ok(conn) = Connection::open(db) else {
        return false;
    };
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(_) => false,
        Err(rusqlite::Error::SqliteFailure(err, _)) => matches!(
            err.code,
            rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
        ),
        Err(_) => false,
    }
}

/// 校验版本/root 并返回已有的 per-file 哈希登记。首次建库或版本不符时
/// 重建 schema 并返回空表。
fn ensure_schema(
    conn: &mut Connection,
    root: &Path,
) -> Result<HashMap<String, PreviousFile>, Box<dyn Error + Send + Sync>> {
    let root_str = normalized_root(root);

    if !table_exists(conn, "files")? {
        create_schema(conn, &root_str)?;
        return Ok(HashMap::new());
    }

    let version = read_meta(conn, "schema_version")?;
    let expected = CALLGRAPH_SCHEMA_VERSION.to_string();
    let root_matches = read_meta(conn, "root")?.as_deref() == Some(root_str.as_str());

    if version.as_deref() != Some(expected.as_str()) || !root_matches {
        log::info!(
            "CallGraph db schema/root mismatch (version {:?}, root {:?}); rebuilding",
            version,
            root_str
        );
        drop_schema(conn)?;
        create_schema(conn, &root_str)?;
        return Ok(HashMap::new());
    }

    let mut stmt = conn.prepare("SELECT rel_path, content_hash, size, mtime_ms FROM files")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            PreviousFile {
                content_hash: row.get(1)?,
                size: row.get::<_, i64>(2)? as u64,
                mtime_ms: row.get::<_, i64>(3)? as u64,
            },
        ))
    })?;

    let mut previous = HashMap::new();
    for row in rows {
        let (rel, file) = row?;
        previous.insert(rel, file);
    }
    Ok(previous)
}

fn create_schema(conn: &Connection, root: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
           key   TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS files (
           rel_path     TEXT PRIMARY KEY,
           content_hash TEXT NOT NULL,
           size         INTEGER NOT NULL,
           mtime_ms     INTEGER NOT NULL,
           symbol_count INTEGER NOT NULL DEFAULT 0,
           edge_count   INTEGER NOT NULL DEFAULT 0,
           indexed_at   INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS symbols (
           file_rel_path TEXT NOT NULL REFERENCES files(rel_path) ON DELETE CASCADE,
           local_idx     INTEGER NOT NULL,
           name          TEXT NOT NULL,
           kind          TEXT NOT NULL,
           start_line    INTEGER NOT NULL,
           end_line      INTEGER NOT NULL,
           signature     TEXT NOT NULL,
           PRIMARY KEY (file_rel_path, local_idx)
         );
         CREATE TABLE IF NOT EXISTS call_edges (
           file_rel_path    TEXT NOT NULL REFERENCES files(rel_path) ON DELETE CASCADE,
           caller_local_idx INTEGER NOT NULL,
           callee_name      TEXT NOT NULL,
           line             INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_call_edges_lookup
           ON call_edges(file_rel_path, caller_local_idx);",
    )?;
    write_meta(
        conn,
        "schema_version",
        &CALLGRAPH_SCHEMA_VERSION.to_string(),
    )?;
    write_meta(conn, "root", root)?;
    Ok(())
}

fn drop_schema(conn: &Connection) -> Result<(), Box<dyn Error + Send + Sync>> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS call_edges;
         DROP TABLE IF EXISTS symbols;
         DROP TABLE IF EXISTS files;
         DROP TABLE IF EXISTS meta;",
    )?;
    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
}

fn read_meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

fn write_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ── 杂项 ──────────────────────────────────────────────────────────────────────

/// 把损坏的库改名留档，并清掉 WAL 侧文件（否则新库会读到残留的 sidecar）。
fn quarantine_db(db: &Path) {
    if !db.exists() {
        return;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let corrupt = db.with_extension(format!("db.corrupt.{stamp}"));
    if let Err(err) = fs::rename(db, &corrupt) {
        // 被别的进程占用时改名失败；交给调用方退回内存兜底。
        log::warn!("failed to quarantine {}: {err}", db.display());
    }
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", db.display()));
        let _ = fs::remove_file(side);
    }
}

/// 冷热信号：`.star/context/index.json` 的 mtime。
///
/// 与 `codebase_search::index_mtime` 同口径 —— 索引器每次跑完都会盖章
/// （即使没有内容变化），所以它变化 ≈ "索引管线动过，值得重新看文件"。
fn index_token(root: &Path) -> Option<SystemTime> {
    fs::metadata(root.join(".star").join("context").join("index.json"))
        .and_then(|meta| meta.modified())
        .ok()
}

fn walk_options() -> crate::utils::file_walk::WalkOptions {
    // 与 `codebase_search::walk_options` 同口径：三层 ignore 交给默认值，
    // 只显式剪掉 agent 自身状态目录（否则 sync 会尝试索引刚写出的库文件）。
    crate::utils::file_walk::WalkOptions::new().exclude([".star/"])
}

fn normalized_root(root: &Path) -> String {
    root.canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

fn mtime_millis(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn canonical(root: &Path) -> PathBuf {
        root.canonicalize().unwrap()
    }

    fn graph_from_db(root: &Path) -> CallGraph {
        let root = canonical(root);
        sync(&root).expect("sync");
        load_graph(&root).expect("load").expect("db has a graph")
    }

    fn names(graph: &CallGraph, hint: &str) -> Vec<String> {
        graph
            .find_symbols(hint)
            .iter()
            .map(|symbol| symbol.name.clone())
            .collect()
    }

    /// 设计文档 §10.1：库加载与内存全量构建必须等价。
    #[test]
    fn db_graph_matches_in_memory_build() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() { b(); }\nfn b() {}\n");
        write(root, "src/c.rs", "fn c() { a(); }\n");

        let memory = crate::core::tools::codebase_search::build_call_graph(root).unwrap();
        let db = graph_from_db(root);

        assert_eq!(db.len(), memory.len(), "符号数必须一致");
        assert_eq!(
            db.edge_count(),
            memory.edge_count(),
            "解析出的调用边数必须一致"
        );
        assert_eq!(
            names(&db, "a"),
            names(&memory, "a"),
            "find_symbols 的排序结果必须一致"
        );
    }

    /// 设计文档 §10.2：只有内容变了的文件需要重解析。
    #[test]
    fn resync_reparses_only_the_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");
        write(root, "src/b.rs", "fn b() {}\n");
        graph_from_db(root);

        write(root, "src/b.rs", "fn b() {}\nfn b2() {}\n");
        let report = sync(&canonical(root)).unwrap();

        assert_eq!(report.reparsed, 1, "只有 b.rs 需要重解析：{report:?}");
        assert_eq!(
            report.fast_path_skips, 1,
            "a.rs 应走 size/mtime 快路径：{report:?}"
        );

        let graph = load_graph(&canonical(root)).unwrap().unwrap();
        assert_eq!(graph.find_symbols("b2").len(), 1, "新增符号必须入库");
    }

    /// 设计文档 §10.3：`caller_local_idx` 在加载重映射 id 后仍指向正确符号。
    #[test]
    fn call_edges_survive_the_id_remap_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() { b(); }\n");
        write(root, "src/b.rs", "fn b() {}\n");

        let graph = graph_from_db(root);
        let a = graph
            .find_symbols("a")
            .into_iter()
            .next()
            .expect("a should be indexed");
        let chain = graph.call_chain(a.id, 3);

        let callees: Vec<&str> = chain
            .callees
            .iter()
            .flat_map(|level| level.symbols.iter())
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert_eq!(callees, vec!["b"], "加载后 a 仍应调用 b");
    }

    /// 设计文档 §10.4：未变文件在第二次同步时不得被读取/重解析。
    #[test]
    fn unchanged_resync_takes_the_fast_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");
        graph_from_db(root);

        let report = sync(&canonical(root)).unwrap();
        assert_eq!(report.reparsed, 0, "没有变化就不该重解析：{report:?}");
        assert_eq!(
            report.fast_path_skips, report.files_total,
            "所有文件都应命中快路径：{report:?}"
        );
    }

    /// 设计文档 §10.5：版本号不匹配 → 整库重建（复刻 Indexer 的版本丢弃）。
    #[test]
    fn schema_version_mismatch_rebuilds_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");
        graph_from_db(root);

        let root = canonical(root);
        {
            let conn = Connection::open(db_path(&root)).unwrap();
            conn.execute(
                "UPDATE meta SET value = '0' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }

        let report = sync(&root).unwrap();
        assert_eq!(report.reparsed, 1, "版本不匹配必须触发全量重建：{report:?}");
        assert!(load_graph(&root).unwrap().is_some());
    }

    /// 设计文档 §10.6：损坏的库必须被隔离并重建，查询不失败。
    #[test]
    fn corrupt_database_is_quarantined_and_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");

        let db = db_path(&canonical(root));
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        fs::write(&db, b"not a sqlite database").unwrap();

        let graph = get_call_graph(root).expect("query must never fail on a corrupt db");
        assert_eq!(graph.find_symbols("a").len(), 1, "重建后仍应有符号");
        assert!(db.exists(), "重建应生成新的库文件");
    }

    /// 设计文档 §10.7：磁盘上删掉的文件，同步后不能残留符号。
    #[test]
    fn deleted_files_drop_out_of_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");
        write(root, "src/b.rs", "fn b() {}\n");
        graph_from_db(root);

        fs::remove_file(root.join("src/b.rs")).unwrap();
        let report = sync(&canonical(root)).unwrap();
        assert_eq!(report.removed, 1, "删除的文件应被统计：{report:?}");

        let graph = load_graph(&canonical(root)).unwrap().unwrap();
        assert!(graph.find_symbols("b").is_empty(), "b 的符号必须消失");
    }

    /// 设计文档 §10.8：未知 kind 字符串降级为 Unknown，不能让查询炸掉。
    #[test]
    fn unknown_kinds_degrade_to_unknown_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");
        graph_from_db(root);

        let root = canonical(root);
        {
            let conn = Connection::open(db_path(&root)).unwrap();
            conn.execute("UPDATE symbols SET kind = 'weird' WHERE name = 'a'", [])
                .unwrap();
        }

        let graph = load_graph(&root).unwrap().unwrap();
        let symbol = graph.find_symbols("a").into_iter().next().unwrap();
        assert_eq!(symbol.kind, SymbolKind::Unknown);
    }

    /// 设计文档 §10.9：没有符号的文件也要走组装路径（输入空间与内存版一致）。
    #[test]
    fn files_without_symbols_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/empty.rs", "\n");

        let root = canonical(root);
        sync(&root).unwrap();
        assert!(
            load_graph(&root).unwrap().is_some(),
            "空符号文件不能让加载路径返回 None"
        );
    }

    /// 设计文档 §8：库被拷到别的仓库（meta.root 不符）时必须拒绝并重建，
    /// 否则会把 A 仓库的符号当成 B 仓库的。
    #[test]
    fn database_from_another_root_is_rejected_and_rebuilt() {
        let source = tempfile::tempdir().unwrap();
        write(source.path(), "src/a.rs", "fn a() {}\n");
        let source_root = canonical(source.path());
        sync(&source_root).unwrap();

        let target = tempfile::tempdir().unwrap();
        let target_root = canonical(target.path());
        let target_db = db_path(&target_root);
        fs::create_dir_all(target_db.parent().unwrap()).unwrap();
        fs::copy(db_path(&source_root), &target_db).unwrap();

        assert!(
            load_graph(&target_root).unwrap().is_none(),
            "异根库必须被拒绝"
        );

        sync(&target_root).unwrap();
        let graph = load_graph(&target_root).unwrap().expect("rebuild");
        assert!(
            graph.find_symbols("a").is_empty(),
            "重建后只能包含目标根自己的文件"
        );
    }

    /// `caller_local_idx` 不是简单的 0/1 占位：文件里有多个符号时，
    /// 调用归属必须指向正确的那个局部下标（这里 second 的下标是 1）。
    #[test]
    fn caller_indices_remap_when_a_file_has_several_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "src/a.rs",
            "fn first() {}\nfn second() { first(); }\n",
        );
        write(root, "src/b.rs", "fn caller() { second(); }\n");

        let graph = graph_from_db(root);
        let first = graph.find_symbols("first").into_iter().next().unwrap();
        let second = graph.find_symbols("second").into_iter().next().unwrap();

        let callers_of_first: Vec<&str> = graph
            .callers_of(first.id)
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert_eq!(
            callers_of_first,
            vec!["second"],
            "first 的调用者必须是 second（局部下标 1）"
        );

        let callers_of_second: Vec<&str> = graph
            .callers_of(second.id)
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert_eq!(callers_of_second, vec!["caller"], "跨文件调用边必须保留");
    }

    /// 设计文档 §8：状态目录不可写时，查询必须退回内存构建而不是失败。
    #[test]
    fn unwritable_state_dir_falls_back_to_memory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");
        // `.star/context` 是文件而不是目录 → sync 的 create_dir_all 必然失败。
        fs::create_dir_all(root.join(".star")).unwrap();
        fs::write(root.join(".star").join("context"), b"not a directory").unwrap();

        let graph = get_call_graph(root).expect("查询必须退回内存构建而不是失败");
        assert_eq!(graph.find_symbols("a").len(), 1, "兜底结果必须完整");
    }

    /// 设计文档 §7 / §12.4：watcher 打脏 → token 失效 → 下一次查询重新同步。
    ///
    /// 这是"改完文件立刻问 CodeGraph"的回归防线：没有这一步，进程内会一直
    /// 读编辑前的旧图（`index.json` 可能根本没重新盖章）。
    #[test]
    fn invalidating_the_token_makes_the_next_query_resync() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");

        let first = get_call_graph(root).unwrap();
        assert!(first.find_symbols("fresh").is_empty());

        write(root, "src/a.rs", "fn a() {}\nfn fresh() {}\n");
        invalidate(root);

        let second = get_call_graph(root).unwrap();
        assert_eq!(
            second.find_symbols("fresh").len(),
            1,
            "失效后必须重新同步，而不是返回旧图"
        );
    }

    /// 手动基准（不进 CI）：
    /// `cargo test --release --lib -- core::context::call_graph_db::tests::bench_call_graph_paths --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_call_graph_paths() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = canonical(root);

        // 清掉旧库，保证 sync 计时是冷启动。
        let db = db_path(&root);
        let _ = fs::remove_file(&db);
        for suffix in ["-wal", "-shm"] {
            let _ = fs::remove_file(PathBuf::from(format!("{}{suffix}", db.display())));
        }

        let started = Instant::now();
        let memory = crate::core::tools::codebase_search::build_call_graph(&root).unwrap();
        let memory_ms = started.elapsed().as_millis();

        let started = Instant::now();
        sync(&root).unwrap();
        let first_sync_ms = started.elapsed().as_millis();

        let started = Instant::now();
        let loaded = load_graph(&root).unwrap().unwrap();
        let load_ms = started.elapsed().as_millis();

        assert_eq!(loaded.len(), memory.len(), "基准下仍须等价");
        assert_eq!(loaded.edge_count(), memory.edge_count(), "基准下仍须等价");

        println!(
            "callgraph bench: memory-build={memory_ms}ms cold-sync={first_sync_ms}ms load={load_ms}ms | symbols={} edges={}",
            loaded.len(),
            loaded.edge_count()
        );
    }
}
