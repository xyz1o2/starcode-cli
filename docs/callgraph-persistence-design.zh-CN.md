# CallGraph SQLite 持久化设计（`.star/context/callgraph.db`）

状态：**已实现**（2026-11-02，`src/core/context/call_graph_db.rs` + 13 个模块内测试）。
实现遵照 CLAUDE.md 约定（tests 放 `src/` 内 `mod tests`、`cargo test --workspace --lib -- --test-threads=1`、中文注释 + 英文用户串、`cargo fmt --all -- --check` 是 CI 阻断项）。

---

## 1. 背景与现状痛点

调用图当前是**纯内存即时构建**，没有任何持久化：

- `build_call_graph(root)`（`src/core/tools/codebase_search.rs:1598`）：每次调用都全量 `walk` 整个仓库 → 逐文件 `read_to_string` → tree-sitter parse → `extract_symbols` → `CallGraph::build`。
- 唯一消费方是 `trace_call_chain`（`codebase_search.rs:1664`），被 CodeGraph 工具调用（`code_graph_tool.rs:69`）。也就是说**每次 who-calls / callees / call-chain 查询都重跑一次全量 parse**。
- `Indexer` 那套增量机制（size/mtime 快速路径 + sha256 + 格式版本号，见 `src/core/context/indexer.rs:62,146-171,189-195`）只服务于 CodebaseSearch 的倒排索引，调用图完全没有搭上。

副词法引擎有倒排索引兜底，但"精确符号 → 调用关系"这类查询（尤其是 agent 追执行路径时）每次付出全仓库 tree-sitter parse 的代价，换来的收益却只是查一次。这是 CodeGraph 工具响应慢的主要来源。

参照物是 codegraph（github.com/colbymchenry/codegraph，MIT，76k+ stars）：预索引符号/边/文件哈希进 SQLite，文件变动增量同步，查询 O(log)。"学设计、不引依赖"——本设计在 StarCode 现有基建上落地同一模式。

## 2. 目标

1. **持久化**：`Vec<FileSymbols>`（每文件的符号定义 + 调用位）物化到 `.star/context/callgraph.db`（SQLite，`rusqlite` bundled）。
2. **增量**：变更检测复刻 `Indexer` 的机制——size/mtime 快路径零读取，hash 变了才重解析；按文件粒度 delete + insert 事务替换。
3. **读路径零语义变化**：查询 API（`find_symbols` / `callers_of` / `callees_of` / `call_chain` / `format_call_chain`）与输出格式**完全不变**——库只是把"从磁盘 parse"换成"从库组装"，然后原样走 `CallGraph::build`。
4. **失败兜底**：库缺失/损坏/版本不匹配 → 回退现有内存全量构建，查询不失败。

## 3. 非目标

- 不引入向量 / embedding（保持词法路线，与索引器一致）。
- 不做 `unresolved_refs` 两遍式候选消歧（codegraph 的升级项）：现状就是 `callee_name` 字符串同名匹配、一callee名多定义全命中，persist 库保持同一语义；候选消歧留给后续阶段。
- 不改 CodebaseSearch 的倒排索引；两者共享 watcher 触发点但不共享数据。
- 第一版不做 FTS5 全文符号搜索（后续阶段；它会把 `find_symbols` 的 `contains` 内存扫描换成 SQL 查询，收益/风险比放在二期）。
- 不新增边类型（只存调用边；导入/继承边与 codegraph 的四类 edges 是二期能力）。

## 4. 关键代码事实（决定 schema 形状）

来源均以现行源码为准：

| 事实 | 出处 | 对设计的影响 |
|---|---|---|
| `SymbolId = usize`，是跨文件**全局自增计数器**（walk 顺序敏感） | `symbol.rs:12`、`build_call_graph` 里贯穿传递的 `next_id` | id 不能入库；入库用 (rel_path, 局部下标) 定位，**加载时重新分配 id** |
| `CallEdge.callee_id` 提取时恒 `None`（"resolved later in cross-file pass"） | `symbol.rs:189` | 库里**只存 callee_name 字符串**，解析留给加载后的 `CallGraph::build`，与现状一致 |
| 跨文件解析只发生在 `CallGraph::build`，按 `callee_name` 全名匹配、一 callee 名命中全部定义 | `call_graph.rs:90-110` | 不需要持久化已解析边；`CallGraph::build` 一行不动 |
| `SymbolDef` 字段：name / kind / file_path / start_line / end_line / signature | `symbol.rs:46-55` | `SymbolKind::as_str()` 恰好是稳定字符串，可直接列存 |
| 语言白名单 `language_for_call_graph`（rs/py/js/ts/go/c/cpp/java…） | `codebase_search.rs:1646` | 同步流程沿用同一白名单，不扩语言 |
| 忽略规则：`.star/` 已在 watcher 的 builder 里排除 | `watcher.rs:772` | 库文件天然不会造成自索引死循环 |

