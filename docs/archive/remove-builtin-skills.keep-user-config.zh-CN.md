# 去掉源码级内置 skills —— 保留用户配置的 skills 与 MCP

- 日期：2026-11-05
- 仓库：`H:\test\starcode`（Rust CLI，branch `dev-m`）
- 目标：删掉**写死在二进制里**的 skill 实现，**用户自己写的** skill 与 MCP 配置必须照常可用

---

## 1. 结论

| | 处理 | 依据 |
|---|---|---|
| 内置 Rust 子代理（analyzer / auto_fix / editor / explore / navigator / verify） | **删除**（-2335 行） | 用户明确：「像那种内置的skills没必要」 |
| 内置 skill 的 prompt 资源（`src/core/prompts/skills/`） | **删除**（-162 行） | 随内置 agent 一起成为死资源 |
| 用户自定义 agent（`.star/agents/*.md`） | **保留并加强** | 用户明确：「用户配置 skills 要能用」 |
| 用户自定义 SKILL.md（`.star/skills/`） | **保留 + 接线**（原本是死代码） | 同上 |
| MCP 全部能力 | **保留，一行未改** | 用户明确：「mcp 要能用」 |

**中途走偏过一次**：我把 MCP 也删了（-6477 行），用户发现后立即纠正。已完整回滚——最终 diff 中 MCP 相关改动为 **0 行**。

---

## 2. 外部依据（exa MCP 核实）

Claude Code 官方的分层（<https://code.claude.com/docs/en/skills>、<https://code.claude.com/docs/en/mcp>）：

- **MCP 是用户配置的**。三种 scope：local（`~/.claude.json`）、project（`.mcp.json`）、user（`~/.claude.json`）。built-in MCP server 只有一个 `computer-use`，而且**默认关闭**。
- **bundled skills 是 prompt-based 的 markdown**（`/doctor`、`/code-review`、`/batch`、`/debug`），可以用 `disableBundledSkills` 关掉。

对照本地参考副本 `.study/study_or_copy_projects/claude-code-main/src/skills/`：Claude Code 把 skills 分成 `bundledSkills`（程序注册，随二进制发布）和 `loadSkillsDir`（扫用户目录）两条**独立**路径。

结论：**「删内置、保用户」正是 Claude Code 自己的分层**，两边行为一致。

---

## 3. 改了什么

### 3.1 删除内置 skill（commit `b8fa838` / `6ecbffd`）

```
src/agent/skills/analyzer.rs     -277
src/agent/skills/auto_fix.rs     -235
src/agent/skills/editor.rs       -488
src/agent/skills/explore.rs      -288
src/agent/skills/navigator.rs    -799
src/agent/skills/verify.rs       -248
src/core/prompts/skills/         -162
```

**保留**（框架，用户自定义 agent 依赖它）：
- `src/agent/skills/mod.rs` —— `SubAgent` trait / `SubAgentManager` / `SubTask`
- `src/agent/skills/custom.rs` —— 用户自定义 agent 的加载与执行
- `src/agent/skills/loader.rs` —— SKILL.md 解析
- `src/agent/skills/resources/*.md` —— **marketplace 的安装模板**（见 §3.4）

### 3.2 团队成员从静态清单改为动态

`TEAM_AGENT_CATALOG`（`&'static` 常量，写死 5 个内置 agent）→ `team_agent_catalog(project_root)`，成员来自用户定义的 agent。

连带改动：`TeamAgentDef` 的字段从 `&'static str` 变成 `String`（名字不再可能是静态的），`find_team_agent` / `resolve_team_agents` / `plan_team_run` 都多带一个 catalog 参数。

**清单为空是正常状态**（用户还没定义 agent）—— `/agents team list` 如实显示「没有成员」，而不是列出一堆点不动的东西。

### 3.3 Explorer 子代理：专用实现下线，落到通用 LLM 循环

`ExploreAgent` 是内置的确定性检索专家（ACE 广搜 + deep filter）。删掉后 `explorer` 请求走通用 LLM 循环——这条路径本来就能工作（`analyzer` / `editor` / `code_reviewer` 一直都是这么跑的），只是少了固定编排。

用户**自定义**的 explorer agent 不受影响（走 `SubAgentManager`）。

### 3.4 用户自定义 SKILL.md 接线（原本是死代码）

**发现**：`SkillLoader::load_skills_from_dir` **零调用者** —— 用户在 `.star/skills/<name>/SKILL.md` 写的 skill 从来没被加载过。内置 skill 下线后，这是唯一的 prompt 型 skill 来源，不接通就等于用户写的 skill 全都不可用。

现在 `SkillTool` 按名字解析：

