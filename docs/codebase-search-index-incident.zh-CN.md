# CodebaseSearch 索引永远建不出来 + 模型无限重试 —— 根因修正版

- 日期：2026-11-05
- 仓库：`H:\test\starcode`（Rust CLI，branch `dev-m`）
- 复现环境：WSL2，仓库在 `/mnt/h/test/ARTEX`（Windows 盘，696 文件 / 387 Go 文件 / 667 条索引条目）
- 取代：2026-11-04 那份「四处修复」总结的**根因分析部分**（代码改动已落地且正确保留，但根因归因有误，见 §2）

> **给转发方的一句话**：原总结的四个根因描述的都是**真实存在**的代码问题，但**没有一个是 ARTEX 这次事故的致因**。真正的根因是「分裂缓存」——watcher 和工具读写的是两个不同的缓存对象。以下逐条给出证据。

---

## 1. 结论摘要

| | 原总结 | 本修正版 |
|---|---|---|
| 根因 | A 显式调用不等待 / B 文案邀请重试 / C 截断不落地 / D root 归属 | **E 分裂缓存**（watcher 写 A 桶，工具读 B 桶） |
| ARTEX 是否触发 A | 是（显式调用无等待） | 是，但**不是致因**——修好 E 之后 A 依然存在，查询已正常 |
| ARTEX 是否触发 B | 是（文案邀请重试） | 是，但**不是致因** |
| ARTEX 是否触发 C | 声称是 | **否**（`index.json` 667 条 / 7.19 MB < 12 MB 预算，日志零条 `truncated`） |
| ARTEX 是否触发 D | 声称是 | **否**（ARTEX 路径下 `canonicalize` 是恒等变换，见 §3.4） |
| 修复后 | — | ARTEX 生产路径端到端通过，直接命中 `server/repeater.go:61` |

---

## 2. 真正的根因：分裂缓存（split-brain cache）

### 2.1 代码位置

```text
src/agent/agent_core.rs:221   watcher 写入 → shared_search_cache()          ← 进程级单例
src/agent/agent_core.rs:260   工具读取   → context_engine.search_cache       ← 另一个实例！
src/core/context/engine.rs:103 ContextEngine::new() 里 Arc::new(SearchEngineCacheManager::new())
```

`lazy_init` 在 `agent_core.rs:262` 用 `with_cache(...)` 调用 `register_tool` —— 这会**按名字覆盖**掉 `runtime_bootstrap`（`agent_runtime.rs:145`）里那个已经挂好共享缓存的工具。于是最终生效的是「读 B 桶」的那份。

结果：watcher 把索引建进 A 桶，工具去 B 桶取 —— **永远取不到**。

### 2.2 ARTEX 日志的直接证据

`.star/logs/starcode_debug.log`：

```text
18:12:40.362  [Context] index coordinator started (watching /mnt/h/test/ARTEX, 93 dirs)
18:12:43.834  [Context] Background index refresh finished: total_files=667, new_blobs=0, removed_blobs=0
18:12:44.701  [Context] semantic engine updated in background: full rebuild, 639 files in 4.336446783s
               ↑ 引擎建好了，写进共享单例

18:12:56.751  CodebaseSearch(path=/mnt/h/test/ARTEX, query="repeater replay request send draft parse HTTP")
18:12:58.321  → "The cross-file search index for '/mnt/h/test/ARTEX' is still building in the background"
               ↑ 引擎 12 秒前就建好了，工具却读不到
```

`full rebuild, 639 files in 4.34s` 这行是决定性的：**watcher 侧的索引构建完全正常**。事故不是「建不出来」，是「建好了读不到」。

### 2.3 为什么原总结的四个根因都判错了

原总结的推断链是：模型看到 "still building" → 说明索引在构建 → 于是归因到「显式调用不等待」「文案邀请重试」「截断不落地」「root 归属」。

但**「still building」这句话恰恰是分裂缓存的症状，不是索引慢的症状**。索引当时已经建完 12 秒了。从症状反推「索引还在建」是这次归因的根本偏差。

逐条核实：

