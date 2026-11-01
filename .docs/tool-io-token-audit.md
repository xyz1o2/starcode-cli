# 工具 I/O 与提示词审计

> 快照：2026-10-31
> 范围：`src/core/tools/*`、`src/core/prompts/system-prompts/tool-description-*.md`、`src/agent/tool_routing/*`、`src/core/context/search_engine.rs`、`src/ui/components/tool_render.rs`
> 外部来源：AWS《MCP tool design: Practical approaches and tradeoffs》(2026-07-09)、Nodeflare《MCP Token Usage from Tool Responses: 8 Ways to Shrink MCP Tool Results》(2026-10-15)、OpenKrill《Lean MCP Tool Answers: 74 Percent Less Text, Measured》(2026-10-08)、Runpod《Designing MCP tools that don't blow up your agent's context window》、agentpatterns《Token-Efficient Tool Design》《Graceful tool-output truncation》、Voxgig《How to shape a tool result so an agent can reason about it》(2026-10-19)。
> 原计划用 exa MCP 检索；当前环境未配置 exa（仅 `chrome-devtools` / `opencode` 两个 MCP server），改用内置 websearch。**第三方数字均为对方实测，非本项目实测**。

这份文档记录两件事：**一次线上事故的根因**（§1），以及**工具输入输出与提示词的 token 成本现状**（§2–§4）。带 `待确认` 标记的条目尚未验证，不作为结论。

---

## 0. 一句话结论

1. `CodebaseSearch` 零命中是**路由事故**，不是检索缺陷 —— 它被喂了中文自然语言关键词去检索纯英文代码库，数学上必然零命中；
2. 触发谓词**完全不看提问内容**（`_user_input` 是下划线参数），所以同一句中文每轮都白打一次；
3. 56 个工具描述共 **50032 字节 ≈ 12508 token**，是每次请求都要付的固定税，尚未做任何预算或 lazy-load。

---

## 1. 事故：同一个提问，一个工具 0 命中、另一个 82 命中

### 1.1 现象

同一会话里出现两种结果形态：

```
● codebase search(query=停, turn=43)
  ⎿  Codebase search · 0 ranked results
● codebase search(query=没动 动静, turn=1)
  ⎿  Codebase search · 0 ranked results

● CodeGraph(path=/mnt/d/ai-workspace/test/new-api, symbol=ApplyParamOverride)
  ⎿  Call Graph Search for 'ApplyParamOverride'
     Total symbols indexed: 13186 | Resolved call edges: 65403
     Found 82 matching symbols. Showing top 5 by name match:
     ... +344 lines  (press Tab to expand)
```

### 1.2 根因：两个工具检索的不是同一个东西

| | `CodebaseSearch(query=…)` | `CodeGraph(symbol=…)` |
|---|---|---|
| 检索对象 | 分块后的**代码文本 token** | tree-sitter 解析出的**符号表** |
| 匹配方式 | TF-IDF 倒排查 posting + ripgrep 字面 | 符号名 `contains` 子串 |
| 输入该是什么 | schema 明文要求 `Keywords or a short phrase` | 真实存在的标识符 |
| 实际拿到的 | 中文自然语言压成的 bigram | 真实的 Go 函数名 |

**两者都没坏，坏的是把 A 的输入形态喂给了 B。**

### 1.3 `CodebaseSearch` 零命中的完整链路

① **关键词提取** —— `src/agent/tool_routing/tool_call_builders.rs:27`
自动触发路径把整句提问压成关键词，而非直接透传（该处注释 §18-25 已记录过"整句塞给词法引擎"的前车之鉴）。

② **CJK 走 bigram** —— 同文件 `:215-229`
中文无空格，整句会被切成单 token，虚词表失效。这里逐字滤虚词后拼二元组。「没动 动静」由此而来；单字成词时（`:226-228`）保留末字兜底 —— 「停」由此而来。

