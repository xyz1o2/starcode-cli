//! System Prompt 品牌类型与动态组装
//!
//! 对标 Claude Code 的 System Prompt 架构：
//! - 品牌类型防止普通 string[] 被意外传入 API
//! - 动态分界标记实现静态/动态区分离，优化 Prompt Cache
//! - Section 注册表管理动态区的内容注入

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

// ── Constants ──

/// 分界标记：将 System Prompt 分为"不变的静态区"和"因用户/会话而异的动态区"。
/// 静态区对所有用户相同，可获得跨组织缓存；动态区每次不同。
/// 该标记在发送给 API 前被移除，AI 永远看不到。
pub const SYSTEM_PROMPT_DYNAMIC_BOUNDARY: &str = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";

// ── Branded Type ──

/// System Prompt 品牌类型（对标 Claude Code 的 SystemPrompt branded type）
///
/// 使用品牌类型（branded type）防止普通 `Vec<String>` 被意外传入 API 调用。
/// 只有通过 `as_system_prompt()` 显式转换才能获得此类型。
#[derive(Debug, Clone)]
pub struct SystemPrompt {
    /// 内容段数组，每个元素是 System Prompt 的一个逻辑段
    segments: Vec<String>,
    /// 标记是否包含动态分界标记
    has_boundary: bool,
}

impl SystemPrompt {
    /// 创建新的 System Prompt
    pub fn new(segments: Vec<String>) -> Self {
        let has_boundary = segments.iter().any(|s| s.contains(SYSTEM_PROMPT_DYNAMIC_BOUNDARY));
        Self { segments, has_boundary }
    }

    /// 获取所有段
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// 转换为单个字符串（用于不支持数组的 API）
    pub fn to_string_lossy(&self) -> String {
        self.segments.join("\n\n")
    }

    /// 获取静态区内容（分界标记之前的部分）
    pub fn static_sections(&self) -> Vec<&str> {
        let mut result = Vec::new();
        for seg in &self.segments {
            if seg.contains(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
                // 只取分界标记之前的部分
                if let Some(idx) = seg.find(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
                    let before = &seg[..idx];
                    if !before.trim().is_empty() {
                        result.push(before.trim());
                    }
                }
                break;
            }
            result.push(seg.as_str());
        }
        result
    }

    /// 获取动态区内容（分界标记之后的部分）
    pub fn dynamic_sections(&self) -> Vec<&str> {
        let mut result = Vec::new();
        let mut past_boundary = false;

        for seg in &self.segments {
            if !past_boundary {
                if seg.contains(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
                    past_boundary = true;
                    // 取分界标记之后的部分
                    if let Some(idx) = seg.find(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
                        let after = &seg[idx + SYSTEM_PROMPT_DYNAMIC_BOUNDARY.len()..];
                        if !after.trim().is_empty() {
                            result.push(after.trim());
                        }
                    }
                }
                continue;
            }
            result.push(seg.as_str());
        }
        result
    }

    /// 是否包含动态分界标记
    pub fn has_boundary(&self) -> bool {
        self.has_boundary
    }

    /// 转换为 API 友好的 TextBlockParam 格式（带 cache_control 标记）
    ///
    /// 对标 Claude Code 的 buildSystemPromptBlocks()
    pub fn to_cache_blocks(&self) -> Vec<CacheBlock> {
        if !self.has_boundary {
            // 没有分界标记时，整个内容作为单个块
            return vec![CacheBlock {
                text: self.to_string_lossy(),
                cache_scope: CacheScope::Org,
            }];
        }

        let mut blocks = Vec::new();
        let mut static_text = String::new();
        let mut dynamic_text = String::new();
        let mut in_static = true;

        for seg in &self.segments {
            if seg.contains(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
                if let Some(idx) = seg.find(SYSTEM_PROMPT_DYNAMIC_BOUNDARY) {
                    let before = &seg[..idx];
                    if !before.trim().is_empty() {
                        if !static_text.is_empty() {
                            static_text.push_str("\n\n");
                        }
                        static_text.push_str(before.trim());
                    }
                    let after = &seg[idx + SYSTEM_PROMPT_DYNAMIC_BOUNDARY.len()..];
                    if !after.trim().is_empty() {
                        if !dynamic_text.is_empty() {
                            dynamic_text.push_str("\n\n");
                        }
                        dynamic_text.push_str(after.trim());
                    }
                }
                in_static = false;
                continue;
            }

            if in_static {
                if !static_text.is_empty() {
                    static_text.push_str("\n\n");
                }
                static_text.push_str(seg);
            } else {
                if !dynamic_text.is_empty() {
                    dynamic_text.push_str("\n\n");
                }
                dynamic_text.push_str(seg);
            }
        }

        // 静态区：可跨组织缓存
        if !static_text.is_empty() {
            blocks.push(CacheBlock {
                text: static_text,
                cache_scope: CacheScope::Global,
            });
        }

        // 动态区：组织级缓存或不缓存
        if !dynamic_text.is_empty() {
            blocks.push(CacheBlock {
                text: dynamic_text,
                cache_scope: CacheScope::Org,
            });
        }

        blocks
    }
}