| 原根因 | 核实方法 | 结果 |
|---|---|---|
| **A** 显式调用路径无等待 | 读 `agent_tool_process.rs:987` 与 `codebase_search.rs` 调用链 | ✅ 属实（只有自动触发路径等）。但**不是致因** |
| **B** Warming 文案邀请重试 | 读旧文案 | ✅ 属实。但**不是致因**——它是症状的放大器，不是症状本身 |
| **C** 截断时引擎不落地 | `index.json` 667 条 / 实测 sum size 7,544,754 B = **7.19 MB** < 12 MB 预算；`grep -c truncat` = **0** | ❌ **未触发**。ARTEX 的 639 files 是**正常**全量构建，不是截断 |
| **D** 查询 root ≠ watcher root | `canonicalize("/mnt/h/test/ARTEX")` = `/mnt/h/test/ARTEX`（恒等） | ❌ **未触发**。ARTEX 路径下两边本来就相等 |

C 尤其值得注意：`index.json` 里 667 条中有 29 个文件未进索引，但那 29 个全是 `.png` / `.sqlite` / `data/` 下的产物（被 `.gitignore` 排除或超出白名单），**不是**预算耗尽。全部 667 条记录的 size 之和 7.19 MB，远低于 12 MB。

### 2.4 旁证：mtime 令牌是好的

ARTEX 的 `.star/context/index.json` mtime = `2026-11-04 15:50:53`，而 watcher 建引擎是 `18:12:44` —— **索引文件在查询期间没有被重写**。这排除了「mtime churn 导致 `get_engine` 永不命中」这一类假设：令牌正常，唯一的问题是谁在读它。

---

## 3. 已实施的修复

### 3.1 核心修复（根因 E）

**`src/core/context/engine.rs`** —— `ContextEngine` 持有进程级共享单例，而不是独立实例：

```rust
// 以前：search_cache: Arc::new(SearchEngineCacheManager::new()),
search_cache: super::search_cache::shared_search_cache(),
```

这样进程里**只有一个**语义缓存：watcher 写它、工具读它、ContextEngine 也指向它。分裂在结构上不再可能发生。

**`src/agent/agent_core.rs`** —— 补注接线原因（`with_cache` 会覆盖 bootstrap 的工具，传入的必须是共享单例）。

### 3.2 配套修复（保留原总结的四处改动，但定位为「加固」而非「治本」）

| 文件 | 改动 | 定位 |
|---|---|---|
| `codebase_search.rs` `run_codebase_search()` | 显式调用也等索引 | **加固**。E 修好后冷启动本就不再进这条路径 |
| `codebase_search.rs` Warming 分支 | 文案改真话（「重跑无用」+ 指向 Grep / ProjectMap） | **加固**。B 的死循环指令确实该去掉 |
| `watcher.rs` `run_incremental_rebuild()` | 截断续扫 `skip_idle_backoff` 插队 | **加固**。C 未触发，但插队逻辑本身是对的 |
| `codebase_search.rs` `resolve_engine()` + `watcher.rs` `rebuild_root()` | 非托管 root 走同步构建 | **加固**。D 未触发 |

### 3.3 等待逻辑改三态（本次新增）

原先的等待只看「引擎在不在」，于是「磁盘变了」也会白等 8 秒。现在按 `resolve_engine` 的同一套判据分三态：

| 状态 | 判据 | 行为 |
|---|---|---|
| `Fresh` | 缓存里有引擎，且 mtime 与 `index.json` 一致 | **不等**，立刻检索 |
| `Stale` | 缓存里有引擎，mtime 对不上 | **不等**，用旧引擎回答（serve-stale-while-rebuild 的本意） |
| `Absent` | 缓存里压根没这个 root 的引擎 | 等（默认 8s，`STAR_CODEBASE_SEARCH_WARMUP_WAIT_MS`） |

判据与工具读的对象是**同一份缓存** —— 这是关键：修好 E 之后，等的就是工具真正会读的那一份。

新增 API：

```rust
pub enum EnginePresence { Fresh, Stale, Absent }
pub fn root_engine_presence(cache: &Arc<SearchEngineCacheManager>, root: &Path) -> EnginePresence
pub async fn wait_for_index_ready_in(cache: &Arc<SearchEngineCacheManager>, root: &Path, timeout: Duration) -> bool
```

### 3.4 顺带修掉：`rebuild_root()` 的路径比较

`rebuild_root()` 原来返回裸 `project_root`（可能是相对路径或带 symlink 的形式），而 `resolve_engine` 拿 canonical 路径去比 —— 两边形式不同就永远不相等。现在 `rebuild_root()` 也做 canonicalize。

