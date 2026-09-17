# 后台索引与语义引擎增量更新 —— 设计结论

`26a19ac` 引入了后台索引（notify watcher + 低优先级 worker + serve-stale-while-rebuild）。
管线能跑，但审计后发现三类问题。本文记录结论与选定方案，作为实现与后续维护的依据。

---

## 一、现状问题

### 1. 边界口径不一致

| 问题 | 位置 |
|------|------|
| 四份互不相同的扩展名白名单 | `codebase_search.rs:164`（13 个）、`chunking.rs:178`、`integration.rs:264`、`commands/mod.rs:901` |
| `Indexer` 对任何文件都无上限地 `read` + SHA256，不看类型不看大小 | `indexer.rs:181` |
| 两个线程并发 load-modify-save 同一个 `index.json`，会丢更新 | `watcher.rs:264` × `engine.rs:152` / `:307` |
| 同尺寸 + 同秒的修改被快速路径漏检；空文件因 `entry.size != 0` 永远走慢路径 | `indexer.rs:207` |

### 2. 语义引擎的"增量"名不副实

`run_incremental_rebuild` 里 `index.json` 是增量的，但 `build_engine_into_cache`
每次从零重建整个 `SearchEngine`。同时 watcher 是 `RecursiveMode::Recursive` 且无
exclude，跑一次 `cargo build` 会灌进几千个 `target/` 事件。

### 3. UI 零出口

`watcher` 的 `status_text` / `is_rebuilding` / `is_engine_ready` 没有任何调用点；
`/index` 显示的是另一套恒为空的子系统；`SemanticSearch` 的结构化结果被压成
1200 字符 + "N lines"；`ui/utils/tool_formatter.rs` 整个模块是死代码。

---

## 二、边界正确性

### 2.1 扩展名白名单：单一事实源

在 `codebase_search.rs` 定义唯一权威：

```rust
pub const SEMANTIC_INDEXABLE_EXTS: &[&str] = &[
    // tree-sitter 可解析
    "rs", "py", "pyi", "js", "jsx", "mjs", "cjs", "ts", "tsx",
    "go", "java", "c", "h", "cpp", "cc", "cxx", "hpp", "hxx",
    // 纯文本 / 配置（走 SmartChunker 启发式分层）
    "md", "txt", "json", "toml", "yaml", "yml",
];
pub fn is_indexable_ext(ext: &str) -> bool { SEMANTIC_INDEXABLE_EXTS.contains(&ext) }
```

`language_for_call_graph` 保持"只返回 tree-sitter 语言"的语义不变，但补齐
`pyi`/`mjs`/`cjs` 使其与 `chunking.rs:178` 一致。`integration.rs:264` 与
`commands/mod.rs:901` 改为委托 `is_indexable_ext`。

### 2.2 `Indexer` 上限与类型过滤

- `INDEX_MAX_FILE_BYTES = 512 KiB`、`INDEX_MAX_TOTAL_BYTES = 12 MiB`（与语义引擎同口径）。
- 超过单文件上限 → 记入 `skipped_large`，**不读不哈希**，但仍写入 `found_paths`（避免被误判为删除）。
- 累计超总上限 → 停止读新文件，`truncated = true`。
- 二进制过滤：前 8 KiB 内含 `\0` 直接跳过（lossy 解码纯属浪费）。
- `IndexResult` 增加 `skipped_large: usize` 与 `truncated: bool`。

### 2.3 并发写 `index.json`

进程内 `parking_lot::Mutex` 覆盖整个 read-modify-write。跨进程不保证 ——
同一仓库同时跑两个 StarCode 非目标场景，注释写明。

### 2.4 漏检修复

把快速路径判定收进具名函数，并：
- 去掉 `entry.size != 0`（空文件也该走快速路径）；
- 额外要求 `mtime_ms != 0`（旧格式回填失败时强制走慢路径）；
- 注释写明这是"尽力而为的启发式，宁可多读一次不可漏检"。

---

## 三、语义引擎增量更新（核心）

### 3.1 `SearchEngine` 结构：swap-remove + doc_id 重映射

选 **swap-remove 保持 `documents` 稠密**，而非墓碑：

```rust
struct DocumentEntry { path: String, chunks: Vec<CodeChunk> }

pub struct SearchEngine {
    index: HashMap<String, Vec<(usize, usize)>>,  // word -> [(doc_id, chunk_id)]
    documents: Vec<DocumentEntry>,
    path_to_doc: HashMap<String, usize>,
    doc_freqs: HashMap<String, usize>,
    // 没有 total_docs 字段 —— 由 documents.len() 派生
}
```

关键点：

- **删除 `total_docs` 字段，改为 `fn total_docs(&self) -> usize`**。这是收益最高的
  一处简化：独立计数器在增量删除下极易与实际文档数脱节，而 IDF 依赖它。
  稠密存储让 `total_docs() == documents.len()` 成为结构性不变量。
- **`chunk_words(path, chunk)` 为插入与删除共用的分词口径**。两边各写一份是增量索引
  最经典的 bug 来源（加入的词与移除的词不一致 → `doc_freqs` 单向漂移）。
