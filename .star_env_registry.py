# STAR_* 环境变量注册表（对标 CLAUDE_CODE_* 的管理模型）
#
# CC 的做法（官方 env-vars 页 · 2026-11 实抓）：每个环境变量一个**专行条目**，
# 文档就是 registry —— 变量的默认值、优先级规则（vs CLI flag / settings 键）、
# 交互限制（project 级禁止设置哪些）全部落在那一行。代码端没有中央注册表。
#
# 本仓库沿用同一哲学：**使用点内联解析为主**（kernel §0 的 ARCHITECTURE 决策），
# 但 CC 做了而本仓库缺的是这条**对账链**：文档侧有清单（本文件），
# CI 侧钉住「文档清单 = 代码实际消费」两个集合，漂移即红。
#
# 维护规则：
# - 新增消费点（`std::env::var("STAR_…")`）后，把变量名加进本文件的
#   DOCUMENTED 列表并写一行用途；同名多文件统一到最早声明处。
# - **不选**「删除一个还在被读的变量」——对账脚会拦住你，看 unset 之处。
# - 分类字段是给将来做文档页排序用的，不参与对账。

# 已文档化的变量（必须与代码消费集完全一致）
DOCUMENTED = [
    # ── 通道/提供方（provider_resolution.rs 的 SourceRef 链最高优先级）──
    "STAR_API_KEY",
    "STAR_BASE_URL",
    "STAR_MODEL",
    # ── 运行时门禁 ──
    "STAR_AUTO_COMPACT",
    "STAR_CONTEXT_WINDOW",
    "STAR_ENABLE_SKILL_TOOL",
    "STAR_DISABLE_SKILL_TOOL",
    "STAR_FORK_AGENT_ENABLED",
    "STAR_LOG_ENABLED",
    "STAR_LOG_DIR",
    "STAR_PROMPT_DIR",
    "STAR_TOOL_TIMEOUT_SECS",
    # ── AgentMail 局域网互通（docs/agent-messaging-design.zh-CN.md §7）──
    "STAR_AGENT_MAIL_ENABLED",
    "STAR_AGENT_MAIL_HOST",
    "STAR_AGENT_MAIL_KEY",
    "STAR_AGENT_MAIL_PORT",
    "STAR_AGENT_PEERS",
    # ── 自调参防护（core/self_tuning）──
    "STAR_SELFTUNING_LOCKED",
    "STAR_SELFTUNING_MAX_SETS",
    # ── 模型回退（agent/model_fallback.rs）──
    "STAR_MODEL_FALLBACK_ENABLED",
    "STAR_MODEL_FALLBACK_MAX_RETRIES",
    "STAR_FALLBACK_MODEL",
    "STAR_FALLBACK_MODELS",
    # ── 调试 ──
    "STAR_REMOTE_DRAIN_SECS",
]

# （占位：将来若加 STAR_* 新分类，用 dict 替代 list；CI 对账只看集合。）

"""
维护方式（CI 对账的后半段在 src/core/star_env_registry.rs 的测试里，
两个来源——本文件 DOCUMENTED 与代码里 grep 出的全部 STAR_* 字面量——
不一致时那条测试失败，修法是双向：要么把新变量写进这里，
要么删掉代码里已经没人读的变量（并同步 CLAUDE.md 的清单（若有））。
"""