**符号 id 重新分配为什么是安全的**：`CallGraph::build` 的解析与 id 的具体值无关（边归属靠 name_index 查表）；id 只在图内部流转，从不进入工具输出（`format_call_chain` 不含 id）。当前内存版的 id 也是只由 walk 顺序决定的——所以"每次加载重新分配"是行为等价，不是近似。

## 5. Schema（SQLite，v1 最小集）

```sql
-- 版本与元信息，两行 key/value：
--   schema_version = '1'   开库时版本不匹配 → 删库重建（复刻 Indexer::load_index 的版本丢弃逻辑）
--   root = '<canonical>'   防库被拷去别的仓库误用
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE files (
  rel_path     TEXT PRIMARY KEY,     -- 相对根、正斜杠化；与现有 rel_path 口径一致
  content_hash TEXT NOT NULL,        -- sha256（复用 Indexer::calculate_hash 的 sha256_hex）
  size         INTEGER NOT NULL,     -- 字节；fast-path 判定用
  mtime_ms     INTEGER NOT NULL,     -- unix ms；fast-path 判定用
  symbol_count INTEGER NOT NULL DEFAULT 0,
  edge_count   INTEGER NOT NULL DEFAULT 0,
  indexed_at   INTEGER NOT NULL      -- 写入时间（unix ms，可观测）
);

-- 符号定义。local_idx 是文件内的符号下标（即提取序），
-- (rel_path, local_idx) 联合主键；全局 SymbolId 永不入库。
CREATE TABLE symbols (
  file_rel_path TEXT NOT NULL REFERENCES files(rel_path) ON DELETE CASCADE,
  local_idx     INTEGER NOT NULL,
  name          TEXT NOT NULL,
  kind          TEXT NOT NULL,        -- SymbolKind::as_str()
  start_line    INTEGER NOT NULL,
  end_line      INTEGER NOT NULL,
  signature     TEXT NOT NULL,
  PRIMARY KEY (file_rel_path, local_idx)
);

-- 调用位（未解析）。caller 用文件内局部下标指回符号；callee 只存名字。
CREATE TABLE call_edges (
  file_rel_path    TEXT NOT NULL REFERENCES files(rel_path) ON DELETE CASCADE,
  caller_local_idx INTEGER NOT NULL,  -- → symbols.local_idx（同文件）
  callee_name      TEXT NOT NULL,
  line             INTEGER NOT NULL
);
CREATE INDEX idx_call_edges_lookup ON call_edges(file_rel_path, caller_local_idx);
```

开库与连接设置（查询与 sync 各自连接或同一连接，见 §9）：

```sql
PRAGMA journal_mode = WAL;   -- 读写并发的关键：sync 写库期间查询可继续读
PRAGMA synchronous = NORMAL; -- WAL 下够安全且快
PRAGMA foreign_keys = ON;    -- 级联删除依赖它，rusqlite 默认关
```

版本历史：
- `CALLGRAPH_SCHEMA_VERSION = 1`（初版）
- 语义变化（如 kind 词表调整、加新列影响读结果）一律 bump 版本 → 旧库整文件重建，不做就地迁移（量级不值得）。

## 6. 增量同步（sync）

新增模块 `src/core/context/call_graph_db.rs`：

```text
fn sync(root: &Path) -> SyncReport {
  let canonical = root.canonicalize();
  开库：
    文件不存在 → 建表 + 写 meta(seed root)；
    存在 → 读 meta.schema_version；不匹配或读失败 → 删除后重建
  walk（file_walk::walk + 现有 walk_options，三层 ignore 同口径）：
    收集支持语言的白名单文件 → 统一按 rel_path 排序（见下"确定性"）
  对每个文件：
    stat → (size, mtime_ms)
    快路径：files 行 size/mtime 相等 → 跳过（零读取、零 parse）
    否则 read_to_string + sha256：
      hash 相同 → 仅回填 size/mtime（旧行缺列值时），不重解析
      hash 不同 / 新文件 → extract_symbols(content, rel_path, ext, lang, &mut LOCAL next_id)
        BEGIN;
          DELETE FROM symbols WHERE file_rel_path = ?;  -- CASCADE 不含 call_edges，手删
          DELETE FROM call_edges WHERE file_rel_path = ?;
          INSERT symbols (local_idx = 提取序 0..n)
          INSERT call_edges (caller_local_idx = caller_id - file_symbol_base)
          UPSERT files 行（hash/size/mtime/counts）
        COMMIT;（per-file 事务，见决策）
  DB 有而磁盘没有的 rel_path → DELETE（关联行级联消）
  last_sync_token 更新（§7）
}
```