**注意**：在 ARTEX 上 `canonicalize` 是恒等变换，所以这**不是**本次致因，属于防御性修复。原总结的根因 D 在这台机器上不成立。

### 3.5 顺带修掉：一个永远匹配不上的死标记

原总结在 `src/agent/tool_helpers.rs` 的 `COLD_MARKERS` 里加了 `"no hits —"`（**小写 n**）。但工具的实际文案是 `"No hits — ..."`（**大写 N**），`contains` 大小写敏感 —— 那个标记**永远匹配不到任何东西**，等于没加。

同时旧标记 `"no hits this turn"` 也是小写，而文案是 `"No hits this turn"`（大写 N）—— 同样失效。

已改为：

```rust
const COLD_MARKERS: &[&str] = &[
    "is still building",
    "is building in the background",
    "No hits this turn",
    "No hits —",
];
```

---

## 4. 验证

### 4.1 ARTEX 生产路径端到端（新增探针，真实仓库）

```bash
STAR_DIAG_ROOT=/mnt/h/test/ARTEX \
STAR_DIAG_QUERY="repeater replay request send draft parse HTTP" \
cargo test --release --lib -- artix_e2e_probe --ignored --nocapture
```

实测输出：

```text
[1] index_project: total_files=667 new_blobs=0 removed=0 truncated=false
[1] watcher rebuild outcome: FullRebuild { files: 639 }
[2] same object as shared cache? true          ← 接线修好了
[2] engine presence (tool's view): Fresh      ← 工具看得到引擎
[3] contains 'still building'? false | contains 'No hits'? false
      Semantic Search Results for 'repeater replay request send draft parse HTTP' (Top 10)
      Root: /mnt/h/test/ARTEX
      File: server/repeater.go (Score: 348.30, Lines: 61-234)
      Context: func (s *Server) sendRepeater(w http.ResponseWriter, r *http.Request) {
[OK] ARTEX 生产路径端到端通过
```

事故当时的查询，现在直接命中 `server/repeater.go:61-234` —— 正是模型反复找的那个文件。

探针刻意走**生产接线**三段（watcher 写共享单例 → 工具从 `ContextEngine` 那份读 → 完整检索路径），而不是直接用共享单例 —— 否则测不到真正出问题的那段接线。

### 4.2 新增回归测试（6 条，均已验证「去掉修复就变红」）

| 测试 | 锁住什么 |
|---|---|
| `the_context_engine_shares_the_process_wide_search_cache` | ContextEngine 必须持有共享单例（回退 `engine.rs` → FAILED ✓） |
| `the_tool_and_the_context_engine_read_one_cache` | 工具与 ContextEngine 读同一个对象 |
| `the_watcher_and_the_query_path_share_one_engine` | 端到端接线（回退 → FAILED ✓） |
| `an_engine_written_by_the_watcher_is_visible_to_the_query_path` | watcher 写的引擎查询路径能看到 |
| `an_explicit_call_never_tells_the_model_to_retry` | 绝不再出现「重跑一次」死循环指令 |
| `the_cold_markers_match_the_tools_actual_wording` | 标记与工具实际文案逐字对得上（含大小写） |

### 4.3 门禁

```text
cargo check --workspace --all-targets --locked   → exit 0
cargo fmt --all -- --check                       → clean
cargo clippy --workspace --all-targets           → 改动文件上零警告
cargo test --workspace --lib -- --test-threads=1 → 1934 passed / 0 failed / 3 ignored
```

> 关于「基线 37 failed」：那份基线是 2026-11-04 的产物，失败集中在 `commands/agents/hooks/permissions`，原因是测试环境的用户目录/profile 不一致。本机复跑**未复现**（0 failed）。这类失败与本次改动无关。

---

## 5. 仍未解决的两项（原总结的 §4.3 / §4.4）

### 5.1 根因 C 未彻底解决 —— 截断时引擎仍完全不落地

`src/core/context/watcher.rs:654`：

```rust
let cache = if truncated { None } else { coord.engine_cache.lock().clone() };
```

索引被 12 MB 预算截断时，引擎一个字节都不写进 cache，查询侧永远 cache miss。

当前只靠「续扫插队」让它更快收敛。**若仓库索引内容持续超过 12 MB，续扫仍可能反复截断。**

彻底解法是让部分索引也喂引擎，但要重新权衡原注释里的顾虑：