③ **查询侧分词** —— `src/core/context/search_engine.rs:687`、`push_cjk_tokens :778`
产出「整串 + bigram + 单字」三种形态。`base_terms = {没动, 动静}`（或 `{停}`）非空，检索继续。
> 注意 `push_cjk_tokens` 是**直接 insert**，绕过了 `push_token` 里的 `MIN_TOKEN_LEN = 2`（`:37`、`:804`）。单字能进查询是设计如此，不是 bug。

④ **倒排表查不到** —— `search_with_options :449-466`
`self.index.get("没动")` → `None`。索引里存的是代码 token（标识符按 snake/camel 拆分 + 注释中的中文），一个 Go 仓库里没有「没动」这个词。

⑤ **第二路同源，同样空** —— `codebase_search.rs:1152-1159`
Strategy A（expanded）与 Strategy B（exact）是**同一个词法引擎的两种参数**，A 空则 B 必空。二者不是异构信号。

⑥ **第三路字面召回同样空** —— `codebase_search.rs:1188`
`literal_recall_files("没动 动静", root)` 走 ripgrep，代码里没有这个中文字面串。

⑦ **三路空集融合** —— `:1236-1239`
`fusion::fuse(&[expanded, exact, literal])` 输入全空 → 输出空 → `0 ranked results`。

**结论：零命中在语义上是正确的，错在根本不该发这次调用。**

### 1.4 `CodeGraph` 为什么必然命中

`code_graph_tool.rs:69` → `codebase_search::trace_call_chain` → `build_call_graph` 全仓解析出 13186 符号 / 65403 条调用边 → `find_symbols`（`call_graph.rs:134`）按 `name.to_lowercase().contains(&lower)` 匹配。

`ApplyParamOverride` 是真实存在的 Go 函数（`relay/common/override.go:149`），命中是必然的。

> **顺带发现（非事故根因）**：`trace_call_chain`（`codebase_search.rs:1611-1623`）输出 `Found 82 matching symbols. Showing top 5 by name match:`，但代码是 `symbols.iter().take(5)` —— **没有任何排序**。所谓 "top 5 by name match" 实际是符号表的插入顺序，即 `find_symbols` 的返回顺序。82 个命中本身也是 `contains` 子串匹配带来的膨胀（同名 wrapper / 测试 / 生成代码）。文案与实现不符，会误导模型以为它看到了最相关的 5 个。

### 1.5 为什么「一直会出来」

`src/agent/tool_routing/triggers.rs:549-557`：

```rust
fn should_trigger_codebase_search_with_flag(
    _user_input: &str,        // 下划线 —— 未使用
    _current_content: &str,   // 下划线 —— 未使用
    active_tools: &HashSet<String>,
    already_attempted: bool,
    auto_enabled: bool,
) -> bool {
    auto_enabled && !already_attempted && active_tools.contains("CodebaseSearch")
}
```

谓词**不看提问内容**。真正的判分在 `score_rank_locate_route`（`:126`），但它只判「这句话像不像跨文件定位」，**不判「这些关键词在这个代码库里有没有可能命中」**。

代码注释自己承认了这段历史（`:545-548`）：「以前它带下划线是因为谓词压根不看提问内容……于是开关一打开就是每轮盲打。现在判分在 `score_rank_locate_route` 里做」。

结果：每来一句长得像「帮我找找 X」的中文 → 打一次 → 返回 0 → 白烧一整轮。

`待确认`：**`turn=1` 出现两次完全相同的调用**。怀疑 `should_prefetch_codebase_search`（首轮预取，`history_len == 0`，`:560`）与 `select_best_auto_trigger`（`:126`）两条路径各注入一次，尚未验证。

---

## 2. 工具描述的固定上下文税

| 项 | 值 |
|---|---|
| 文件数 | 56 个 `tool-description-*.md` |
| 总体积 | 50032 字节 |
| 估算 token | ≈ 12508（字节 / 4，粗估，非分词器实测） |

