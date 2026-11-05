[COMPACTION-RECOVERY]
Route: ct-attack-defense (engineering executor)
Active target: /mnt/h/test/starcode (branch dev-m)
Goal: 去掉「源码级内置」的 skills，保留用户配置的 skills 与 MCP

Exact identifiers/evidence:
- 根因修复已提交: f0210bc（分裂缓存）+ b8a80d9（修正版文档）
- 内置 skills 删除已提交: b8fa838 + 6ecbffd
- exa MCP 已恢复可用（此前长时间 fetch failed）
- Claude Code 官方文档核实结论:
  * MCP = 用户配置（.mcp.json / ~/.claude.json，local/project/user 三 scope）
    仅极少数 built-in（computer-use，默认关闭）
  * bundled skills = prompt-based markdown（/doctor /code-review /batch /debug），
    可用 disableBundledSkills 关闭
  * → 用户意图：删源码级内置，保留用户配置能力

Technical scope:
- 用户明确纠正过一次：「内置 skills 没必要，但用户配置 skills 和 mcp 要能用」
- MCP 一度被误删 6477 行，已完整回滚（剩余 diff 中 MCP 改动 = 0 行）
- 全量测试: 1935 passed / 0 failed / 4 ignored

Key structural findings:
1. 内置 Rust 子代理 6 个已删: analyzer/auto_fix/editor/explore/navigator/verify
   保留框架: src/agent/skills/mod.rs (SubAgent trait/SubAgentManager) + custom.rs + loader.rs
2. 用户自定义 agent（.star/agents/*.md）→ load_custom_subagent_definitions ✅ 通路完好
3. 用户自定义 SKILL.md（.star/skills/）→ load_skills_from_dir **零调用者**（既有死代码，非本次引入）
   SkillTool 只支持 GitHub/本地路径显式加载，不扫目录
4. MCP 全套完好: src/core/mcp/ (10 文件) + /mcp 命令 + 弹窗 + 11 个 Mcp* 工具 + 插件 mcpServers
5. 团队成员清单已从静态常量改为动态（team_agent_catalog(project_root)）

Deliverable checkpoints:
- [x] 分裂缓存根因修复（f0210bc）
- [x] 修正版根因文档（b8a80d9）
- [x] 内置 skills 源码删除（b8fa838 + 6ecbffd）
- [x] MCP 误删回滚（工作区，剩余 diff 无 MCP 改动）
- [x] 全量测试 1935 passed
- [ ] 用户 SKILL.md 目录加载接通（既有死代码，是否做待定）
- [ ] 清理 src/core/prompts/skills/（内置 skill 的 prompt 资源，已无引用者）
- [ ] marketplace 的 6 个内置 skill .md 模板（安装源，用户装后成为自定义 skill）
- [ ] 最终文档与提交

Current stage & primary technique: 收尾阶段 — 确认剩余「内置」残留物的处理方式
Observed ledger:
- src/core/prompts/skills/{analyzer,editor,explore,navigator}.rs 现已无引用（内置 agent 删了）
- src/agent/skills/resources/*.md (6 个) 仍被 src/core/extensions/marketplace.rs include_str! 引用
  → 它们是 /extension marketplace 的安装模板，装到 .star/skills/ 后成为用户自定义 skill
- 用户 .star/skills/ 目录扫描从未接线（既有缺陷）
Next action: 清理 prompts/skills 死资源 + 确认 marketplace 模板去留，然后接线用户 SKILL.md 目录加载
Continuation cues: 继续 / 接着做 / 照做
