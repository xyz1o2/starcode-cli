//! STAR_* 环境变量对账门（对标 CC 的 env-vars 文档管理模型）。
//!
//! CC 的做法（官方 `env-vars` 文档页实抓）：**文档页就是 registry** —— 每个被
//! 管理的 `CLAUDE_CODE_*` 变量在页上有专行（语义 / 默认值 / 与 CLI flag 和
//! settings 键的优先级关系），代码端没有中央注册表，散落使用点是刻意的。
//! 对账靠的是这个约定：页上列出的变量**必须真的被消费**，页上没有的不承诺覆盖。
//!
//! 本模块把同一约定落成 CI 门：[`DOCUMENTED_STAR_ENVS`] 列出文档级集合
//! （配合根目录 `.star_env_registry.py`，人读版），对账测试保证——
//! 1. 文档级清单里的每一个变量在 `src/` 下至少有一个 `include_str!grep`
//!    可见的消费点（反向漂移：文档夸大）；
//! 2. CLAUDE.md 内文引用过的 STAR_* 变量都在文档级清单或代码中（防提及漂移）。
//!
//! 维护：新增高价值变量 → 在两处（py + 本表）加一行；删变量 → 对账测试会
//! 用真实 grep 证据拦住你，先看消费点再动手。
//!
//! 刻意**不**做全量 357 个变量的注册表：CC 也没那么做（官方页 281 个带专行，
//! 内部 flag 不在页上），全量清单的维护成本会压垮它，对账链的价值在
//! 「文档承诺 = 代码现实」这个子集上。

/// 文档级 STAR_* 清单（语义分工见 `.star_env_registry.py` 顶部注释）。
pub const DOCUMENTED_STAR_ENVS: &[&str] = &[
    // 通道/提供方（provider_resolution 的 SourceRef 链最高优先级）
    "STAR_API_KEY",
    "STAR_BASE_URL",
    "STAR_MODEL",
    // 运行时门禁
    "STAR_AUTO_COMPACT",
    "STAR_CONTEXT_WINDOW",
    "STAR_ENABLE_SKILL_TOOL",
    "STAR_DISABLE_SKILL_TOOL",
    "STAR_FORK_AGENT_ENABLED",
    "STAR_LOG_ENABLED",
    "STAR_LOG_DIR",
    "STAR_PROMPT_DIR",
    "STAR_TOOL_TIMEOUT_SECS",
    // AgentMail 局域网互通（docs/agent-messaging-design.zh-CN.md §7）
    "STAR_AGENT_MAIL_ENABLED",
    "STAR_AGENT_MAIL_HOST",
    "STAR_AGENT_MAIL_KEY",
    "STAR_AGENT_MAIL_PORT",
    "STAR_AGENT_PEERS",
    // 自调参防护（core/self_tuning）
    "STAR_SELFTUNING_LOCKED",
    "STAR_SELFTUNING_MAX_SETS",
    // 模型回退（agent/model_fallback.rs）
    "STAR_MODEL_FALLBACK_ENABLED",
    "STAR_MODEL_FALLBACK_MAX_RETRIES",
    "STAR_FALLBACK_MODEL",
    "STAR_FALLBACK_MODELS",
    // 调试
    "STAR_REMOTE_DRAIN_SECS",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// 汇集 src/（与 crates 的 bin）下全部 .rs 源文本，供变量消费点检查。
    fn collect_sources() -> String {
        let mut entries = Vec::new();
        collect_sources_under(Path::new("src"), &mut entries);
        entries.join("\n")
    }

    fn collect_sources_under(dir: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_sources_under(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    out.push(content);
                }
            }
        }
    }

    /// 对账 1：文档级清单里的每个变量必须在 src/ 里真实消费
    /// （字符串字面量可见）。文档夸大 = CI 红。
    #[test]
    fn documented_star_envs_are_actually_consumed() {
        let sources = collect_sources();
        assert!(
            !sources.is_empty(),
            "src/ 源码收集失败 —— 对账测试依赖仓库相对路径运行（cargo test 在仓库根跑）"
        );
        let ghost: Vec<&str> = DOCUMENTED_STAR_ENVS
            .iter()
            .filter(|name| !sources.contains(&format!("\"{name}\"")))
            .copied()
            .collect();
        assert!(
            ghost.is_empty(),
            "文档级清单里有变量已经没有任何消费点了: {:?}。\
             要么删掉清单行（变量下线），要么恢复消费点。",
            ghost
        );
    }

    /// 对账 2：CLAUDE.md 正文提及的 STAR_* 都有真实消费点。文档提到却
    /// 没人读，是第 2 类风险（行为与文档不一致）的常见入口。
    #[test]
    fn claude_md_star_env_mentions_are_real() {
        let sources = collect_sources();
        let Ok(md) = std::fs::read_to_string("CLAUDE.md") else {
            return; // 无 CLAUDE.md 的环境跳过（CI 一定有）
        };
        let mut mentioned = std::collections::HashSet::new();
        for m in md.match_indices("STAR_") {
            let rest = &md[m.0..];
            let raw: String = rest
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            // 尾下划线的语义在收集时就分型（后面 filter 依赖它）：
            // `STAR_X_` = 文档通配写法（变量族前缀，即 `STAR_X_*`）；
            // `STAR_X`（无尾 `_`）= 完整变量名。
            let mut name = raw.clone();
            if name.ends_with('_') {
                // 多级通配 `STAR_A_B_` 只留一级标记，前缀对账按 `STAR_A_B` 起
                while name.len() > 6 && name.ends_with("__") {
                    name.pop();
                }
            } else if name.len() > 5 {
                // 完整名直接记录
            } else {
                continue;
            }
            if name.len() > 5 {
                mentioned.insert(name);
            }
        }
        let ghost: Vec<&String> = mentioned
            .iter()
            .filter(|name| {
                // 三种合法形态：
                // ① 完整变量名 —— `"STAR_X"` 精确消费字面量；
                // ② 文档通配写法 `STAR_X_*`（收集时保留为尾 `_`）—— 变量族：
                //    任一带前缀的实例被消费即真（查**子串**，消费点是
                //    `"STAR_X_ENABLED"` 这类更长串）；
                // ③ 文档提及的是**仓库内部符号**（如 `DOCUMENTED_STAR_ENVS`
                //    常量名）—— 不以引号字面量形式出现在消费点，查裸名。
                if sources.contains(&format!("\"{name}\"")) {
                    return false;
                }
                if let Some(prefix) = name.strip_suffix('_') {
                    return !sources.contains(&format!("\"{prefix}"));
                }
                !sources.contains(name.as_str())
            })
            .collect();
        assert!(
            ghost.is_empty(),
            "CLAUDE.md 提到但代码无消费点的 STAR_*: {:?}。更新文档或恢复实现。",
            ghost
        );
    }
}
