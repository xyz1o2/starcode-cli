# StarLens 设计方案：三层代码认知，重构 codebase_search

> 目标读者：starcode-cli 维护者。本文把"codebase_search 触发少、语义不准"的问题，
> 落成一套可落地的三层架构，核心是引入 **StarLens** 代码认知层，并明确 codebase_search 与
> ProjectMap 的去留。全程**不引入向量库 / embedding**。
>
> 命名说明：StarLens 是本项目自有子系统名。其认知 entry 的四元素设计（职责/关系/接口/信号）
> **概念借鉴**了公开规范 AOCI（`aoci-spec/aoci-code`）的 FRAS，但不沿用其项目名与实现。

## 1. 背景与目标

现状问题（已核实）：
- `codebase_search` 的 "ACE-POWERED/semantic" 是**词法 TF-IDF**，对真正的概念查询（词汇不匹配）召回弱。
- 自动触发开关 `STAR_ENABLE_AUTO_CODEBASE_SEARCH` 默认 `false`，且触发谓词**根本不读提问内容**，
  于是这套机制唯一的实际状态就是"永远关着" → 触发少 + 体验差。
- `ProjectMap`（`project_map.rs`）与 `codebase_search` 在"理解架构"的定位上撞车。
- **（2026-10-14 定位）"何时用哪个工具"没有单一决策点。** 这句话被手抄在四处
  （`system-prompt-tool-guidance.md` 的表格+Workflow、`tool-description-codebase_search.md`、
  `code_base_graph.md` 的 `NOT for`、`tool-description-projectmap.md` 的 `NOT for`），四处互不校验。
  最致命的一次漂移：目录表写 `Search **by meaning** | CodebaseSearch`，而工具自己的 schema
  描述写 `Lexical, **NOT** meaning-based — exact strings use Grep`。模型在同一段 system prompt 里
  同时被告知两件相反的事，裁决权归**离调用点最近**的那份（schema），于是它选 Grep，
  而 bundle 里 `tool-description-grep.md` 还挂着一条无条件的 `ALWAYS use Grep for search tasks`。
  **结果是：`codebase_search` 不是"触发条件太严"，而是架构上从来没有过触发器。**

设计目标：**快、准、省 token、无向量库、随代码在 Git 里演进**。做法依据是业界同类工具
（codemap/agentmap/L-SDF/CodeGraph）与 AOCI 规范收敛出的"解析器出结构、cheap model 出语义、
整仓几百行开工前读完"的模式。

非目标（本期不做）：
- 不做 embedding / 向量检索 / 向量存储。
- 不做重型事务治理（CAS、Ledger、恢复事务、策略指纹）。先拿到"快准省"，治理推后。
- **不做数据库认知**（`db://` 对象推到 P5）；本期只认源码文件（tree-sitter 可解析的那批）。
- 不做跨仓 / 多 repo。

## 2. 架构总览：三层

| 层 | 何时用 | 职责 | 承载 | 复用的现有代码 |
|---|---|---|---|---|
| **L0 认知层** | 开工前读，无 query | 意图 F / 关系 R / 接口 A / 信号 S，整仓几百行 | **StarLens**（吸收 ProjectMap） | `project_context.rs` 注入、`project_map.rs` 遍历、`chunking.rs` tree-sitter |
| **L1 结构层** | 干活时，精确 | who-calls / callees / blast-radius | **CodeGraph** | `codebase_search.rs:1359` `build_call_graph`、`1425` `trace_call_chain`、`symbol.rs` |
| **L2 词法层** | 干活时，精确 token | 精确串/符号 | **Grep + 瘦身 codebase_search** | 现有 TF-IDF 倒排 `search_engine.rs` |

核心判断：**codebase_search 的"语义"职责上移到 L0（authored，不是模糊匹配），结构职责下沉到 L1。
它本体退化为"带排序的词法取片"，或并入 L0 的 select 步骤。**

## 3. 认知 entry schema（四元素，概念借鉴 AOCI 的 FRAS）

每个受管对象（源文件 / 数据表）一条 entry = **符号标签**（结构事实，解析器出）+ **语义四元素**
（F/R/A/S，cheap model 出）。**entry 相互独立**：改一个文件只重算它那条，R 用文件名级引用、
不绑版本，故未变文件的 entry 仍有效（O(1)/改动文件）。

