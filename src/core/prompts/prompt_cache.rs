//! Prompt Cache 优化模块
//!
//! 对标 Claude Code 的 Prompt Cache 策略：
//! - 将 System Prompt 分为静态区和动态区
//! - 静态区标记为 `scope: 'global'`（跨组织缓存）
//! - 动态区标记为 `scope: 'org'`（组织级缓存）
//! - 支持 TTL 决策（1h 缓存）

use std::sync::OnceLock;

// ── 配置 ──

/// Prompt Cache 是否启用
fn prompt_cache_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("STAR_PROMPT_CACHE_ENABLED")
            .ok()
            .map(|v| {
                let v = v.trim().to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(true)
    })
}

/// 是否使用全局缓存（仅 1P 可用）
fn should_use_global_cache() -> bool {
    // 默认启用，除非明确禁用
    std::env::var("STAR_PROMPT_CACHE_GLOBAL")
        .ok()
        .map(|v| {
            let v = v.trim().to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true)
}

/// 缓存 TTL（秒）
fn cache_ttl_secs() -> u64 {
    std::env::var("STAR_PROMPT_CACHE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3600) // 默认 1 小时
}

// ── 缓存块 ──

/// 缓存作用域
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheScope {
    /// 不缓存
    None,
    /// 组织级缓存
    Org,
    /// 全局缓存（跨组织共享）
    Global,
}

/// 带缓存控制的文本块
#[derive(Debug, Clone)]
pub struct CacheControlBlock {
    /// 文本内容
    pub text: String,
    /// 缓存作用域
    pub scope: CacheScope,
    /// 是否为分界标记
    pub is_boundary: bool,
}

/// 缓存控制参数（用于 API 请求）
#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl CacheControl {
    /// 创建 ephemeral 缓存控制
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
            ttl: None,
            scope: None,
        }
    }

    /// 创建带 TTL 的缓存控制
    pub fn with_ttl(ttl_secs: u64) -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
            ttl: Some(format!("{}s", ttl_secs)),
            scope: None,
        }
    }

    /// 创建带作用域的缓存控制
    pub fn with_scope(scope: CacheScope) -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
            ttl: None,
            scope: match scope {
                CacheScope::None => None,
                CacheScope::Org => Some("org".to_string()),
                CacheScope::Global => Some("global".to_string()),
            },
        }
    }

    /// 创建带 TTL 和作用域的缓存控制
    pub fn with_ttl_and_scope(ttl_secs: u64, scope: CacheScope) -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
            ttl: Some(format!("{}s", ttl_secs)),
            scope: match scope {
                CacheScope::None => None,
                CacheScope::Org => Some("org".to_string()),
                CacheScope::Global => Some("global".to_string()),
            },
        }
    }
}

// ── System Prompt 分块 ──

/// System Prompt 分界标记
pub const SYSTEM_PROMPT_DYNAMIC_BOUNDARY: &str = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";

/// 将 System Prompt 分为静态区和动态区
///
/// 返回 (static_text, dynamic_text)
pub fn split_system_prompt(system_prompt: &str) -> (String, String) {
    if let Some(idx) = system_prompt.find(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
        let static_part = system_prompt[..idx].trim().to_string();
        let dynamic_part = system_prompt[idx + SYSTEM_PROMPT_DYNAMIC_BOUNDARY.len()..]
            .trim()
            .to_string();
        (static_part, dynamic_part)
    } else {
        (system_prompt.to_string(), String::new())
    }
}

/// 为 System Prompt 生成缓存控制块
///
/// 对标 Claude Code 的 buildSystemPromptBlocks()
pub fn build_cache_blocks(system_prompt: &str) -> Vec<CacheControlBlock> {
    if !prompt_cache_enabled() {
        return vec![CacheControlBlock {
            text: system_prompt.to_string(),
            scope: CacheScope::None,
            is_boundary: false,
        }];
    }

    let (static_text, dynamic_text) = split_system_prompt(system_prompt);

    let mut blocks = Vec::new();

    // 静态区：可跨组织缓存
    if !static_text.is_empty() {
        blocks.push(CacheControlBlock {
            text: static_text,
            scope: if should_use_global_cache() {
                CacheScope::Global
            } else {
                CacheScope::Org
            },
            is_boundary: false,
        });
    }

    // 动态区：组织级缓存
    if !dynamic_text.is_empty() {
        blocks.push(CacheControlBlock {
            text: dynamic_text,
            scope: CacheScope::Org,
            is_boundary: false,
        });
    }

    blocks
}

