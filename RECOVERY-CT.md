[COMPACTION-RECOVERY]
Route: ct-attack-defense (engineering executor)
Active target: /mnt/h/test/starcode (branch dev-m)
Goal: 完全删除内置 skills + MCP 源码，但保留用户自定义 SKILL.md 加载能力

Exact identifiers/evidence:
- 根因修复已在 commit f0210bc（分裂缓存）+ b8a80d9（修正版文档）
- 门禁基线: 1934 passed / 0 failed / 4 ignored
- 用户决策: "完全删除源码" + "只去掉内置，保留用户自定义"

Technical scope:
- Topology: local repo; Operation class: 修改本地源码（无网络/无外部影响）
- exa MCP 全端点持续 fetch failed（web_search_exa / web_fetch_exa / agent_run 均失败）
- 本地参考副本可用: .study/study_or_copy_projects/claude-code-main

Key structural findings (已核实):
1. src/agent/skills/ 内置子代理: analyzer.rs auto_fix.rs editor.rs explore.rs navigator.rs verify.rs
   - 保留: mod.rs(框架) loader.rs custom.rs（用户自定义 skill 加载）
   - 外部引用点仅 3 处文件:
     src/agent/subagent/runner.rs (ExploreAgent)
     src/agent/tools/skill.rs (Analyzer/Editor/Navigator/AutoFix)
     src/commands/agents/{mod,team_definitions,team_execution}.rs
     src/core/agents/mod.rs
2. src/agent/skills/resources/*.md (6 个) → 被 src/core/extensions/marketplace.rs:539-559
   include_str! 引用，是 /extension marketplace 的**安装模板**。
   ⚠️ 删除会破坏 marketplace 安装功能（安装后落到 .star/skills/ 变成用户自定义）。
   ⚠️ 这些是 Markdown 资源不是 Rust 内置子代理 —— 需与用户确认是否保留
3. MCP: 108 文件引用 / 309 真实改动点
   - ToolName 枚举 11 个 Mcp* 变体 + Skill
   - MCP 管理工具（McpListServers 等 11 个）在 tool_executor.rs:1904-2110 **合成**
     （不在 registry 里注册），需改 get_tool_definitions
   - 注册点 3 处: agent_runtime.rs:193 McpAuth / :239 McpListResources / :242 McpReadResource
   - src/core/tools/mcp_auth.rs, mcp_resources.rs, src/tools/mcp_tool.rs
   - src/commands/mcp.rs + /mcp 命令 + MCP 弹窗
   - dynamic mcp__ 工具路径: tool_executor.rs:1053, 2560
4. 用户自定义 skill 路径（必须保留）:
   Storage::user_skills_dir() / project_skills_dir() / user_agents_dir() / project_agents_dir()
   → src/commands/import.rs, src/commands/skill_doctor.rs

Deliverable checkpoints:
- [x] 根因修复（分裂缓存）
- [x] 修正版文档
- [ ] 删除内置 Rust 子代理（6 文件）
- [ ] 删除 MCP 工具 + 命令 + 枚举变体
- [ ] 处理 marketplace 内置 skill 模板（保留？）
- [ ] 修复 309 处 MCP 引用中真实改动点
- [ ] 全量测试 + fmt + clippy

Current stage & primary technique: 删除执行前的边界确认 — 需要向用户确认
  marketplace 内置 skill 模板（.md 资源）的去留
Observed ledger:
- 内置 skill 模板是 marketplace 安装源，删了 /extension 安装的 skill 就没了
- 但用户已说"完全删除源码"，指的是 Rust 内置子代理；.md 模板性质不同
Next action: 问用户 marketplace 模板去留（1 个问题），然后开始删除
Continuation cues: 继续 / 接着做 / 照做