实现要点：

- **per-file delete+insert 事务**（已定决策）：一个文件的变更不影响其他文件的行；中途崩溃靠 WAL 回滚，文件级原子。
- **初次全量**分批：单文件一个事务在大仓首建时会 fsync 数千次。第一版用**每 200 个变更文件一个事务**的分批提交（单文件粒度的原子性被放宽为"一批"，WAL 仍保证崩溃不了库——未提交的整批回滚，已提交的前批有效，next sync 会补齐）。
- **确定性**：`walk` 是相对路径排序无关的收集；写库与加载都显式按 `rel_path` 排序。这样符号 id 分配、`find_symbols` 的"路径稳定排序"（`call_graph.rs` 的可复现性测试）不受目录遍历顺序影响。
- **`extract_symbols` 返回 `None`**（语言 parse 器构造失败等）：该文件**不入库**——与现行 `build_call_graph` 的 `if let Some(fs)` 行为一致。
- **caller 局部下标换算**：`extract_symbols` 内部用传递进来的 `next_id`，`CallEdge.caller_id = caller_sym.id`。sync 每文件以文件基址（doc：`file_symbol_base = 本文件第一个符号的全局领取值`）换算 `caller_local_idx = caller_id - file_symbol_base`。等价且更稳的写法：sync 阶段根本不透传全局计数器，给 `extract_symbols` 的 `symbol_id_base` 每文件传 0，`caller_id` 天然就是局部下标，无需减法。采用后者（零假设）。
- **不再需要 `std::fs::read` 判二进制**：现行路径读文件失败即跳过，这里保持同判。
- **大文件上限**：沿用 indexer 的 skip_large 精神（具体阈值与 extractor 实际防御一致，v1 先与现行 build 相同——不设限、靠 parse 结果为 None 淘汰；如基准发现慢文件再纳入）。

## 7. 读路径与工具集成

```text
fn get_call_graph(root: &Path) -> Result<CallGraph, ...> {   // 新查询入口，trace_call_chain 改调它
  let canonical = ...;
  let token = index_mtime(canonical);        // 与 SearchEngineCacheManager 同一款冷热判定
  if last_sync_token(canonical) == token {  // 未变 → 纯读
    if let Some(graph) = load_graph(root) { return Ok(graph); }
    // 库被删/坏 → 落到全量兜底分支
  }
  sync(root)?;                               // 全量/fast-path 增量 + 写库
  if let Some(graph) = load_graph(root) { return Ok(graph); }
  build_call_graph(root)                     // 库彻底不可用（如磁盘只读）时的内存兜底
}
```

- `trace_call_chain` 只把 `build_call_graph(root)?` 换成 `get_call_graph(root)?`——工具 schema、输出文案、CodeGraph 工具零改动。
- `build_call_graph` 保留为公开兜底路径（不改签名）。
- **首查行为**：冷启动第一次 CodeGraph 查询 = 全量 walk+parse+写库（比现在多一次写库，写库只是把已提取结果序列化，远低于 parse 成本）；**第二次起**（文件未变）= 打开库 + 全表组装 + `CallGraph::build`，**无磁盘读、无 parse**。
- 查询路径同步执行的线程与现状相同（CodeGraph 工具本来就是同步阻塞调用），不引入异步改造。
- `index_mtime(canonical)` 的 token 缓存放模块内 `OnceLock<Mutex<HashMap<PathBuf, Option<SystemTime>>>>`；条目数 = 进程内项目数（个位数），无需淘汰。
- **失效联动（已实现）**：watcher worker 在消费脏快照时，若包含**真实文件路径**（排除 `\0refresh-all` 哨兵——它可能来自周期性 refiller）就调用 `call_graph_db::invalidate(root)`。agent 自己的 Edit/Write 经 `mark_files_dirty` 进入同一个脏集合，因此也走这一条路径；外部编辑则由文件事件进入。失效只清进程内 token（零 I/O），下一次查询再走 fast-path sync，所以"改完文件立刻问 CodeGraph"不会读到编辑前的旧图。

