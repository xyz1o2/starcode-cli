[COMPACTION-RECOVERY]
Route: ct-attack-defense（本仓实为 Rust 工程接线任务）
Active target: /mnt/h/test/starcode（branch sc-dev-m）
Goal: Explore 对标优化落地完毕（路由默认后台 + 只读工具白名单）；等全量回归拍板
Exact identifiers/evidence: router.rs(规则 2.5 explorer 默认 AsyncAgent)/ runner.rs(EXPLORER_READONLY_TOOLS 常量 + 通用路径 sub_config.core_tools + child_config 同收口)/ docs §8 差距账
Bound slots: 无占位符
Technical scope: local | repo-only
Deliverable checkpoints:
  - (②a) Explore 默认后台: ✅ 规则 2.5（显式 background=false 保留同步）；测试 explorer_defaults_to_background_but_respects_explicit_sync 绿
  - (②b) explorer 只读白名单: ✅ EXPLORER_READONLY_TOOLS（Read/Grep/Glob/LS/Bash/ReadManyFiles/CodebaseSearch/CodeGraph/RepoMap/NotebookRead/TodoWrite/AskUserQuestion）；两条派发路径都收口；测试 explorer_readonly_whitelist_excludes_mutating_tools 绿
  - 注册名核对: CodeGraph/RepoMap/ReadManyFiles 与 constants.rs 一致 ✅
  - 发现: Config.allowed_tools/exclude_tools 全仓零消费点（仅次于 /agent-mail 的又一处「存在未接线」），已在对话中说明
  - 全量回归 bp739snqj 运行中
Observed ledger: 新测试 2/2 绿; fmt 绿; 编译绿
Next action: 读 bp739snqj（期待 ~2077 passed）→ 汇报 Explore 优化完成；显示层此前已对标完毕无需改动（search_read_summary_text / render_done_line / @name / ctrl+o）
Continuation cues: continue / 继续 / 照做
