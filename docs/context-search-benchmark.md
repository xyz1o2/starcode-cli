# 代码检索对标：Augment Code × Windsurf FastContext

> 2026-10-26 快照。调研来源：Augment 官方文档 / 官方博客 / 官方开源 npm 包（`auggie`、`auggie-sdk`、`context-connectors`）反编译 + cAST 论文（arXiv:2506.15655，CMU × Augment）；Cognition 官方博客（SWE-grep）+ Devin Docs（Fast Context 原 Windsurf 文档已 307 到 docs.devin.ai）。
> 第三方推测均已标注，不作为设计依据。

这份文档回答三个问题：**我们差在哪**、**为什么差**、**按什么顺序补**。

---

## 0. 一句话结论

StarCode 的检索**不是缺向量**，而是三件事：

1. **已有的零件没接线** —— 调用图、符号索引、精确检索（`precise_retrieval`）全部零调用者；
2. **缺一路字面精确通道** —— ripgrep 通道已经写好，却只挂在错误兜底路径上；
3. **跨语言召回为零** —— 索引与查询两侧都没有 CJK 处理，中文提问对英文代码必然零命中。

Augment 与 Windsurf 的路线截然不同，但**对 StarCode 的启示是同一个**：先把「语义 + 字面」两路接起来，再谈别的。

---

## 1. 两家怎么做

### 1.1 Augment Code Context Engine

**架构**：本地采集 + 服务端索引/检索（`Context Engine` 是 Augment 托管服务，embedding 模型闭源）。

**检索流水线**（从官方 npm 包反编译还原）：

```
用户提问
  ├─ codebase-retrieval      ← 服务端一次调用返回全部 chunk（语义 + 字面）
  ├─ conversation-retrieval   ← 会话历史作为语料（Augment 独有）
  ├─ 客户端 LLM 重排          ← 只作用于会话 chunk，listwise 输出索引数组
  └─ 客户端扩写               ← 用本地 session 文件回填被截断的对话
```

**索引单位**：**整个文件**。`blobName = sha256(path + fileContent)`，内容寻址 —— 增量索引天然幂等。

**切分**：发生在**服务端**，官方从未公开 chunk 参数。可参考的是 cAST 论文（Augment 员工参与）：
tree-sitter 解析 AST → 递归「先切后合」（兄弟节点贪心合并、单节点超限则递归切子节点），按**非空白字符数**计大小，**无 overlap**。
论文实测：AST 边界切分相对固定行数切分，**RepoEval Recall@5 +1.8~+4.3，SWE-bench Pass@1 +2.67**。

**召回**：服务端做向量语义检索，**同时**返回 `matched_string`（字面命中证据）与 `score`。
查询端支持 `contains_string`（字面串约束）、`path`（路径子串过滤）、`max_chunks`（数量上限）——
**语义 + 字面 + 结构化过滤三路在一次调用里完成**。

> **BM25 / RRF / 融合权重：官方全未公开**（bundle 中 `bm25` 出现 0 次）。

**重排**：客户端用 **LLM listwise**（输出 JSON 索引数组），非 cross-encoder。
prompt 明确要求「**宁可包含，不可排除**」（"When in doubt, include the chunk"）。
默认模型 `gemini-3-0-flash-minimal`（feature flag 可覆盖）。4 条降级路径：无 JSON / 索引非法 / 空数组 / 异常，全部回退原顺序。

**可观测性**：`retrieval_tool_data` 遥测字段是一份现成规范：
召回/改写/重排/扩写各自耗时、预算、截断标记、改写后生成的查询数、重排前后 chunk 数对比。

**官方量化**：

| 场景 | 数字 |
|---|---|
| 100M LOC 量化 ANN | 内存 2GB→250MB，搜索 >2s→<200ms，与精确检索一致性 >99.9% |
| 代码搜索端到端 | 提速 >40% |
| PR 评审召回 | Precision 65% / Recall 55%（Claude Code 23% / 51%） |
| Auggie v2 vs Claude Code（SWE-bench Pro） | 成本 $1.27 vs $2.70，耗时 329.6s vs 408.6s |
| Context Engine MCP vs Claude Code | 代码量完成度提升 30–80% |

**跨文件关联**：官方口径是 "Relationship awareness"，**具体机制未公开**（调用图是否存在、如何构建均为未知）。

### 1.2 Windsurf FastContext

**最重要的澄清**：FastContext **不是索引系统**，是**一个用专门训练的模型做检索的子代理**。

Cognition 官方博客原文：*"SWE-grep and SWE-grep-mini, fast agentic models specialized in highly parallel context retrieval... These models now power Fast Context, a subagent."*