`load_graph` 的组装（schema → FileSymbols → CallGraph::build）：

```text
fn load_graph(root) -> Option<CallGraph> {
  let db = 打开（不自动重建，仅读）；
  meta 校验（schema_version + root）失败 → None
  SELECT rel_path FROM files ORDER BY rel_path;
  let mut next_id = 0usize;
  let mut file_symbols = Vec<...>;
  for rel_path:
    symbols = SELECT ... WHERE file_rel_path = ? ORDER BY local_idx;
    NOTE：空符号文件也要 push 空 FileSymbols（与现状 build_input 空间一致）
    edges  = call_edges 同文件查询；
      caller_id = local_to_global[caller_local_idx]（本文件局部序 → 当前 next_id 份额）
    各 SymbolDef.id = next_id 顺序领取
    file_symbols.push(FileSymbols { ... });
  Some(CallGraph::build(&file_symbols))
}
```

kind 反序列化：按 `SymbolKind::as_str()` 词表 match；未知字符串 → `Unknown`（防御将来词表变动不炸查询）。

## 8. 失败与兜底

| 场景 | 行为 | 参照 |
|---|---|---|
| `.star/context/callgraph.db` 不存在 | `sync` 建库建表（首次全量）后 load | 首装 |
| `meta.schema_version` 不匹配 | 删除库文件 → 重建 | `Indexer::load_index` 的版本丢弃（`indexer.rs:152-159`） |
| 确认损坏（`NotADatabase` / `DatabaseCorrupt`） | 把库 rename 成 `callgraph.db.corrupt.<ts>` 后重建；rename 失败（被占用）则直接重建也允许失败 → 本次退回内存兜底 | 同上 |
| 锁争用（另一进程正在写）/ 权限打不开 | **不隔离**，直接退回内存构建；有 5s `busy_timeout` 兜底 | 多进程共用项目 |
| `sync` 中途崩溃 | WAL 未提交事务自动回滚，库保持上个一致版本 | SQLite 保障 |
| 磁盘只读 / 开不了写 | 返回内存 `build_call_graph` 结果，写库失败仅 `log::warn!` | 查询永不因持久化失败而失败 |
| 库落后于磁盘（watcher 没打脏） | fast-path 判定仍会拦住：`last_sync_token != index_mtime` → sync 先跑；token 判定粒度见 §7 | token 与引擎缓存同源 |

## 9. 依赖与并发

- `Cargo.toml` 新增：`rusqlite = { version = "0.32", features = ["bundled"] }`。Windows 构建必须 bundled（不做系统 sqlite3 的探测），编译产物 +~2-3MB，一次性编译时间成本可接受。
- 并发模型：
  - **sync 单飞**：进程级 `parking_lot::Mutex<()>`（`SYNC_LOCK`）串行化 sync——watcher 批量打脏时不会并发写库。
  - **跨进程争用**：`busy_timeout = 5s`；超时后按普通错误退回内存兜底，**不隔离库**（隔离只针对确认损坏，见 §8）。
  - **读不阻塞写**：WAL 允许查询连接与 sync 连接并发；v1 简化为"每次操作现开一个 `Connection`"（SQLite 连接开销 μs 级），不做连接池。
  - `Connection` 非 `Sync`，不跨线程共享实例，各路径各开自己的连接。

## 10. 测试计划（mod tests in src/，`--test-threads=1`）

沿用 `codebase_search.rs` / `indexer.rs` 测试的临时项目 fixture 模式（tempdir + 相对路径文件构造）。

1. **等价性**：`sync(temp 项目) → load_graph` 的 `len()` / `edge_count()` / `find_symbols().len()` 与一次性内存 `build_call_graph` 完全相等。
2. **解析等价性**：变更一个文件后 re-sync，`find_symbols("X").first().file_path` 与内存全量重 build 结果一致。
3. **caller 局部下标重映射**：构造一文件两个函数 A→B 调用、跨文件 C→A 调用，load 后 `call_chain` 的调用者/被调者名称结构与内存版一致。
4. **fast-path**：第二次 `sync` 对未变文件零读取——以 `SyncReport.fast_path_skips == files_total` 且 `reparsed == 0` 断言（不额外包 read recorder）。
5. **版本不匹配重建**：`meta.schema_version` 手写 `0` → sync 全量重建且 files 行数恢复。
6. **损坏兜底**：往 callgraph.db 写垃圾字节 → `get_call_graph` 返回与内存构建等价的结果且库被重建。
7. **删除同步**：物理删文件 → sync 后 files 行与对应 symbols/call_edges 行消失（级联）。
8. **kind 词表防御**：手工插一行 `kind='weird'` → load 后该符号 `SymbolKind::Unknown`，查询不出错。
9. **空 FileSymbols**：extract 出 `Some(fs)` 但 symbols 为空的文件照样组装（和内存 build 的输入空间一致）。
10. **异根校验**：把 A 仓库的库拷到 B 仓库 → `load` 返回 `None`，`sync` 重建为 B 自己的内容。
11. **多符号局部下标**：一文件内两个函数（后者调用前者）→ 加载后 `callers_of` 指向正确的局部下标；跨文件边同时保留。
12. **状态目录不可写**：`.star/context` 被占成一个普通文件 → `get_call_graph` 仍返回内存构建的完整结果。
13. **dirty 联动**：改文件后调用 `invalidate` → 下一次 `get_call_graph` 必须看到新符号（而不是返回旧图）。