/// 获取缓存控制参数
pub fn get_cache_control(scope: &CacheScope) -> Option<CacheControl> {
    if !prompt_cache_enabled() {
        return None;
    }

    match scope {
        CacheScope::None => None,
        CacheScope::Org => Some(CacheControl::with_scope(CacheScope::Org)),
        CacheScope::Global => {
            if should_use_global_cache() {
                Some(CacheControl::with_scope(CacheScope::Global))
            } else {
                Some(CacheControl::with_scope(CacheScope::Org))
            }
        }
    }
}

/// 获取带 TTL 的缓存控制参数
pub fn get_cache_control_with_ttl(scope: &CacheScope) -> Option<CacheControl> {
    if !prompt_cache_enabled() {
        return None;
    }

    let ttl = cache_ttl_secs();

    match scope {
        CacheScope::None => None,
        CacheScope::Org => Some(CacheControl::with_ttl_and_scope(ttl, CacheScope::Org)),
        CacheScope::Global => {
            if should_use_global_cache() {
                Some(CacheControl::with_ttl_and_scope(ttl, CacheScope::Global))
            } else {
                Some(CacheControl::with_ttl_and_scope(ttl, CacheScope::Org))
            }
        }
    }
}

// ── 消息缓存标记 ──

/// 为消息添加缓存控制标记
///
/// 对标 Claude Code 的 cache_control 字段
pub fn add_cache_control_to_message(message: &mut serde_json::Value, scope: CacheScope) {
    if !prompt_cache_enabled() {
        return;
    }

    if let Some(content) = message.get_mut("content") {
        if let Some(content_str) = content.as_str() {
            // 单文本内容：直接添加 cache_control
            let cache_control = get_cache_control(&scope);
            if let Some(cc) = cache_control {
                *content = serde_json::json!({
                    "type": "text",
                    "text": content_str,
                    "cache_control": cc
                });
            }
        } else if let Some(content_array) = content.as_array_mut() {
            // 数组内容：为最后一个元素添加 cache_control
            if let Some(last) = content_array.last_mut() {
                let cache_control = get_cache_control(&scope);
                if let Some(cc) = cache_control {
                    if let Some(obj) = last.as_object_mut() {
                        obj.insert(
                            "cache_control".to_string(),
                            serde_json::to_value(cc).unwrap(),
                        );
                    }
                }
            }
        }
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_system_prompt_with_boundary() {
        let prompt = format!(
            "Static content here\n{}\nDynamic content here",
            SYSTEM_PROMPT_DYNAMIC_BOUNDARY
        );
        let (static_text, dynamic_text) = split_system_prompt(&prompt);
        assert_eq!(static_text, "Static content here");
        assert_eq!(dynamic_text, "Dynamic content here");
    }

    #[test]
    fn test_split_system_prompt_without_boundary() {
        let prompt = "All content here";
        let (static_text, dynamic_text) = split_system_prompt(prompt);
        assert_eq!(static_text, "All content here");
        assert!(dynamic_text.is_empty());
    }

    #[test]
    fn test_build_cache_blocks() {
        let prompt = format!(
            "Static content\n{}\nDynamic content",
            SYSTEM_PROMPT_DYNAMIC_BOUNDARY
        );
        let blocks = build_cache_blocks(&prompt);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].scope, CacheScope::Global);
        assert_eq!(blocks[1].scope, CacheScope::Org);
    }

    #[test]
    fn test_cache_control_serialization() {
        let cc = CacheControl::with_ttl_and_scope(3600, CacheScope::Global);
        let json = serde_json::to_value(&cc).unwrap();
        assert_eq!(json["type"], "ephemeral");
        assert_eq!(json["ttl"], "3600s");
        assert_eq!(json["scope"], "global");
    }
}