**它没有向量索引、没有 BM25、没有 RRF、没有 rerank。** 检索 = **受限轮数 + 高并行的 agentic 工具调用循环**，工具集被刻意收紧到 `grep` / `read` / `glob`（为跨平台一致性）。

**"fast" 来自三处叠加**：

| 来源 | 机制 |
|---|---|
| 少串行轮数 | 常规 agentic search 需 10–20 串行轮；FastContext 只用 **4 轮**（3 探索 + 1 作答） |
| 高并行 | 每轮最多 **8** 个并行工具调用；消融显示 4→8 并行把 6 轮压到 4 轮且性能不变 |
| 快推理 | Cerebras 部署，SWE-grep-mini **>2800 tok/s**（Haiku 4.5 的 20 倍） |

**精度控制不在检索层，在训练层**：奖励函数 = weighted F1（**β=0.5，precision 优先于 recall**）。
官方理由：*"polluting the context of the main agent was more detrimental than leaving some context out, as the agent is typically only a few searches away to recover any remaining context."*

> 这条取舍值得直接抄 —— **污染主 agent 上下文，比漏掉部分上下文更糟**。

**输出格式是刻意设计的**：返回「文件 + 行范围」的结构化清单，**不返回散文摘要**。
官方两条理由：*"A fast model summary can draw wrong conclusion and mislead the smart model."* / *"it is hard to grade free-form summaries"*（摘要没法做 RL 奖励）。

**它要解决的问题**（官方）：*"our agent trajectories were often spending >60% of their first turn just retrieving context."*

**对照**（Windsurf 内部的 M-Query RAG 索引，与 FastContext 是两条独立的线）：
语义块级 embedding + Lucene 风格关键词检索的 hybrid + 复杂 rerank（并行 LLM yes/no 判定 + intent 权重：文件是否打开、邻近目录、最近 commit、当前 ticket），指标 **Recall@50**。

---

## 2. StarCode 现状盘点

### 2.1 已有（且质量不差）

| 能力 | 位置 | 对标 |
|---|---|---|
| tree-sitter AST 分块 | `context/tree_sitter_chunker.rs` | ≈ cAST 论文方向（Augment 员工参与的那篇） |
| RRF 融合（K=60） | `context/fusion.rs` | 工业标准 hybrid 融合算法 |
| 启发式重排 | `context/reranker.rs` | 比 Augment 的纯 LLM 重排更便宜 |
| 调用图（双向） | `context/call_graph.rs` | Augment「Relationship awareness」未公开的那部分 |
| 符号索引 | `context/symbol.rs` | — |
| ripgrep 精确匹配 | `core/tools/ripgrep.rs` | **Augment 的 `matched_string` 那路** |
| 并行工具执行（并发 10） | `agent/tool_executor.rs` | > Windsurf 的 8 |
| 增量索引（hash 比对） | `context/incremental_index.rs`、`watcher.rs` | ≈ Augment 的 `find-missing → batch-upload → checkpoint` |
| serve-stale-while-rebuild | `context/search_cache.rs` | Augment 量化索引的「构建期用旧索引」同理 |

### 2.2 缺口（按严重度）

| # | 缺口 | 状态 | 证据 |
|---|---|---|---|
| **1** | **CJK 零处理** | ✅ 已修 | bigram + 单字切分；中文↔中文召回从 0 → 可用 |
| **2** | **调用图 / 符号索引没接线** | ⬜ 未做 | `call_graph.rs`、`symbol.rs` 只服务于独立的 `trace_call_chain` 工具 |
| **3** | **精确检索层零调用者** | ⬜ 未做 | `context/precise_retrieval.rs`、`integration.rs` 的 `for_edit` / `find_references` / `search_functions` 全无外部引用 |
| **4** | **字面通道只当兜底** | ✅ 已修 | ripgrep 提为并列召回路，能独立把结果带进候选集 |
| **5** | **两路 RRF 其实同源** | ✅ 已修 | 改成三路异构（expanded / exact / 字面） |
| **6** | **冷启动首查必空** | ✅ 已修 | 等索引就绪后框架自动补跑一次，不依赖模型自觉 |
| **7** | **中文标记词覆盖窄** | ✅ 已修 | 补「怎么/如何/设计/机制/原理」等；概念词否决降为扣分 |
| **8** | **无查询改写** | ⬜ 未做 | 中文 → 英文标识符仍无通路（**C1**） |
| **9** | **无 embedding** | ⬜ 未做 | 跨语言语义召回不可得 —— 这是**约束**，不是缺陷（**C2**） |
| **10** | **触发闸门不看内容** | ⬜ 未做 | 闸门与判分耦合脆弱：改判分不影响闸门语义 |
| **11** | **无停用词表** | ✅ 已修 | 英文虚词进倒排表，`tf` 乘爆把噪声结果推上榜首 |
| **12** | **搜索引擎缓存没接线** | ✅ 已修 | 工具挂 `None`、watcher 写另一份 → 每次全量重建 11.9s |

