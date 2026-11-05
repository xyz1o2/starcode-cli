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

---

[COMPACTION-RECOVERY] 2026-11-05 · prompt/tool 闭环 + 缓存边界（本次会话）
Route: ct-attack-defense | Active target: src/agent/{prompt_builder,reference_guards}, src/core/prompts, src/agent/compact | Goal: BackgroundTask 漂移修复 + 提示词/工具一致性闭环 + system 消息缓存边界拆分

Exact identifiers/evidence:
- 已随 43bf157 提交（另一会话合并提交，含确认框重构）
- 根因: bash.md 两处 `BackgroundTask` 指引 + 未声明工具 + bundle 静默截断（cap 26k 撞线）
- 修复: md×6 + reference_guards.rs(4 守卫测试) + assemble_core_bundle 单实现 + strip_frontmatter pub(crate) + BUNDLE_STATIC_FILE_NAMES + build_two_layer_registry_for_tests
- 缓存: system 拆 [静态块(cache_control) + 准静态块]；rig apply_system_cache_control 保留既有标记只补最后一块（已读源码验证）
- 压缩: snip/auto_compact + context_compression 只保 messages[0] → 改为整段前缀（leading_system_count/push_leading_system_messages in message_processing.rs）+ 回归测试
- bundle: frontmatter 剥离(~3k chars) + cap 26000→28000 + dropped 日志
- 本机预存环境失败 29→37（另一会话 tool_executor/policy_engine 确认框重构所致，与本工作无关，已用基线对照验证）
- 我的范围测试: 107 passed / 0 failed（prompt_builder/reference_guards/compact prefix/context_compression/prompt_cache_ext/prompts/tool_search/tool_name_consistency）

Next action: 若继续，仅两件事待办: (1) 等 policy/tool-executor 重构收敛后全量 cargo test 复核; (2) CLAUDE.md 可补 3 行缓存边界说明（未做，避免与另一会话冲突）
