[COMPACTION-RECOVERY]
Route: ct-attack-defense (engineering executor)
Active target: /mnt/h/test/starcode (branch dev-m)
Goal: 本轮已完成 —— 用户 SKILL.md frontmatter 语义 + 权限弹窗收敛

Exact identifiers/evidence:
- 根因修复（分裂缓存）：f0210bc + 文档 b8a80d9
- 内置 skill 删除：b8fa838 / 6ecbffd；用户 SKILL.md 接线：7c9eb08
- frontmatter 语义实现：9dfb8cb → 3e624d3 → 6bcf8bf → f4470fa → 08ce855 → 9a25e2d
- 文档：0a88265（frontmatter 实现说明）、5bc77a0（内置删除说明）
- 权限弹窗收敛：d3d4ab2（分类器熔断 + 判词解析 + 双门闸）
- 门禁基线: 1977 passed / 0 failed / 4 ignored

本轮用户的两条要求（都已完成）:
1. 「✓ Allowed 这种不需要显示」
   → confirmation_dialog.rs 两处渲染器去掉裁决文字，只留图标（+ 内容摘要）
2. 「现在的权限老是弹窗」
   → 三个独立缺陷，都用 ARTEX 日志实证：
   a) chat() 忽略 model 参数 → 分类器永远用主模型 → 主模型回散文 →
      判词 unrecognized → 熔断 → 每次工具调用都问人（日志 12 次熔断）
      修：chat() 尊重 model 覆盖，client_for_model() 造临时 client
   b) 判词解析只认裸首词 → `**ALLOW**` / `Verdict: ALLOW` / 围栏 全算 unrecognized
      修：extract_verdict_word() 剥 markdown 装饰 + 跳标签词（前 4 词内找）
      否定与矛盾护栏不变（认不出 = 问人，绝不放行）
   c) 工具层与策略层是两道独立的门，问同一件事 → 已授权还再问
      修：工具层在已有会话/持久授权时不再单独弹；
      但 requires_invocation_confirmation 为真时不跳（那是路径级顾虑）

刻意**没有**做的事:
- 没有放宽 PolicyEngine 的「分类器不可用 = 问人」默认 —— 那是安全属性，
  既有测试 auto_mode_asks_when_no_classifier_is_installed 锁着。
  （我一度改松了它，导致该测试失败，已回退。）

用户决策记录:
- 「完全删除源码」的内置 skills，但「只去掉内置，保留用户自定义」
- MCP 一度被误删 6477 行，已完整回滚（剩余 diff 中 MCP 改动 = 0 行）

Technical scope:
- exa MCP 恢复了（此前长时间 fetch failed）；Claude Code 官方文档核实过
  skills/MCP 分层、frontmatter 参考、权限判定顺序（deny→ask→allow）
- 本地参考副本 .study/study_or_copy_projects/claude-code-main

Deliverable checkpoints:
- [x] 分裂缓存根因修复
- [x] 内置 skill 源码删除 + 用户 skill 目录接通
- [x] SKILL.md frontmatter 全字段语义
- [x] ✓ Allowed 文案移除
- [x] 权限弹窗三处收敛

Current stage & primary technique: 收尾 —— 文档与最终复核
Observed ledger:
- 根因 C（索引截断时引擎不落地）仍未解决，见
  docs/codebase-search-index-incident.zh-CN.md §5.1（ARTEX 未触发截断，无样本）
- 权限方面还有一个未动的点：敏感文件（.env）每次都问是有意设计
  （sensitive_paths 模块头明确写了「会话授权不能降级它」），
  用户若嫌吵可加 STAR_SENSITIVE_FILE_ASK=0
Next action: 写 docs/permission-prompt-reduction.zh-CN.md 记录三个缺陷与修法
Continuation cues: 继续 / 接着做 / 照做