### 2.3 「触发不够智能」的直接成因

用户反馈的「`CodebaseSearch` 触发不智能」，实测下来是**三个独立问题叠加**：

1. **中文提问拿 0 分**（缺口 7）：`score_rank_locate_route` 的中文标记词太少；
2. **首次调用返回空**（缺口 6）：冷启动 `Warming`，自动触发那一次基本白跑；
3. **框架不重试**：`already_attempted` 闸门保证只自动触发一次， Warming 后完全依赖模型自觉重跑。

> 注意：`CodebaseSearch` 的**工具名**与拼写无关（上一轮已确认 `Codebase` 才是正确拼法，CC 里也**没有**这个工具 —— 其等价物是 `RepoMap`）。

---

## 3. 方案

### 3.0 优先级依据

- **先做无依赖的**（本地可算、不改接口、立刻见效）；
- **再做接线的**（把已有零件接上，改动小、收益大）；
- **最后才考虑外部依赖**（embedding / 重训模型）。

### 3.1 第一梯队：立刻可做

| # | 动作 | 对标 | 状态 |
|---|---|---|---|
| **A1** | **CJK 分词**（bigram + 单字） | cAST 的语言无关性主张 | ✅ 已完成 |
| **A2** | **ripgrep 字面通道提为并列召回路** | Augment `matched_string` + `contains_string` | ✅ 已完成 |
| **A3** | **RRF 融合改成异构三路**（语义-expanded / 语义-exact / 字面-ripgrep） | Augment 三路合一 | ✅ 已完成 |
| **A4** | **补中文标记词 + 概念否决降为扣分** | — | ✅ 已完成 |
| **A7** | **英文停用词表** | Lucene/BM25 的标准做法 | ✅ 已完成 |
| **A8** | **搜索引擎缓存接线**（共享单例） | Augment「索引只建一次」 | ✅ 已完成 |
| **A5** | **查询级 `path` 过滤 + `contains_string` + `max_chunks`** | Augment 官方 schema 直接照抄 | ⬜ 未做（schema 已有 `path`，缺 `contains_string` / `max_chunks`） |
| **A6** | **冷启动补跑**（等索引就绪后框架自动重跑一次） | — | ✅ 已完成 |

### 3.1.1 已完成项的实测收益

**召回精准度**（真实仓库 `/mnt/h/test/starcode`，851 文件 / 10 MB）：

| 查询 | 改前命中 | 改后命中 |
|---|---|---|
| `permission gate` | `commands/permissions.rs`（匹配词 `an, and, docs, in, is, it...` —— 12 个里 10 个是噪声） | **`core/permission_rules/mod.rs`**（`permission, policy, tool`） |
| `tool name constants` | `core/tools/edit.rs`（无关） | **`agent/tool_routing/helpers.rs`**（`constants, name, tool`） |
| `policy engine rule evaluation` | `core/context/search_engine.rs`（无关） | **`core/policy/policy_engine.rs`**（`policy, rule, engine, deny, ask`） |

**触发命中率**（`score_rank_locate_route`，阈值 60）：

| 提问 | 改前 | 改后 |
|---|---|---|
| `用户鉴权是怎么做的？` | 0 | **75** |
| `如何实现限流？` | 0 | **75** |
| `为什么限流要这么设计？` | 0 | **67** |
| `重试机制是什么原理？` | 0 | **67** |
| `config parsing` / `重构这个模块` | 0 | 0（正确地不触发） |

**检索延迟**：

| 场景 | 改前 | 改后 |
|---|---|---|
| 首次（建索引） | 11.9s | 11.9s（一次性） |
| 不同查询 | **11.9s**（每次全量重建） | **2.3s**（引擎缓存） |
| 同一查询 | 11.9s | **3ms**（查询缓存） |

> 11.9s → 2.3s 的原因是根因修复：`CodebaseSearchTool` 挂的是 `None`，
> 而 watcher 把索引写进了 `ContextEngine` 那份 cache —— **两边不是同一个对象**。
> 于是 watcher 明明在后台把索引建好了，工具却永远读不到，每次检索都退回
> 同步全量重建。改成进程级共享单例后，索引只建一次。