- `upsert_document` 先 `remove_document` 再插入；`remove_document` 做 swap-remove
  后**必须重映射被换到坑位的文档**（倒排表里的 `doc_id` 要跟着改）——
  这是最容易写错的地方。
- `doc_freqs` 递减到 0 时删 key；posting 清空时删词条，不留空列表。
- `verify_invariants()`（`#[cfg(test)]`）自检 `index` / `doc_freqs` / `path_to_doc`
  三者与 `documents` 一致。

### 3.2 `SearchEngineCacheManager` 补丁 API

```rust
pub struct EngineMeta { pub doc_count: usize, pub total_bytes: u64, pub index_mtime: Option<SystemTime> }
pub enum PatchOutcome { Applied, Missing, Stale }

pub fn engine_meta(&self, key: &(PathBuf, String)) -> Option<EngineMeta>;
pub fn patch_engine<F>(&self, key, expected_mtime, f: F) -> PatchOutcome
where F: FnOnce(&mut SearchEngine) -> bool;
```

- `expected_mtime` 在**写锁内**比对 —— mtime 即并发令牌，`Stale` 检测天然 race-free，不需要新锁。
- **`Applied` 时无条件盖新 mtime**。无操作补丁（变更文件全是非索引类型）若不盖，
  `get_engine` 永不命中 → 每次查询都触发刷新 → 死循环。这是本设计最隐蔽的正确性点。
- tree-sitter 分块在**锁外**，`patch_engine` 在**锁内** —— 写锁只持有 O(触及的 posting)。

### 3.3 `admit_file`：全量与增量的单一策略点

```rust
enum Admission { Admit, SkipExt, SkipTooLarge, SkipBudget }
fn admit_file(path, size, used_files, used_bytes, replaced_bytes, limits) -> Admission;
```

全量构建与增量路径**共用**它，使白名单/上限不可能分叉。

### 3.4 全量重建启发式

```rust
const INCREMENTAL_MIN_CORPUS: usize = 8;
const INCREMENTAL_MIN_CHANGED: usize = 4;
const INCREMENTAL_MAX_CHANGE_RATIO: f64 = 0.30;

fn should_full_rebuild(indexable_changed: usize, doc_count: usize) -> bool {
    if doc_count < INCREMENTAL_MIN_CORPUS { return indexable_changed >= INCREMENTAL_MIN_CHANGED; }
    (indexable_changed as f64) / (doc_count as f64) > INCREMENTAL_MAX_CHANGE_RATIO
}
```

- 小语料用绝对阈值（3 个文件改 1 个就是 33%，比例无意义）。
- `indexable_changed` 只数扩展名过白名单的文件 —— `git checkout` 碰 500 个
  `.lock`/`.svg` 不该触发重建。**删除不计入分子**：删除永远比重建便宜，
  且 `rm -rf src/` 由补丁正确处理（引擎收缩）。
- 这三个是调优旋钮，不是正确性旋钮。

### 3.5 变更集来源：`IndexResult`，不是 dirty 集合

dirty 集合目前口径不一致（绝对路径 vs 相对路径混用），直接用会静默失败。
`Indexer::index_project()` 返回的 `new_blobs` / `removed_blobs` 是 hash 校验过的、
路径口径正确的权威变更集。

### 3.6 关键不变式：增量路径里 "skip" 意味着 "remove"

文件**涨过** `max_file_bytes`、或**扩展名从可索引变为不可索引**时，该文件可能
**原本就在索引里**。"跳过"必须表达为 `PatchOp::Remove`，否则留下陈旧文档。
这是最容易被实现错的一条。

### 3.7 其它边界

| 情况 | 处理 |
|------|------|
| 删除文件 | `removed_blobs` → `Remove`，预算**先**释放 |
| 重命名 | `removed=[old]` + `new=[new]`，同批下发；移除先记账 → 不会误触上限 |
| 空 chunks | → `Remove`（文件变空/纯空白，不能留陈旧文档） |
| `index_file_safe` 返回 `Err` | 文件仍在 → 保留旧版本；已消失 → 移除 |
| 预算饱和 | `SkipBudget` → 全量重建（全量有明确的截断语义，补丁没有合理的逐出策略） |
| 引擎不存在 / mtime 不匹配 | `Missing` / `Stale` → 全量重建 |
| `index.json` 从未写过 | `index_mtime` 为 `None` → 全量重建 |
| `\0refresh-all` 哨兵 | 路由到全量分支 |
| 冷语料（`doc_count == 0 && io_files > 0`） | 全量重建（否则把空引擎补成 1 文件后谎报 Fresh） |

---

## 四、watcher 调整

1. `run_incremental_rebuild` 第 2 步由 `build_engine_into_cache` 换成
   `update_engine_in_cache(root, &cache, &result)`。