```rust
/// 一条 StarLens 认知 entry。磁盘上以紧凑单行文本存储，这里是解析后的内存态。
pub struct LensEntry {
    /// 受管对象路径（源文件相对路径；`db://schema.table` 预留给 P5，本期不生成）。
    pub object: String,
    /// 当前绑定的源内容指纹，漂移检测与 source-binding 用。
    pub source_sha256: String,
    /// 认知角色：决定是否进全认知索引。
    pub role: ScopeRole, // Index | Observe | Exclude
    // ── 符号标签：结构坐标，100% 来自 tree-sitter，永不交给模型 ──
    pub lang: String,            // rust / python / sql ...
    pub symbols: Vec<String>,    // 公开符号名（fn/struct/trait/class...）
    pub imports: Vec<String>,    // 依赖的模块/文件（解析得到）
    // ── 语义四元素：模糊字段，仅此四项交给 cheap model ──
    pub f_role: String,          // F: 一句话业务职责/意图
    pub r_related: Vec<String>,  // R: 主要关联文件（文件名级）
    pub a_api: Vec<String>,      // A: 对外 API/接口签名
    pub s_signals: Vec<String>,  // S: 高熵实现细节的密集关键词（限流/回退/加密...）
}

pub enum ScopeRole { Index, Observe, Exclude }
// token 占用是派生值，用方法 estimate_tokens()（chars/4）按需算，不作字段存储，
// 避免字段与内容脱节（已在实现中踩过：mutate 后忘记重算会导致 Eq 不一致）。
// band 策略（core<200、peripheral<50）在生成管线里据此校验，超限标 blocked 要求压缩。
```

设计要点：
- **解析器/模型分工是准确性的根**：`symbols`/`imports`/`a_api` 的签名部分来自 `chunking.rs` +
  `symbol.rs`，永远正确；只有 `f_role`/`s_signals`（以及 `r_related` 的归纳）来自模型，
  "模型只会在意思上出错，不会在结构上出错"。
- **token band**：core 文件目标 <200 token、peripheral <50，整仓几百行；超限的 entry 标 `blocked`
  要求重写更紧凑，防认知膨胀。
- 磁盘格式用紧凑单行（类似 L-SDF 的 sigil 锚），便于 LLM 单遍扫描、也便于 git diff。

一条真实 entry 长这样（以本仓 `src/runtime/messages.rs` 为例，为便于阅读按字段换行；
实际存储可进一步压紧，符号标签 + F/R/A/S 构成一个 entry）：

```
@src/runtime/messages.rs | rs | Index | #a1b2c3d4
F: UI↔Agent 消息协议——定义两半之间所有合法交互
R: ui/services/worker.rs, ui/services/stream.rs, runtime/agent_runtime.rs
A: enum AgentRequest{SendMessage,Abort,SetModel,GenerateNote,PluginOp}; enum StreamMessage{TextDelta,ToolCalls,ToolResult,AgentTaskUpdate,Done}
S: mpsc, cap-100, steering-queue, abort-beats-queued, 8ms-batch
sym: AgentRequest, StreamMessage, AgentTaskUpdate
```

读法：首行是**符号标签**（路径 | 语言 | 角色 | 源指纹前 8 位）——结构事实，解析器出；
`F/R/A/S` 四行是语义，cheap model 出；`sym` 回到解析器的公开符号。模型单遍读完就知道
"这文件是协议中枢、改它牵动 worker/stream/agent_runtime、关键约束是容量 100 和中断优先"。

## 4. 磁盘布局（`.star/lens/`）

路径一律经 `Storage`（`storage.rs`，`STAR_DIR=".star"`）构造，不手拼。

```
.star/lens/
  lens.txt           # Whole-Index 根：编码规则 + 全部 Index 角色 entry 的紧凑文本（git 跟踪、开工前读的就是它）
  baseline.json      # 每个受管对象的 source_sha256 基线，漂移检测的权威参照（git 跟踪）
  scope.json         # Managed Scope 规则：哪些路径是 Index/Observe/Exclude（git 跟踪）
  drafts/            # 生成中的候选、断点续作（.gitignore）