### 3.2 第二梯队：把已有零件接上线

| # | 动作 | 说明 |
|---|---|---|
| **B1** | **调用图接入检索**：命中 chunk 后附带「调用者/被调用者」 | 回答「改这里影响谁」；`call_graph.rs` 已就绪 |
| **B2** | **`precise_retrieval` 接线**：编辑前自动取目标函数的完整定义 | `for_edit` 已实现但零调用者 |
| **B3** | **符号级召回通道**：函数名/类名精确匹配优先于文本匹配 | Augment「Relationship awareness」的可实现近似 |
| **B4** | **会话历史作为检索语料** | Augment 独有（`conversation-retrieval`），StarCode 有 transcript 存储，成本低 |

### 3.3 第三梯队：需要外部依赖

| # | 动作 | 代价 | 收益 |
|---|---|---|---|
| **C1** | **LLM 查询改写**：中文提问 → 英文代码关键词 | 一次额外 LLM 调用（有延迟） | 跨语言召回；**这是当前唯一可行的跨语言方案**（无 embedding） |
| **C2** | **本地 embedding** | 需引入模型（依赖体积 + 首次下载） | 真正的语义跨语言召回 |
| **C3** | **LLM listwise 重排** | 一次 LLM 调用 | 精排；Augment 用 `gemini-3-0-flash-minimal` 这类小模型 |

> **C1 与 C2 的选择**：C1 无依赖、延迟可控、能立刻验证；C2 一次性投入后每次查询都更快更准。
> 建议先 C1 验证「跨语言是真实痛点」，再决定是否上 C2。

### 3.4 明确不抄的

| 不抄 | 原因 |
|---|---|
| 纯向量 + 服务端闭源 | embedding 模型不可得；且 Augment 自己承认 RAG 有上限 |
| 训练专用检索模型（SWE-grep） | 成本量级完全不同 |
| 引入 Merkle tree 增量索引 | 那是第三方项目 `zilliztech/claude-context`，**不是 Augment 的实现** |
| `ast-indexing` / `context-compression` / `real-time-sync` 工具名 | 第三方误传，官方 bundle 里出现次数为 0 |

---

## 4. 值得直接抄的三条原则

1. **precision 优先于 recall**（Windsurf 官方：*"polluting the context of the main agent was more detrimental than leaving some context out"*）。
   → StarCode 的重排与截断都该偏向「宁少勿滥」。

2. **结构化输出优于散文摘要**（Windsurf：子代理只交回「文件 + 行范围」，不交回摘要）。
   → `CodebaseSearch` 的输出应当是文件 + 行范围 + 语义块，便于下游直接用。

3. **完整语义块，不切碎的片段**（Windsurf：*"You must include ENTIRE semantic blocks (functions, classes, definitions)"*）。
   → StarCode 已有 tree-sitter 分块，输出时应给完整函数/类，而不是命中行前后几行。

---

## 5. 量化基线

`src/core/context/retrieval_baseline.rs` 建立了一个可复现的合成语料基线：

- 4 条英文查询（应当全中）—— **这是底线，不是目标**
- 3 条中文查询（当前 0/3 命中）—— 标 `#[ignore]`，等跨语言召回落地后转正

```bash
cargo test --lib retrieval_baseline -- --nocapture --include-ignored
```

改造后应能直接看到数字变化，而不是「感觉变好了」。

---

## 6. 来源

**Augment**：官方文档（Context Engine MCP Overview / Context Connectors / Workspace Indexing）· 官方博客（量化向量搜索 2025-06-11、Context Lineage、code review 基准、Auggie CLI harness）· 官方开源 npm 包 `auggie@0.36.0` / `auggie-sdk@0.2.0` / `context-connectors@0.1.4`（反编译）· cAST 论文（arXiv:2506.15655）

**Windsurf / Devin**：Cognition 博客「Introducing SWE-grep and SWE-grep-mini」（2025-10-16）· Devin Docs Fast Context / Context Awareness / Changelog · Devin 博客「Remote Indexing and Multi-repository Context Awareness」

**第三方（仅参考，均已标注）**：`fast-context-mcp` 逆向复刻（mitmproxy 抓包）· `ferstar/fast-context` hybrid 架构 · fast.io / continuumcode.ai

**未查到**：Augment 服务端 chunk 参数 / embedding 模型 / 融合权重 / 是否真有调用图 · FastContext 绝对召回率与延迟毫秒数 · SWE-grep 参数量