**这笔成本在调用任何工具之前就已付出，且每个请求都要重付一遍。**

Top 5（占总量约 37%）：

| 文件 | 字节 |
|---|---|
| `tool-description-bash.md` | 6020 |
| `tool-description-runagent.md` | 3662 |
| `tool-description-grep.md` | 3115 |
| `tool-description-selfconfig.md` | 3007 |
| `tool-description-managetasks.md` | 2521 |

外部研究的一致结论（Runpod）：*「Do not expose 40 narrow tools when six well-designed ones cover the same surface. A description exists to answer one question: should the model pick this tool for this task?」*

对比：Anthropic 的 Tool Search Tool 走 lazy-load，报告最高 **85% token 削减**。本项目已有 `mcpsearch`（460B）与 `toolsearch`（1579B）两个工具 —— 是否可复用它们做按需加载，是 ROI 最高的一条路径（**尚未评估**）。

---

## 3. 输出侧的已知问题

### 3.1 空结果不解释原因

`src/ui/components/tool_render.rs:781-791`：折叠态统计以 `File: ` 开头的行数，为 0 时输出 `Codebase search · 0 ranked results`。

零结果**不区分**以下情形，模型拿不到任何下一步线索：

- 冷索引（后台构建未完成）—— 其实 `codebase_search.rs:1117-1129` 已写了 `Warming` 分支的专门文案，措辞还是刻意的「再试一次，别改用 Grep」；但这条只在 `EngineSource::Warming` 时命中，索引已就绪时的真空结果走不到；
- 查询词与代码库语言不匹配（本次事故的情形）；
- 关键词被过滤/停用词吃光。

外部研究（yaroslavboiko / Nodeflare）的对应准则：*「Explain empty results, and never show a failed lookup as a zero.」* / *「A response that says 'search requires 2 or more terms in query' tells the LLM exactly what to change. A response that returns only 'no results' gives it nothing, causing the model to either give up or keep guessing.」*

### 3.2 折叠态摘要与实际展开体积脱节

`CodeGraph` 折叠态只显示头部几行，展开是 `+344 lines`。模型在折叠态无法判断值不值得展开，只能盲展开。

### 3.3 三路召回里两路同源

§1.3 ⑤：`expanded` 与 `exact` 是同一词法引擎的两种参数。RRF 融合三路，实际只有**两路异构**（词法 + 字面）。`codebase_search.rs:1180-1187` 的注释已说明设计意图是三路异构。

---

## 4. 外部研究的 8 个模式（待逐条评估适用性）

来源：Nodeflare 2026-10-15，带实测。

| # | 模式 | 典型效果 |
|---|---|---|
| 1 | 只返回模型需要的字段 | 大（该文实测 90%） |
| 2 | 带 `limit` 小默认值 + cursor 分页 | 对 list/search 类工具大 |
| 3 | 提供 `response_format`（concise / detailed） | Anthropic 示例约省 ⅔ |
| 4 | 同一份数据不要发两次 | 重复时最多 50% |
| 5 | 大 payload 返回 `resource_link` | 把 bulk 移出结果 |
| 6 | 用名字而非不透明 ID / URL | 省 token 且提升准确率 |
| 7 | 扁平列表用表格 / 紧凑格式 | 21–46% |
| 8 | 截断与错误信息要**引导**模型 | 避免无效重试 |

配套准则（agentpatterns）：截断契约必须三件套齐全 —— **useful prefix + 结构独立的标记 + continuation handle**。标记要放在**载荷之前**（前置 banner 或独立 JSON 字段），因为「末尾一行 `[PARTIAL]` 最容易被模型当成注释忽略」；Anthropic 自己的 Read 工具就出过这个 bug（agent 把预览当全文件，后半段规则静默缺失）。

---

## 5. 下一步（未执行，待确认范围）

### 5.1 事故修复（独立于审计）