## 11. 观测

- `log::info!` 记录每次 sync：耗时、walk 文件数、快路径命中数、重解析数、删除数；load：耗时与组装符号/边数。
- 手动基准（不自动化进 CI）：
  `cargo test --release --lib -- core::context::call_graph_db::tests::bench_call_graph_paths --ignored --nocapture`

### 基准（本仓库，release，2026-11-02）

| 路径 | 耗时 | 说明 |
|---|---|---|
| 内存全量构建（旧行为） | 2081 ms | walk + tree-sitter 全量 parse + 解析边 |
| 冷同步（建库） | 2340 ms | 首查代价：全量 parse + 写库（+12%） |
| 库加载（热查询） | **87 ms** | 全表读取 + `CallGraph::build`，无磁盘解析 |

规模：13,934 个符号 / 167,071 条解析边。第二次起的查询约快 **24×**，且与仓库大小解耦——
这是持久化的全部收益来源。

## 12. 后续阶段（本期不做）

1. **FTS5 符号全文**（对标 codegraph `nodes_fts` + 触发器）：`find_symbols` 的 `contains` 内存扫描换 SQL，支持"记不清全名"的场景，配合 `boundedEditDistance` 容错。
2. **`explore` 复合工具**：一次调用返回"调用链 + 命中符号源码片段 + 影响面（callees 下一跳）"——对标 codegraph benchmark 数字（官方口径 tools -88% / tokens -62%）的真实来源。
3. **查询评分加分项移植**：`nameMatchBonus` / `kindBonus` / `scorePathRelevance` / `boundedEditDistance`。
4. ~~**watcher dirty 联动**~~ —— **已实现（惰性清 token）**：Edit/Write 与 watcher 真实事件都会让 token 失效，查询时增量同步。若将来要"查询零等待"，可让协调器 rebuild 后主动 `sync`（当前不做，避免没人用 CodeGraph 时白跑）。
5. **`unresolved_refs` 两遍式候选消歧 + `provenance`**（codegraph 的四类边 + 边溯源）。
6. **PageRank 式 repo map**（aider 思路，给冷启动的首查提供全局地图）。

## 13. 评审待决问题

1. 初次全量的分批粒度：200 文件/事务是否合适（基准后定）。
2. `files` 表是否需要 `UNIQUE(root)` 防跨仓库误用之外的什么（现在 root 存 meta 单行）。
3. ~~`callgraph.db` 是否要在 `.gitignore` 显式列出~~ —— 已定：忽略 `.star/context/`
   （生成物目录，含 `index.json` 与 `callgraph.db`），`.star/settings.json` 保持可提交。

---

### 附录 A：与 codegraph 设计的对照

| 能力 | codegraph | 本设计（v1） | 二期 |
|---|---|---|---|
| 存储 | SQLite（nodes/edges/files/unresolved_refs/nodes_fts/schema_versions） | files/symbols/call_edges/meta 四表 | FTS5 与两类新表 |
| 增量 | content_hash + 原生文件事件 + 2s debounce | content_hash + size/mtime fast path，查询 token 触发 | watcher dirty 联动 |
| 跨文件边 | unresolved_refs 两遍式候选解析 | callee_name 同名匹配（与现行为一致） | 候选消歧、provenance |
| 符号搜索 | FTS5（name/qname/docstring/signature） | 内存 contains 扫描（不变） | FTS5 |
| 工具形态 | explore 复合调用 | CallGraph 工具输出不变 | explore |
| 语言 | 20+ | 现有 call graph 白名单（9 语言族） | 跟随 tree-sitter lang crate |