```
<project>/.star/skills/<name>/SKILL.md      ← 目录形态
<project>/.star/skills/<name>.skill.md     ← 单文件形态
~/.starcode/skills/<name>/SKILL.md         ← 全局
~/.starcode/skills/<name>.skill.md
```

- 分隔符等价：`my-skill` 也能命中目录 `my_skill`（用户手写目录名时最容易踩的一步）
- 可用名字写进**工具描述**，模型才知道有这些 skill 可调

`marketplace` 装出来的 skill 落在 `.star/extensions/skills/<name>/SKILL.md` —— 现在也能被找到了（之前连它都加载不了）。

---

## 4. 验证

```
cargo check --workspace --all-targets --locked   → exit 0
cargo fmt --all -- --check                       → clean
cargo clippy --workspace --all-targets           → 改动文件零警告
cargo test --workspace --lib -- --test-threads=1 → 1940 passed / 0 failed / 4 ignored
```

新增 5 条测试（`src/agent/tools/skill.rs`）：

| 测试 | 锁住什么 |
|---|---|
| `a_user_defined_skill_is_found_in_the_project_skills_dir` | `.star/skills/review/SKILL.md` 能被找到 |
| `a_skill_is_found_regardless_of_separator_style` | `my-skill` ≡ `my_skill` |
| `a_flat_skill_file_is_found` | `<name>.skill.md` 形态 |
| `a_missing_skill_resolves_to_none` | 没有就是 None |
| `every_listed_skill_name_resolves_to_a_skill` | **描述与查找必须一致** |

最后那条最关键：清单和查找用两套代码，不一致就会出现「工具描述说有 `review`，模型调了却报 no suitable agent」——而这是内置下线后唯一的 skill 来源。

---

## 5. 关键代码位置

```
src/agent/skills/mod.rs                    框架保留，内置 agent 移除
src/agent/skills/custom.rs                 用户自定义 agent（保留）
src/agent/tools/skill.rs:50                工具描述里列出用户 skill
src/agent/tools/skill.rs:277               按名字解析用户 skill
src/agent/tools/skill.rs:358               load_user_skill_from_dirs()
src/agent/tools/skill.rs:417               list_user_skill_names()
src/agent/subagent/runner.rs               Explorer 落到通用循环
src/commands/agents/team_definitions.rs:82 team_agent_catalog()（动态成员）
```

---

## 6. 保留的 MCP 能力（未改动）

```
src/core/mcp/            10 个文件（client / config / transport / oauth / server / manager）
src/commands/mcp.rs      /mcp 命令（add / remove / list / import / refresh / install）
src/ui/components/modal/mcp_modal.rs   /mcp 弹窗
src/core/tools/mcp_auth.rs, mcp_resources.rs, src/tools/mcp_tool.rs
11 个 Mcp* 工具（ListServers / ListTools / ToolInfo / SearchTools / RestartServer /
                 Refresh / ListResources / ReadResource / ListPrompts / GetPrompt / Auth）
插件的 mcpServers 支持
```

---

## 7. 后续：skill 来源统一（2026-11-05 补）

发现并修掉一条**断链**：marketplace 装好的 skill 落在
`.star/extensions/skills/<name>/SKILL.md`，而 agent 侧只扫 `.star/skills/`
与 `~/.starcode/skills/` —— **装完的 skill 永远不会被模型调用**，
`/skills list` 里也看不见。

现在三个来源由 `Storage::skill_roots()` 统一给出（agent 侧、`/skills list`、
`/skills show`、`/reload-skills`、`/context` 全用同一套）：

| 来源 | 位置 |
|---|---|
| 项目 | `.star/skills/` |
| 全局 | `~/.starcode/skills/` |
| 扩展安装 | `.star/extensions/skills/` |

这与 Claude Code 的分层一致（<https://code.claude.com/docs/en/skills>）：
用户 skill、项目 skill、插件 skill 是**三处独立的扫描源**，插件 skill
不会被塞进用户目录。

---

## 8. 遗留与建议

**根因 C（索引截断时引擎不落地）仍未解决** —— 与本次无关，详见 `docs/codebase-search-index-incident.zh-CN.md` §5.1。

**用户 SKILL.md 的元数据（`allowed_tools` / `model` / `when_to_use`）尚未被 `SkillTool` 消费** —— 目前只加载正文并渲染成 prompt。这是既有状态（`load_skills_from_dir` 从未被调用过），本次只接通「能找到并加载」，没有扩到「按 frontmatter 约束执行」。要按 CC 的语义（`disable-model-invocation` 控制模型能否自动触发、`allowed-tools` 限制工具集）实现，是另一件事。
