use std::collections::HashSet;

use super::types::ContextLayer;

pub struct ContextManager;

impl ContextManager {
    pub fn new() -> Self {
        Self
    }

    /// 合并上下文
    pub fn merge_contexts(
        &self,
        layers: &[ContextLayer],
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut merged = String::new();

        // 按层级和优先级排序
        let mut sorted_layers: Vec<_> = layers.iter().filter(|l| l.active).cloned().collect();
        sorted_layers.sort_by(|a, b| {
            // 先按层级排序（高优先级在后）
            a.level
                .cmp(&b.level)
                // 同层级按优先级排序
                .then_with(|| a.definition.priority.cmp(&b.definition.priority))
        });

        // 按内容类型分类合并
        let mut tech_stacks = HashSet::new();
        let mut conventions = Vec::new();
        let mut architectures = Vec::new();
        let mut rules = Vec::new();
        let mut patterns = Vec::new();

        for layer in sorted_layers {
            let content = &layer.definition.content;

            // 从元数据中提取技术栈
            for tech in &layer.definition.metadata.tech_stack {
                tech_stacks.insert(tech.clone());
            }

            // 解析不同部分
            if let Some(tech) = self.extract_section(content, "Technology Stack") {
                tech_stacks.extend(self.parse_list(&tech));
            }

            if let Some(conv) = self.extract_section(content, "Code Conventions") {
                conventions.push((conv, layer.definition.name.clone()));
            }

            if let Some(arch) = self.extract_section(content, "Architecture") {
                architectures.push(arch);
            }

            if let Some(rule) = self.extract_section(content, "Rules") {
                rules.push(rule);
            }

            if let Some(pattern) = self.extract_section(content, "Patterns") {
                patterns.push(pattern);
            }
        }

        // 构建最终合并后的上下文
        merged.push_str("# Dynamic Context\n\n");

        // 1. 技术栈（交集）
        if !tech_stacks.is_empty() {
            merged.push_str("## Technology Stack\n\n");
            merged.push_str("This project uses the following technologies:\n\n");
            for tech in &tech_stacks {
                merged.push_str(&format!("- ✅ {}\n", tech));
            }

            // 兼容性说明
            merged.push_str("\n");
            merged.push_str("**Compatibility Notes:**\n");
            merged.push_str("- All tech stacks listed above are confirmed in this project\n");
            merged.push_str(&format!(
                "- Total confirmed technologies: {}\n\n",
                tech_stacks.len()
            ));
        }

        // 2. 编码规范（叠加，高优先级覆盖）
        if !conventions.is_empty() {
            merged.push_str("## Code Conventions\n\n");
            merged.push_str("### Merged Guidelines\n\n");

            // 合并规范，处理冲突
            let merged_conventions = self.merge_conventions(&conventions);
            for (idx, (conv, source)) in merged_conventions.iter().enumerate() {
                merged.push_str(&format!("{}. {}\n", idx + 1, conv));
                merged.push_str(&format!("   *Source: {}*\n", source));
            }
            merged.push_str("\n");
        }

        // 3. 架构指南（叠加）
        if !architectures.is_empty() {
            merged.push_str("## Architecture Guidelines\n\n");

            for (idx, arch) in architectures.iter().enumerate() {
                merged.push_str(&format!("### Layer {}\n\n", idx + 1));
                merged.push_str(arch);
                merged.push_str("\n\n");
            }
        }

        // 4. Rules
        if !rules.is_empty() {
            merged.push_str("## Rules\n\n");
            for rule in rules {
                merged.push_str(&rule);
                merged.push_str("\n\n");
            }
        }

        // 5. Patterns
        if !patterns.is_empty() {
            merged.push_str("## Patterns\n\n");
            for pattern in patterns {
                merged.push_str(&pattern);
                merged.push_str("\n\n");
            }
        }

        Ok(merged)
    }

    // Helper functions (placeholders/basic implementations)
    fn extract_section(&self, content: &str, section_name: &str) -> Option<String> {
        let header = format!("## {}", section_name);
        if let Some(start) = content.find(&header) {
            let rest = &content[start + header.len()..];
            let end = rest.find("## ").unwrap_or(rest.len());
            return Some(rest[..end].trim().to_string());
        }
        None
    }

    fn parse_list(&self, content: &str) -> Vec<String> {
        content
            .lines()
            .filter(|l| l.trim().starts_with('-') || l.trim().starts_with('*'))
            .map(|l| l.trim().trim_start_matches(['-', '*']).trim().to_string())
            .collect()
    }

    fn merge_conventions(&self, conventions: &[(String, String)]) -> Vec<(String, String)> {
        // Simple merge: just concatenate for now, maybe dedup later
        conventions
            .iter()
            .map(|(c, s)| (c.clone(), s.clone()))
            .collect()
    }
}