2. **`rebuilding` 重标记不得用哨兵**。现在插入 `"\0refresh-all"`；增量下这是脚枪 ——
   补丁窗口内到达的编辑会强制下一轮全量重建。下一轮反正从 `index_project()` 重新
   推导权威变更集，重新标记**实际路径**即可。哨兵只留给 `request_refresh` 与 root 外事件。
3. **锁纪律**：`coord.engine_cache.lock()` 必须在 `update_engine_in_cache` 之前释放。
   现在 guard 横跨整个 `build_engine_into_cache` 调用；全量重建无害，但增量路径会
   回调 `engine_meta`/`patch_engine`。把 `Arc` clone 出来再 drop guard。
4. `last_rebuild_failed` 语义不变：只有真失败才 `Err`，`FullRebuild` 是成功。
   `resolve_engine` 依赖它决定是否回退同步构建，误报会禁用 Warming 快路径。
5. **事件过滤**：用 `ignore::gitignore::GitignoreBuilder` 构建一次匹配器
   （项目 `.gitignore` + `.starignore` + `~/.star/ignore` + `VCS_DIRS` +
   `CODEBASE_SEARCH_EXCLUDES`），复用 `utils::file_walk` 的
   `global_ignore_file` / `project_ignore_file` 保持口径一致。

`engine.rs` **无需改动** —— 它的后台刷新只碰 `IndexCache`，引擎缓存完全由 watcher
worker 驱动。`clear_index_cache` 与在途补丁的竞争是安全的（`Missing` → 全量重建，
无跨 await 持锁）。

---

## 五、UI 显示

### 5.1 状态栏索引指示

- `ChatState` 增加 `index_status` 字段。
- UI 主循环 1s 节流轮询 `watcher::status_text()`，状态变化时置 `needs_redraw`。
  **不要在渲染路径里调** —— `status_text()` 会取锁。
- `build_status_spans` 加一段：`warming` / `rebuilding` / `ready` / `off`（不显示）。

### 5.2 修 `/index`

`extended.rs:1164` 现场 `IncrementalIndex::new(&cwd)` 造空结构 → 恒打印
`Index v0: 0 files indexed, 0 dirty, last updated: never`。

- 改为读真实持久化索引 + `watcher::status_text()`。
- `IncrementalIndex::get_changed_files` 的 `git diff --name-only HEAD~10` 在提交数
  <10 的仓库直接报错 → 改为 `HEAD` 或失败回退空集。
- `/index structure` 保留，扩展名过滤改用 `is_indexable_ext`。

### 5.3 `SemanticSearch` 渲染

- `tool_render.rs:render_rich_tool_content` 加 `SemanticSearch` 分支：
  折叠态 `Found N results` + 前 3 个 `path:line (score)`；展开态完整
  File/Score/Lines/Context/Signals 块。
- `Warming` 来源的输出单独识别为 `Index building…`，而非 "0 results"。
- **删除死代码 `ui/utils/tool_formatter.rs`**（零调用点），避免留下第二份会漂移的实现。

---

## 六、测试策略

测试放各 src 文件的 `#[cfg(test)] mod tests`（仓库约定）。

**基石是差分测试 `incremental_matches_full_rebuild`**：
建临时目录 → 全量构建 → 增量应用脚本化变更（改一个、删一个、重命名一个、
加一个、清空一个、加非索引文件、涨过上限）→ 从变更后的文件系统重建全量 →
断言探针查询的**文件路径、chunk 内容、分数**一致，且 `doc_count` / `total_bytes` /
`doc_freqs` 相等。这一个测试覆盖大半簿记测试。

其余按模块：`search_engine`（swap-remove 重映射、upsert 去重、doc_freqs 递减到零）、
`search_cache`（mtime 令牌、**无操作补丁仍盖 mtime** 的死循环回归）、
`codebase_search`（`admit_file` 各分支、**涨过上限 → 移除** 的高风险用例）、
`watcher`（重标记保留实际路径、哨兵路由、事件过滤）。

> 注意：涉及 limits 的测试必须用 `update_engine_in_cache_with_limits`（显式 limits）。
> `codebase_search_limits_for_profile` 读进程级环境变量，而 Rust 测试并行跑，
> 改 env 的测试会 flaky。这正是要拆成两个函数的原因。

---

## 七、关键设计决策速查

- swap-remove + 重映射，而非墓碑 —— 保持读路径稠密，`total_docs()` 成为结构不变量。
- 删除 `total_docs` 字段，派生 —— 收益最高的正确性简化。
- `chunk_words` 为插入/删除共用分词 —— 结构上杜绝两侧口径分叉。
- 分块在锁外，补丁在锁内 —— 写锁不跨 tree-sitter。
- mtime 作并发令牌，在写锁内比对 —— 无需新锁即 race-free。
- `Applied` 无条件盖 mtime —— 防刷新死循环。
- 变更集来自 `IndexResult` 而非 dirty 集合 —— hash 校验、路径口径正确。
- 增量路径里 "skip" == "remove" —— 最易实现错的一条。
- `admit_file` 为单一策略点 —— 全量与增量不可能在口径上分叉。
- 差分测试为基石 —— 证明观测等价，而非零散测簿记。