> 「开头两次连续 FullRebuild」（11.9s + 14.2s）—— 半截数据 patch 一轮还常常撞 patch 上限直接全量一次。

**本次未改**。ARTEX（7.19 MB）未触发截断，没有真实样本支撑改动方向。

### 5.2 UI 渲染不一致 —— 已定位，与原总结的猜测不同

原总结猜测是「疑似走了字面兜底有命中分支 + 展开视图，未深查」。**实际定位结果**：

- `tool_render.rs:865` 的判据 `text.contains("still building")` **是对的**。
- CodebaseSearch 默认是**折叠态**（`is_tool_expanded`：`is_edit_tool ^ in_expanded` → `false ^ false` = `false`），所以折叠态**会**正确显示 "building index"，已有测试 `codebase_search_collapsed_says_when_the_index_is_building` 覆盖。
- 截图里显示原始文本，说明当时该条目处于**展开态**（用户按了 Tab）—— 展开态走 `render_tool_result_text` 的通用路径，本来就渲染完整正文。
- `collapsed_group.rs:216` 的 `render_tool_result_sub_entry` 直接渲染 `entry.content`（截断到 1200 字符），没有工具专属摘要。但 `auto_collapse_tool_calls` **零调用者**（死代码），不是截图路径。

**结论：这不是 bug，是展开态的预期行为。** 本次未改动 UI 代码。

---

## 6. 经验教训

1. **「still building」是症状，不是诊断。** 看到这句话就推断「索引还在构建」，正好跳过了「索引已经建完但读不到」这个更简单的可能。事故现场恰好有日志（`full rebuild, 639 files`），先看日志再下结论，能省掉一整轮错误的根因分析。

2. **接线正确性需要测试锁住，不能靠读代码。** `shared_search_cache()` 早就存在（2026-10-26 的 `d203ea0` 就引入了），但 `ContextEngine` 一直持有自己的实例 —— 修复「watcher 与工具不是同一个对象」的那次提交，只改了 `agent_core.rs` 的一半（watcher 侧），没改另一半（ContextEngine 侧）。

3. **进程级单例必须真的只有一个创建点。** `SearchEngineCacheManager::new()` 在**生产代码**里原本有 **2 处**：`engine.rs:103`（ContextEngine）与 `search_cache.rs:87`（共享单例本身）。多一个创建点就多一次分裂的可能。修法不是「记得用对的那个」，而是「让另一个不存在」。

---

## 7. 关键代码位置速查（修复后）

```text
src/core/context/engine.rs:129          ← 核心修复：持有共享单例
src/agent/agent_core.rs:273             ← with_cache 接线 + 注释
src/core/tools/codebase_search.rs:476   ← run_codebase_search()：三态等待
src/core/tools/codebase_search.rs:1094  ← resolve_engine()
src/core/tools/codebase_search.rs:1217  ← Warming 分支文案
src/core/tools/codebase_search.rs:1619  ← wait_for_index_ready_in()
src/core/tools/codebase_search.rs:1661  ← root_engine_presence()
src/core/context/watcher.rs:823         ← rebuild_root()（canonicalize）
src/agent/tool_helpers.rs:33            ← COLD_MARKERS（大小写已修）
```

## 8. 外部参考

本次核实用到的公开资料：

- Zoekt 分片机制：索引按 ~100 MiB 分片，超限就 flush 成新分片，查询时各分片独立检索 —— <https://github.com/sourcegraph/zoekt>
- ripgrepx 的 trigram 索引原则：**宁可过近似，绝不漏近似**（over-approximate 而非 under-approximate），未索引区域回退到普通扫描 —— <https://github.com/igorgatis/ripgrepx/blob/main/docs/indexing.md>
- Tantivy 的分段提交：分批 commit 的可见性 —— <https://github.com/quickwit-oss/tantivy/blob/main/src/indexer/index_writer.rs>

这些资料支持 §5.1 的方向判断（部分索引可服务 + 未覆盖区域回退扫描），但**不足以支持直接改动** —— StarCode 的引擎是自研 BM25 结构，不是分片索引，改动成本与收益需要单独评估。

> 检索工具（exa MCP）在本文档成稿时已不可用（`fetch failed`）；以上三条来自本次会话早些时候实际取回的检索结果，结论引自其正文/高亮。若要据此动工，建议重新取回原文逐条复核。