- [x] **触发前置语言检查**：`score_rank_locate_route` 判分后，若提取出的关键词全为 CJK 且代码库中 CJK token 覆盖率低于阈值 → 不触发 `CodebaseSearch`。
  → 落地为 `triggers.rs::codebase_search_language_gate`，判据见 §5.1.1。
- [x] **零结果分类**：区分「冷索引」「语言不匹配」「关键词被过滤」，分别给出可执行的下一步。
  → 落地为 `search_engine.rs::QueryDiagnosis` + `codebase_search.rs` 的诊断块。
- [x] **修 `trace_call_chain` 文案**：要么按名称匹配度真排序，要么把 "Showing top 5 by name match" 改成 "Showing first 5"（`codebase_search.rs:1611`）。
  → 选了「真排序」：`call_graph.rs::find_symbols` 现在按 精确 > 前缀 > 子串、同档短名优先、路径兜底排序。文案保持不变，但此时它是**真的**。
- [x] **查 `turn=1` 双发**（§1.5 待确认项）→ 见 §5.1.2。
- [x] `codebase_search_attempted` 闸门语义复核：`src/agent/tool_helpers.rs:424` 的注释提到它曾从「调用过」改过语义。
  → 复核发现**预取路径仍用旧语义**（见 §5.1.3）。

### 5.1.1 语言闸门的判据

闸门只拦**确定零命中**的情况，两个条件都要成立：

1. 关键词**全是纯中文词** —— 含任何 ASCII 字母或数字就放行。
   （「502错误」里的 `502` 可能命中代码里的状态码，不能当纯中文。）
2. 索引里**一个 CJK token 都没有** —— 索引状态未知（冷启动）时**放行**。
   「不知道」不等于「不匹配」：据此拦截会把一次本该成功的检索也毙掉。

数据来源：`SearchEngine::has_cjk_token()`（由 `cjk_doc_count` 派生，删文档时精确递减），
经 `SearchEngineCacheManager::root_has_cjk_token()` 暴露。

> 语言闸门**不**替代零结果诊断。模型主动调 `CodebaseSearch` 时闸门管不着，
> 那条路径由 §5.1.4 的诊断块兜底。

### 5.1.2 `turn=1` 双发的根因

事故记录里的两行：

```
● codebase search(query=停, turn=43)
● codebase search(query=没动 动静, turn=1)
```

`turn=1` 出现两次**不是**预取与自动触发各注入一次（预取用 `turn=0`，
且预取路径 `*codebase_search_attempted = true` 会挡住随后的自动触发）。

真正的来源是**冷启动补跑**（`agent_tool_process.rs`）：首查拿到 Warming
（索引还在后台构建）→ 框架等索引就绪 → **用同一个 `turn` 重跑一次**。
两次的 `turn` 相同、`query` 相同，看起来就是"同一个调用出现了两次"。

这个重跑本身是**必要的**（冷启动首查几乎必然空手而归），但它不该看起来像重复调用。

**已修**：重跑现在带 `warmup_retry` 标记（`build_codebase_search_tool_call_with`），
UI 摘要会显示 `codebase search(query=…, turn=…, retry)`。补跑与首查的
query/turn 保持一致（它就是首查的重跑），但标记让排障时一眼看出这是有意补跑。

### 5.1.3 `codebase_search_attempted` 闸门语义复核

`tool_helpers.rs::tool_result_is_usable` 的注释说这个闸门曾从「调用过」
改成「拿到可用结果」——自动触发路径（`agent_tool_process.rs`）确实改对了
（`= satisfied`，不是 `= true`）。

但**预取路径**（`agent_loop.rs::handle_prefetch`）仍然是
`*codebase_search_attempted = true`，即旧的「调用过」语义。
预取发生在首轮模型请求**之前**，那时索引几乎必然还在构建 → 工具回 Warming
（"稍后重跑"）→ 闸门被这一次**什么都没搜到**的调用永久关上 →
工具自己给的"再试一次"建议被自家 flag 当场否掉。