```

- `lens.txt` 与 `baseline.json`、`scope.json` **进 Git**，随代码演进、可 diff/review/回滚。
- `drafts/` 是运行态，`.gitignore` 掉。
- 换行归一化：写 `.gitattributes` `* text=auto eol=lf`，避免 Windows autocrlf 把基线搅成字节漂移。

## 5. 生成管线（parser 出结构 + cheap model 出语义）

挂在 `src/core/context/` 下新增 `lens/` 子模块，复用现有索引基础设施：

```
对一个受管文件：
 1. tree-sitter 解析（复用 chunking.rs / symbol.rs）
      → lang, symbols, imports, a_api 的签名骨架   【确定性，永远正确】
 2. 取文件 + 第1步骨架，发一次 cheap model 调用
      → f_role, s_signals, r_related 归纳           【模糊，仅此交给模型】
 3. 组装 LensEntry，算 token_cost，band 校验
 4. 写入 drafts/ → 校验 source_sha256 绑定 → 原子写 lens.txt + 更新 baseline.json
```

- cheap model 走你们现有 provider（`provider_resolution.rs`），用一个便宜快速的模型别名；
  新增 `STAR_LENS_MODEL` 覆盖。只写 F/S/R，不写代码事实 → 便宜、可大批量。
- 增量：复用 `codebase_search.rs` 的 `plan_patch_ops`(719)/`apply_patch_ops`(797) 补丁思路 +
  `search_cache` 的 mtime 令牌，一个文件变只重算它那条 entry。全量仅在首次或 scope 变更时。
- **secret 安全红线（硬规则）**：文件内容会发给 cheap model，所以 scope 评估必须在**调用模型之前**
  把密钥类文件挡在 Index/Observe 之外——`.env`、凭据、`*.pem/key`、`.star/` 内的 token 等，
  一律强制 Exclude，且不可被 scope profile 覆盖。宁可漏覆盖，不可把 secret 送出。
- **成本量级**：P1 对每个 Index 文件各发一次 cheap model 调用。`core` profile 约数百文件，
  按 codemap 的经验（~2700 文件 \$0.5–3）量级在**几角到几元**，且 mtime+hash 缓存后只有改动文件重算，
  日常增量近乎零成本。首次全量可后台跑、断点续作（`drafts/`）。

## 6. 漂移检测（接 watcher，诚实地告诉模型）

权威参照是 `baseline.json` 的 `source_sha256`。对照磁盘现状，分五类（分类口径对标 AOCI 规范）：

| 类别 | 含义 | 处置 |
|---|---|---|
| `Stale` | 文件 hash 变了，entry 还是旧的 | 标脏，下次增量重算该 entry |
| `Missing` | baseline 有、磁盘无 | 标待删 |
| `Orphan` | entry 有、已不在 Managed Scope | 清理 |
| `Unbaselined` | 磁盘有、baseline 无 | 入 scope 则补算 |
| 行尾差异 | 仅 CRLF/LF 变化 | 报告但不阻断 |

- **复用 `watcher.rs`**：后台监听文件事件，debounce 后把受影响对象标 `Stale`，触发增量重算。
- **复用"serve-stale-while-rebuild"**（`codebase_search.rs` 的 `resolve_engine` 已有同款三态）：
  重算期间仍可读旧 entry，但——
- **漂移要对模型可见**（抄 CodeGraph 的 per-file staleness banner）：当 L0 注入或 L1 查询涉及一个
  `Stale` 对象时，结果顶部挂 `⚠️ <file> 的认知可能过期，建议直接 Read`。宁可少、不可错——
  撒谎的地图比没有地图更糟。

## 7. 开工前注入（L0 怎么到模型眼前）

走 `project_context.rs` 同款通道：`SessionStart` / 新会话首轮，把 `lens.txt` 注入 system prompt。

- 预算：沿用 `project_context_max_chars()` 式的上限；分 R/S band 控制，超限优先保 Index 角色核心文件。
- 刷新策略：模型自行决定是否重读——长会话被压缩、系统大改、进入重要阶段时重载 Overview；
  不必每轮机械重读。StarLens 提供 refresh 阈值/checkpoint，由模型按任务判断。
- 与 `CONTEXT_FILE_CANDIDATES`（STAR.md/CLAUDE.md…）并存：StarLens 是机器生成的"系统认知"，
  项目 context 文件是人写的"规则"，各占一段。
- **吃 prompt cache、不进压缩**：L0 作为 system prompt 的稳定前缀注入，天然命中 provider 的
  prompt caching（这正是 L-SDF 省 token 的来源）；并应在 `CompactManager` 里**排除出压缩范围**——
  它是认知锚，被压掉就等于每次压缩后模型"失忆"。注入体量计入预算，故 §3 的 token band 是硬约束。

## 8. 接入 Agent loop（根治"触发少"）

两处挂点，都是现成机制：

1. **PreToolUse nudge**（已确认有 `ManagedHookEvent::PreToolUse`，`hooks/store.rs:33`）：
   当 agent 要 `Grep` 且 query 像"找依赖/谁调用/组件用法/可复用性"时，注入一句非阻断提示——
   "先读 L0 认知 / 用 CodeGraph(L1)"。抄 agentmap 的 `PreToolUse(Grep)` 钩子，**只提示不拦截**，
   对裸字符串/样式类扫描保持沉默。这是把工具"用起来"的关键，而非只是注册了它。
2. **auto-trigger 收敛**：`triggers.rs` 里让 L0 预取（首轮注入 StarLens）替代现在默认关闭的
   `STAR_ENABLE_AUTO_CODEBASE_SEARCH`。L0 到位后，模型开工即被定向，不再需要盲触发语义搜索。

教训（CodeGraph）：**一个强工具比一堆窄工具更能引导 agent，少选错**。所以 §9 的方向是
**合并、不新增**——不再多一个"理解代码"的工具，而是把 ProjectMap 吸收进 L0、codebase_search 降级。

## 9. 工具去留：最终职责表

| 工具 | 处置 | 新描述要点（消除 architecture 撞车） |
|---|---|---|
| **ProjectMap** | **被 StarLens 吸收**。保留遍历/符号骨架作为 StarLens 生成管线的解析层，不再作为独立"理解架构"工具暴露 | — |
| **StarLens(新)** | 新增 L0 工具 + 开工前注入。暴露面尽量窄（读 Overview / 查单 entry / 触发维护） | "系统级持久认知：职责、关系、约束、Scope、版本身份。开工前读。" |
| **codebase_search** | **降级**：去掉 "ACE-POWERED/semantic" 话术，改为"带排序的词法取片，擅长关键词+结构定位" | "Ranked lexical fetch over the index. For keyword+structural locate; NOT conceptual Q&A (use StarLens) and NOT exact strings (use Grep)." |
| **CodeGraph(L1)** | 把现有 `build_call_graph`/`trace_call_chain` 提升为一等公民工具 | "Precise caller/callee/impact over the call graph." |
| **Grep / Glob / Read** | 不变 | 精确串 / 文件名 / 读已知文件 |

描述词改哪里：编辑 `src/core/prompts/system-prompts/tool-description-codebase_search.md`
（body + frontmatter `description:`），映射见 `tool_descriptions.rs:49`。

> **待决策（P4 落地前拍板）**：codebase_search 的最终归宿有两条路，本文暂时都留着——
> (A) 保留为 L2 独立"带排序的词法取片"工具；(B) 彻底并入 L0 的 select 步骤（模型读 lens.txt →
> 直接选文件，不再暴露独立工具）。建议先走 (A)（改动小、可回退），P4 跑一段看模型是否还用它，
> 用量低再转 (B)。此决策定了再改 §2 的表述，别两处各说一次。

## 10. 分期落地

- **P0（止血，当天，零 ML）**：改 codebase_search 的 description 消除错配；
  打开 L0 的首轮注入（即便 L0 暂时只有 ProjectMap 式结构骨架）。
- **P1（StarLens MVP，主修）**：entry schema + 生成管线（parser 骨架 + cheap model 写 F/S）+
  `.star/lens/` 布局 + 注入。先只覆盖 Index 角色核心子系统（见 §11 范围）。
- **P2（新鲜度）**：接 `watcher.rs` 漂移检测 + staleness banner + 增量重算。
- **P3（loop 接入）**：PreToolUse(Grep) nudge；auto-trigger 收敛到 L0 预取。
- **P4（结构层）**：把 CodeGraph 提升为显式工具；codebase_search 正式降级/并入。
- **P5（可选，治理）**：按需补 baseline CAS、Ledger 审计、scope 变更事务——仅在产品化时。

## 11. 范围、风险、配置

**首批 Managed Scope（建议）**：不要一上来全仓每文件一行（190k 行、几百文件，token 吃紧且难维护）。
先圈 Index 角色 = 核心子系统（`src/runtime/`、`src/agent/`、`src/core/tools|context|config/`、
`src/ui/state|services/`）+ 关键配置；其余设 Observe（只存 hash 测漂移）或 Exclude，并显式记为负空间。

**最大风险 = 漂移与信任**：撒谎的地图比没地图更糟。工程重心不是 entry 格式，而是
§6 的"检测→标脏→重算→对模型可见"闭环。P2 没做扎实前，L0 只能当弱提示，不能让模型据以做破坏性改动。

**配置（新增 `STAR_LENS_*`，默认关，渐进开启）**：
- `STAR_LENS_ENABLED`（默认 `false`）：总开关。
- `STAR_LENS_MODEL`：生成 F/S 用的 cheap model 别名。
- `STAR_LENS_MAX_CHARS`：注入上限。
- `STAR_LENS_SCOPE_PROFILE`：`core`（默认）/ `full`。

## 实现状态（2026-10-11）

已落地并通过测试（26 个 lens 单测 + 命令分类守卫，全绿；新增代码零 clippy 警告）：

- **P0** codebase_search 描述消除错配：`tool-description-codebase_search.md` + `codebase_search.rs` 的 `description()`/query schema 改为诚实的"ranked lexical"。
  > ⚠️ **P0 半途而废，且做反了（2026-10-14 修正）**：它只改了被指向的那一份描述，
  > 另外三份（目录表 / code_graph / projectmap）一个字没动，于是"错配"没消除、被制造了。
  > 正确做法见下面的 **P0'**。
- **P0' 路由契约收成单一事实源**（2026-10-14 落地，见 §14）：新增 `src/core/prompts/routing_matrix.rs`
  作为"某个活儿归谁"的**唯一**权威；目录表与 Workflow 段由它渲染（md 里的手写表格已删，
  换成 `{{ROUTING_MATRIX}}` / `{{TOOL_WORKFLOW}}` 占位符）；每个 `tool-description-*.md`
  在 frontmatter 里用 `routing: <slot.id>` 声明自己主张的槽，测试强制"一个槽有且只有一个主张者"。
  另把 Grep 的 `ALWAYS use Grep for search tasks` 收窄为 `ALWAYS use Grep for **exact** content search`
  —— 它原本在指令强度上压过所有竞争者。
- **P1** StarLens 库核心：`src/core/context/lens.rs`（模型/紧凑格式）、`lens_scope.rs`（Managed Scope + secret 红线）、`lens_extract.rs`（tree-sitter 结构抽取）、`lens_build.rs`（全仓组装 + `.star/lens/` 落盘 + baseline）、`lens_enrich.rs`（prompt/parse/apply + 实时 `StarClient` 适配 `enrich_entry_with_client`）、`lens_service.rs`（门面 + `STAR_LENS_*` 门控 + `rebuild_enriched`）。
- **P2** 漂移检测 + 增量闭环：`lens_drift.rs`（Stale/Missing/Orphan/Unbaselined）+ `lens_service::update`（只重算受影响对象并落盘，完成"检测→标脏→重算"闭环）。
- **P3** agent 接线（全部 `STAR_LENS_ENABLED` 门控，默认关，不碰默认行为）：
  - 开工前注入 overview：`prompt_builder.rs` 的 `semi_static_parts` 处；
  - `PreToolUse(Grep)` nudge：`agent_tool_process.rs` 的 `Ok(())` 分支，每批一次；
  - `/lens [build [--enrich]|status|update]` 命令：`parity.rs` + `mod.rs` 分支 + `system.rs` ALL_COMMANDS（category `Tools`）。
  - **语义增强已接线**：`AgentRequest::LensRebuild{message_id, enrich}`（`messages.rs`）→ `control_requests.rs` 用 `StarAgent::get_client()` 跑 `rebuild_enriched`，结果经 `StreamMessage` 回显；`session.rs` 对流式回合内到达的该请求安全跳过并回执。`/lens build --enrich` 触发真实 F/R/S。

- **P4** CodeGraph 提升为一等公民工具：`code_graph_tool.rs`（`CodeGraph` 工具，`symbol`→who-calls/callees/调用链，底层复用 `trace_call_chain`，只读、spawn_blocking）；注册于 `agent_runtime.rs`、入 `ToolName` 枚举（`constants.rs`）+ `CORE_TOOL_NAMES`、带 `tool-description-code_graph.md`。

待做：P5（重型治理：baseline CAS / Ledger 审计 / scope 变更事务，按需）。

## 11.1 触发链路修复（2026-10-14，与 P0' 同批落地）

文案收成单一事实源只能解决"模型读到矛盾指令"。**另一半是：模型不说话时谁来替它调。**
那条链路此前是四重死法，逐条修掉：

| 原状 | 现在 |
|---|---|
| `should_trigger_codebase_search_with_flag(_user_input, _current_content, …)` —— 两个提问参数都带下划线，谓词压根不读提问。真表达式只有 `enabled && !attempted && tool_present`，"概念性查询检测"是一句空话 | `score_rank_locate_route(user_input)` 真判分。**硬否决 + 分档**：先排除点名别的工具 / 精确标识符形状 / 概念问答（词法引擎结构上答不了，触发即零命中），剩下的按证据强度落到 75（有扫面措辞）或 62（问句 + 功能动词）两档；加成只用于同档排序。阈值 `STARCODE_ROUTE_SCORE_MIN`（默认 60） |
| `sort_by(\|a,b\| b.kind.cmp(&a.kind).then(b.score.cmp(&a.score)))` —— kind 是固定枚举且每个 kind 最多 push 一次，**`score` 永远打不破平局**，那一整套关键词打分是死代码 | 分数是**第一**排序键、跨 kind 可比（0..=100），kind 退化为同分兜底。`TriggerEval` 新增 `detail` 字段把判分依据写进 debug log |
| `STAR_ENABLE_AUTO_CODEBASE_SEARCH` 默认 `false` → 整套机制唯一实际状态是"永远关着"，下游 navigator/editor 梯（都依赖 `codebase_search_attempted`）全是死代码 | 默认**开**（`STAR_ENABLE_AUTO_CODEBASE_SEARCH=0` 仍是逃生门）。谓词会判分了，默认开才安全 |
| `build_codebase_search_tool_call(user_input, …)` 把**整句自然语言**塞给词法 TF-IDF 引擎，而 schema 要求 keywords；`codebase_search_attempted` 在**调用前**置位，于是冷启动那次 `Warming` 空结果把闸门永久关上，工具自己给的"稍后重跑"被自家 flag 当场否掉 | `extract_search_keywords()` 抽关键词（英文停用词 + CJK bigram 分词，保 `snake_case`/`::`/路径）；闸门改由 `tool_helpers::tool_result_is_usable()` **事后**判定，`Warming`/失败/空输出都不算"已搜过" |
| `ToolSequenceLearner`（186 行）导出但全仓零调用，且自带一张把 CodebaseSearch 排在第三顺位的关键词表 —— 一份从不生效的竞争性路由意见 | 删除 |

自动触发仍在**"模型有话但没调工具"**的分支里跑（`process_llm_response` 的 `else`）。这是有意的：
模型已经调了 Grep 就说明它做了选择，此时插队是噪音。要改的是"模型该选什么"，不是"抢模型的轮次"。

### 11.2 打分器返工：加法模型本身是漏判的（2026-10-14）

第一版判分写完就交付了，用真实措辞一跑才发现它**主要作用是漏判**：

```
-  where is retry handled          ← 基准20 + 扫面35 = 55，阈值 60 → 漏
-  where is auth                   ← 同上，连最简形式都进不来
-  all the places that build …     ← 标记词 "all places" 匹配不上 "all the places"
-  how does the watcher work       ← 同一类活儿，"which files touch config" 却过
```

根因是**加法模型 + 近阈值**：`where is retry handled` 是这个槽**最典型的措辞**，它离阈值只差 5 分。
标记词再加也救不回来——漏判的不是某条规则，是整个设计。

改成 **硬否决 + 分档**：否决项（点名别的工具 / 精确标识符 / 概念问答）在打分**之前**返回 `None`，
剩下的只落两档（75 / 62），档位离阈值都有富余。标记匹配改跑在 `normalize_for_markers()` 上（摘冠词），
"all the places" 这类被打断的措辞不再漏。`describe_rank_locate_route()` 对被否决的输入报
`veto: <原因>`，排障时能直接看到是哪一条拒的。

回归测试锁住这批真实措辞（`canonical_locate_phrasings_all_trigger`，17 条）以及它们的反面
（`weak_signals_do_not_trigger` 6 条、`conceptual_qa_is_vetoed_not_merely_low_scored` 5 条）。

## 12. 成功指标（怎么知道做成了）

前提是"快准省"，就得能量。固定一组任务，对比 StarLens 开/关：

| 指标 | 怎么测 | 目标方向 |
|---|---|---|
| 每任务探索类工具调用数（Grep/Glob/Read 轮次） | agent 日志计数 | 显著下降（同类工具报告 10–20× 少调用） |
| 每任务输入 token（含 prompt cache 后） | token 计数器 | 下降（L-SDF 对标 ~70%） |
| 首次定位正确率（file-localization） | 固定问题集人工判定命中 | 升高（AOCI 报到 97.67%，仅供方向参考） |
| codebase_search 误触发率 | 调用后被回退到 Grep 的占比 | 下降 |
| L0 漂移率 | `Stale` entry 占比随时间 | 低且可控（验证 §6 闭环有效） |

没有这组数，"快准省"只是口号。

> ⚠️ **这套评测基础设施已随 eval 体系删除（2026-10-13）。** 上表的"每任务工具调用数""token 计数""首次定位正确率"需要一套固定任务集来跑，而 `starcode eval`、`eval/tasks.json`、`src/agent/eval_*.rs` 都已被删除（理由见 `CLAUDE.md` 的 "The eval harness is gone"）。**要落地这张表，必须先重建评测基座** —— 否则 P1 之后的每一期都只能靠感觉判断有没有变快。这不是本文能顺手带过的缺口，是 StarLens 整条路线当前最大的阻塞项。

### 12.1 现在有什么、没有什么（2026-10-14 诚实交代）

本次 P0'/触发链路修复**没有**重建评测基座，因此：

**有了**
- **路由契约守卫**（`core::prompts::routing_matrix` 的 6 个测试）：钉死"一个职责槽有且只有一个主张者"、
  "没有描述文件能声明矩阵外的槽"、"槽的主管工具必须存在描述文件"。这类**文案漂移**从此让 CI 变红。
- **路由判分的判定集**（`tool_routing::triggers`，12 个测试）：约 40 条真实提问分四类——
  跨文件扫面必进、无问号的最简形式必进（17 条，锁 §11.2 那个漏判）、
  精确 token / 点名别的工具 / 概念问答必不进、证据不足的短陈述句必不进。
- **可观测性**：`[ACE_AUTO] selected trigger … detail=[…]` 与 `[ACE_AUTO] CodebaseSearch auto-run satisfied=…`
  两行 debug log，是当下唯一能回答"它为什么触发/为什么没触发"的出口。

**没有**
- **行为度量**：上面这些测试锁的是**契约**（这类问题归哪个工具），**不是**真实模型的路由行为。
  没有任何东西能告诉你"改了描述之后触发率是升是降"——那需要跑真实任务集。
- **A/B 对照**：阈值 60 与两档分值（75 / 62）是按"档位离阈值留富余"这个理由定的，
  **不是**用真实分布校准的。§11.2 已经证明过"拍脑袋定一个数"会出事（加法模型漏判最典型措辞），
  这两档同样可能偏松或偏紧——只是现在偏松偏紧都不会让常见问法掉出界。
- **中英文覆盖不对等**：标记词表里 `how does` / `where is` 等英文变体明显比中文细，
  中文只覆盖了"在哪/哪里/哪些/几个地方"。真实语料里没统计过各语言的措辞分布。

所以：**契约不会再漂了，但"漂得对不对"仍然要靠人判断**。要真正闭环，还是得重建评测基座。

## 13. 参考

- 认知层概念借鉴：AOCI 规范与论文 `github.com/aoci-spec/aoci-code`、arXiv `2605.02421`
  （FRAS 四元素、Managed Scope 角色、漂移分类）——仅借鉴设计思想，不沿用其项目名与实现。
- 无向量 map 同类：codemap(jonnonz1，parser/model 分工、no-vector)、agentmap(PreToolUse 钩子)、
  L-SDF(紧凑索引 + sync)、Aider repo-map(graph ranking)。
- 结构层：CodeGraph(colbymchenry，staleness banner / one-tool)、Lordymine/codegraph(类型检查级调用边)。
- 组合检索共识：Sourcegraph（keyword + structural + semantic 各司其职）。