/// 缓存作用域
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheScope {
    /// 不缓存
    None,
    /// 组织级缓存
    Org,
    /// 全局缓存（跨组织共享，仅 1P 可用）
    Global,
}

/// 带缓存控制的文本块
#[derive(Debug, Clone)]
pub struct CacheBlock {
    pub text: String,
    pub cache_scope: CacheScope,
}

// ── Section Registry ──

/// System Prompt Section 类型
#[derive(Debug, Clone)]
pub enum SectionType {
    /// 静态区：对所有用户相同，可跨组织缓存
    Static,
    /// 动态区：因用户/会话而异
    Dynamic,
}

/// System Prompt Section 定义
#[derive(Debug, Clone)]
pub struct PromptSection {
    /// Section 名称（唯一标识）
    pub name: String,
    /// Section 类型（静态/动态）
    pub section_type: SectionType,
    /// Section 内容（延迟计算）
    pub content: Option<String>,
    /// 是否已缓存
    pub cached: bool,
    /// 是否每轮重新计算（破坏缓存）
    pub cache_break: bool,
    /// 优先级（越小越靠前）
    pub priority: i32,
}

/// Section 注册表（对标 Claude Code 的 systemPromptSection / DANGEROUS_uncachedSystemPromptSection）
///
/// 管理 System Prompt 的动态区内容注入，支持：
/// - 缓存式 Section：计算一次，/clear 或 /compact 后才重新计算
/// - 危险 Section：每轮重新计算，会破坏 Prompt Cache
pub struct SectionRegistry {
    sections: Arc<RwLock<HashMap<String, PromptSection>>>,
}

impl SectionRegistry {
    pub fn new() -> Self {
        Self {
            sections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册一个缓存式 Section（计算一次，后续使用缓存）
    pub fn register_cached(
        &self,
        name: &str,
        section_type: SectionType,
        priority: i32,
        content_fn: impl FnOnce() -> String,
    ) {
        let content = content_fn();
        let section = PromptSection {
            name: name.to_string(),
            section_type,
            content: Some(content),
            cached: true,
            cache_break: false,
            priority,
        };
        if let Ok(mut sections) = self.sections.write() {
            sections.insert(name.to_string(), section);
        }
    }

    /// 注册一个动态 Section（每轮重新计算，会破坏 Prompt Cache）
    ///
    /// # Safety
    /// 使用此方法必须提供 `cache_break_reason`，说明为什么必须每轮重新计算。
    pub fn register_uncached(
        &self,
        name: &str,
        section_type: SectionType,
        priority: i32,
        cache_break_reason: &str,
        content_fn: impl FnOnce() -> String,
    ) {
        let content = content_fn();
        let section = PromptSection {
            name: name.to_string(),
            section_type,
            content: Some(content),
            cached: false,
            cache_break: true,
            priority,
        };
        if let Ok(mut sections) = self.sections.write() {
            sections.insert(name.to_string(), section);
        }
    }

    /// 获取所有 Section（按优先级排序）
    pub fn resolve_all(&self) -> Vec<PromptSection> {
        if let Ok(sections) = self.sections.read() {
            let mut list: Vec<PromptSection> = sections.values().cloned().collect();
            list.sort_by_key(|s| s.priority);
            list
        } else {
            Vec::new()
        }
    }

    /// 获取指定 Section
    pub fn get(&self, name: &str) -> Option<PromptSection> {
        if let Ok(sections) = self.sections.read() {
            sections.get(name).cloned()
        } else {
            None
        }
    }

    /// 清除所有缓存式 Section（用于 /clear 或 /compact）
    pub fn clear_cache(&self) {
        if let Ok(mut sections) = self.sections.write() {
            for section in sections.values_mut() {
                if section.cached {
                    section.content = None;
                }
            }
        }
    }

    /// 获取会破坏缓存的 Section 名称列表
    pub fn cache_breaking_sections(&self) -> Vec<String> {
        if let Ok(sections) = self.sections.read() {
            sections
                .values()
                .filter(|s| s.cache_break)
                .map(|s| s.name.clone())
                .collect()
        } else {
            Vec::new()
        }
    }
}

// ── Global Registry Instance ──

fn global_section_registry() -> &'static SectionRegistry {
    static REGISTRY: OnceLock<SectionRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| SectionRegistry::new())
}