已修：预取路径改用 `tool_result_is_usable(&result)`，与自动触发路径同口径。

### 5.1.4 零结果诊断的分类

`QueryDiagnosis`（`search_engine.rs`）给出四个事实：

| 事实 | 含义 |
|---|---|
| `query_is_pure_cjk` | 查询词是否全是中文 |
| `index_has_cjk_token` | 索引里有没有中文 token |
| `index_cjk_token_ratio` | 索引里中文 token 占比 |
| `term_coverage` | 查询词有多少比例在倒排表里存在 |

`codebase_search.rs` 据此分四类，每类给出**可执行**的下一步：

| 情况 | 诊断 |
|---|---|
| 纯中文查询 + 纯英文索引 | 语言不匹配 —— 换英文领域词，或用 `Grep` |
| 关键词一个都不在索引里（覆盖率 <15%） | 词选错了 —— 用 `Grep` 搜确切字符串 |
| 索引里有中文但词仍没命中 | 词只在没被分块的注释/文档里 —— 用 `Grep` |
| 词存在但没通过排名阈值 | 换个更具体的词，或用 `Grep` 看全部原始命中 |

外部准则（Nodeflare）：*「A response that returns only 'no results' gives it nothing,
causing the model to either give up or keep guessing.」* —— 分类诊断就是为了
让模型**有下一步可走**，而不是换一个说法再猜一遍。

### 5.2 审计与改造

- [ ] 逐工具评输入/输出，产出超标清单与具体改法
- [ ] 描述体积预算 + 测试卡口（外部研究建议：测试强制 `tools/list` 总字节上限）
- [ ] 评估 `toolsearch` / `mcpsearch` 复用做 lazy-load
- [ ] 逐条评估 §4 八个模式对本项目哪些工具适用

---

## 6. 工具块的展开 / 收起：操作 ergonomics

> 本节与 §1–§5 无关：不是 token 账，是**人机交互账**。
> 起因是用户反馈：「展开一个工具视图之后，要再收起它非常难 —— 得一路滚回
> 顶部去点那个显示出来的工具名。」

### 6.1 一个工具块在内存里是**两条** entry

排查这个问题时最先撞上的事实：一次工具调用在 `chat_history` 里是**两条**
entry，不是两条。

| entry | `entry_type` | 渲染成什么 | 共享的东西 |
|---|---|---|---|
| 工具头 | `ToolCall` | 一行 `● 工具名(参数)` | `tool_call.id` |
| 工具体 | `ToolResult` | 结果正文 | `tool_call.id` |

折叠位 `expanded_tool_call_ids` 按 `tool_call.id` 记 —— 两条共用同一份，
所以从哪一条点都是切同一块。

### 6.2 三个真 bug

**① `is_tool_expanded` 对同一个工具给出相反的答案。**

旧实现按 entry 类型给不同的默认值：工具头默认「展开」，工具体默认「折叠」
（编辑类工具除外）。于是同一个工具的头与身**给出相反的结论** —— 拿头部问
「正文展开着吗」得到的判断是反的。现在改成**按工具问**（`is_tool_expanded`
只读 `tool_call.id` 与翻转位），头与身永远一致。

**② 切换只失效了被点的那一条的渲染缓存。**

工具头与工具体是两条 entry、两个缓存槽。旧实现只清被点的那一条：点工具头
时工具体还留着旧渲染缓存 —— 屏幕上纹丝不动，用户的感受是「点了没反应」。
现在按 `tool_call.id` 把**同一块的全部条目**一起失效。

**③ 鼠标命中测试认错了行。**

工具头那条 entry 渲染出来是 `[前导空行] + [工具名行]` —— 工具名通常在第 1 行，
不是第 0 行。而旧命中测试判的是「第 0 行」，也就是那行**看不见的空行**。
于是用户点工具名没反应，点工具名上面那行空白才有反应。

