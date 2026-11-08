[COMPACTION-RECOVERY]
Route: ct-attack-defense（本仓实为 Rust 工程接线任务）
Active target: /mnt/h/test/starcode（branch sc-dev-m）
Goal: 四点风险清理 + parity.rs 分拆 交付完毕；终验全量回归中（任务 bjlh8ciik）
Exact identifiers/evidence: 已删 src/constants src/sdk tests/子目录（git rm 41+ 文件）；docs/archive/ 8 份归档；star_env_registry.rs 两条对账测试；parity/ 11 子模块 + 主文件 218 行转发
Bound slots: 无占位符
Technical scope: local | repo-only
Deliverable checkpoints:
  - 删除类: ✅（回归 2080 passed 拍板过一次）
  - STAR_* 对账门: ✅ documented_star_envs_are_actually_consumed + claude_md_star_env_mentions_are_real 双绿（含通配/内部符号豁免分支）
  - parity 分拆: ✅ 主文件 216 行 + 11 子模块（diag 2128 最大）；lib 测试 2091 passed 过一轮；clippy parity/ 0 新警告（collapsible_str_replace + is_multiple_of 已修）；Verifier/Check/Step pub(crate) 化
  - CLAUDE.md: ✅ 四段同步（env 对账门 / 孤儿目录删 / tests 清理 / parity 分拆结构 + docs 归档规则）
Next action: 读 bjlh8ciik（期待 ~2092 passed / FMT_GREEN）→ 终报（含 CC env 管理对标结论：文档即注册表 + env block 缺口列为未来工作项）
Continuation cues: continue / 继续 / 照做