/// 获取全局 Section 注册表
pub fn section_registry() -> &'static SectionRegistry {
    global_section_registry()
}

// ── Builder ──

/// System Prompt 构建器
///
/// 支持三级优先级（对标 Claude Code 的 buildEffectiveSystemPrompt）：
/// 1. Override：完全替换
/// 2. Custom：--system-prompt 参数指定
/// 3. Default：完整组装
pub struct SystemPromptBuilder;

impl SystemPromptBuilder {
    /// 构建默认 System Prompt（包含静态区 + 动态区 + 分界标记）
    pub fn build_default(static_segments: Vec<String>, dynamic_segments: Vec<String>) -> SystemPrompt {
        let mut segments = Vec::new();

        // 静态区
        segments.extend(static_segments);

        // 分界标记（仅在启用全局缓存时插入）
        if should_use_global_cache() {
            segments.push(SYSTEM_PROMPT_DYNAMIC_BOUNDARY.to_string());
        }

        // 动态区
        segments.extend(dynamic_segments);

        SystemPrompt::new(segments)
    }

    /// 构建 Override System Prompt（完全替换）
    pub fn build_override(override_content: String) -> SystemPrompt {
        SystemPrompt::new(vec![override_content])
    }

    /// 构建 Custom System Prompt（替换默认，但保留 append）
    pub fn build_custom(custom_content: String, append_segments: Vec<String>) -> SystemPrompt {
        let mut segments = vec![custom_content];
        segments.extend(append_segments);
        SystemPrompt::new(segments)
    }
}

/// 是否使用全局缓存（对标 Claude Code 的 shouldUseGlobalCacheScope）
fn should_use_global_cache() -> bool {
    // 默认启用，除非明确禁用
    std::env::var("STAR_PROMPT_CACHE_ENABLED")
        .ok()
        .map(|v| {
            let v = v.trim().to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_sections() {
        let sp = SystemPrompt::new(vec![
            "Static content 1".to_string(),
            "Static content 2".to_string(),
            format!("Before{}After", SYSTEM_PROMPT_DYNAMIC_BOUNDARY),
            "Dynamic content".to_string(),
        ]);

        assert!(sp.has_boundary());
        let statics = sp.static_sections();
        assert_eq!(statics.len(), 3); // "Static 1", "Static 2", "Before"
        let dynamics = sp.dynamic_sections();
        assert_eq!(dynamics.len(), 2); // "After", "Dynamic content"
    }

    #[test]
    fn test_cache_blocks() {
        let sp = SystemPrompt::new(vec![
            "Static intro".to_string(),
            format!("Static end{}Dynamic start", SYSTEM_PROMPT_DYNAMIC_BOUNDARY),
            "Dynamic content".to_string(),
        ]);

        let blocks = sp.to_cache_blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].cache_scope, CacheScope::Global);
        assert_eq!(blocks[1].cache_scope, CacheScope::Org);
    }

    #[test]
    fn test_section_registry() {
        let registry = SectionRegistry::new();
        registry.register_cached("test", SectionType::Dynamic, 10, || "cached content".to_string());
        
        let section = registry.get("test").unwrap();
        assert_eq!(section.content.unwrap(), "cached content");
        assert!(section.cached);
    }
}