修法不是把行号写死成 1，而是把「这条 entry 前面要不要垫空行」抽成
`components::chat_history::leading_blank_rows()`，让**渲染与命中测试用同一份
判断**。规则本身没变（除首条外都垫一行，但工具序列与 Agent 任务条目之间
连续排布不垫），只是不再各算一遍。

### 6.3 现在的交互约定

| 操作 | 结果 |
|---|---|
| 点**看得见的工具名**那一行 | 展开 / 收起 |
| 点工具名**上面那行空白** | 什么也不发生 |
| 点折叠态的摘要行 | 展开 |
| 点**展开后正文里任意一行** | 收起 |
| Tab | 切换视口下的那一块 |

「点正文里任意一行就收起」与 thinking 块的既有约定一致
（「展开时点任意行折叠」）。这是本次修改的核心：展开后的正文可能几十上百行，
工具名远在屏幕外 —— 不这样，用户就得一路滚回顶部才收得起来。

`TodoWrite` 的结果是任务清单（对标 CC 的 TaskList），整份渲染、不看展开位 ——
对它做折叠/展开没有意义，点它必须什么都不发生。规则收在
`ChatState::tool_block_is_foldable()` 里，鼠标与 Tab 走同一条路径，谁都
绕不过去。

### 6.4 Tab 的目标：整块，不是工具头那条 entry

旧实现只认 `ToolCall` 那条 entry。展开后的正文能有两百行，视口中心落在正文里
是常态 —— 那时 Tab 匹配不上任何东西，用户看到的是「按了没反应」。

现在候选是「同一个工具块的头 + 身」：工具体有多高都算进这一块的区间里。
命中后统一换成工具头那条 entry（折叠位与视口锚定都以头为准）。

### 6.5 收起之后，视口不得指向无关内容

收起正文后，正文下面的内容整体上移；视口若停在原处，用户看到的是一段毫不
相干的内容 —— 同样是「点了没反应」的观感。

`ChatState::anchor_viewport_to_tool()` 保证工具名那一行始终在眼前：

- 工具名当前可见 → 视口不动（不打断正在读的内容）
- 工具名已被滚出视口（用户正在正文中间）→ 把工具头拉回视口顶部

工具头的**绝对行号**不受这次折叠影响（工具体排在工具头后面），所以直接用
上一次渲染的行高累加即可。同时停掉 `auto_follow`：用户刚操作了某一块，
新消息不该把视口拽走。

### 6.6 改了什么

| 文件 | 改动 |
|---|---|
| `src/ui/state/store/mod.rs` | `is_tool_expanded` 改为按工具问；新增 `tool_header_index` / `entry_top_line` / `tool_block_is_foldable`；`toggle_tool_expansion` 失效整块缓存并锚定视口 |
| `src/ui/components/chat_history.rs` | 抽出 `leading_blank_rows()`，渲染与命中测试同源 |
| `src/ui/events/mouse.rs` | 单击按「工具名那一行」命中；正文展开时点任意一行收起 |
| `src/ui/events/input/mod.rs` | `find_focused_tool_entry` 认整块；Tab 与单击共用同一条切换路径 |

回归测试：`ui::events::mouse`（7 条）、`ui::events::input`（4 条）、
`ui::state::store`（3 条）。

---

## 附：尚未回答的问题

- ~~`turn=1` 为何出现两次相同调用（§1.5）~~ → **已查明**：冷启动补跑用了同一个
  `turn`，不是预取与自动触发各注入一次。见 §5.1.2。
- 56 个描述中，有多少是**当前真正被注册**的（`CORE_TOOL_NAMES` / `ToolName` 枚举 vs 描述文件数是否一一对应，未核对）
- 输入输出的实际 token 分布（本次只统计了描述体积，**没有任何工具 I/O 的实测 token 数据**）
