use std::path::Path;
use walkdir::WalkDir;

use super::types::{ContextDefinition, ContextSource};

pub struct ContextFinder;

impl ContextFinder {
    pub fn new() -> Self {
        Self
    }

    pub fn has_dynamic_context_candidates(&self, project_root: &Path) -> bool {
        let project_star_context = project_root.join(".star").join("context");
        if has_context_files(&project_star_context, true) {
            return true;
        }

        if let Some(user_home) = dirs::home_dir() {
            let user_contexts_dir = user_home.join(".star").join("contexts");
            if has_context_files(&user_contexts_dir, false) {
                return true;
            }
        }

        false
    }

    pub fn find_all_contexts(
        &self,
        project_root: &Path,
    ) -> Result<Vec<ContextDefinition>, Box<dyn std::error::Error>> {
        let mut contexts = Vec::new();

        // 1. Workspace-scoped contexts live under .star/context. Root-level
        // instruction files like CLAUDE.md are loaded directly by PromptBuilder.
        let project_star_context = project_root.join(".star").join("context");
        if project_star_context.exists() {
            let star_contexts = self.find_workspace_contexts(
                &project_star_context,
                ContextSource::WorkspaceConfig(project_star_context.to_string_lossy().to_string()),
            )?;
            contexts.extend(star_contexts);
        }

        // 2. User Global
        if let Some(user_home) = dirs::home_dir() {
            let user_contexts_dir = user_home.join(".star").join("contexts");
            if user_contexts_dir.exists() {
                let user_contexts = self.find_in_dir(
                    &user_contexts_dir,
                    ContextSource::UserGlobal(user_contexts_dir.to_string_lossy().to_string()),
                )?;
                contexts.extend(user_contexts);
            }
        }

        // 3. System (Simplified for now)
        // ...

        Ok(contexts)
    }

    fn find_workspace_contexts(
        &self,
        dir: &Path,
        source: ContextSource,
    ) -> Result<Vec<ContextDefinition>, Box<dyn std::error::Error>> {
        let mut contexts = Vec::new();
        for entry in WalkDir::new(dir)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() || !is_supported_context_extension(path) {
                continue;
            }
            if !is_workspace_context_file(path) {
                continue;
            }
            if let Ok(context) = self.load_context(path, &source) {
                contexts.push(context);
            }
        }
        Ok(contexts)
    }

    fn find_in_dir(
        &self,
        dir: &Path,
        source: ContextSource,
    ) -> Result<Vec<ContextDefinition>, Box<dyn std::error::Error>> {
        let mut contexts = Vec::new();
        for entry in WalkDir::new(dir)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() && is_supported_context_extension(path) {
                if let Ok(context) = self.load_context(path, &source) {
                    contexts.push(context);
                }
            }
        }
        Ok(contexts)
    }

    fn load_context(
        &self,
        path: &Path,
        source: &ContextSource,
    ) -> Result<ContextDefinition, Box<dyn std::error::Error>> {
        let raw_content = std::fs::read_to_string(path)?;
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let id = uuid::Uuid::new_v4().to_string(); // Or derive from path hash

        // 解析 Frontmatter
        let (metadata_map, content) = self.parse_frontmatter(&raw_content);

        let mut def = ContextDefinition::new(id, name, content);
        def.metadata.source = source.clone();

        // 填充元数据
        if let Some(val) = metadata_map.get("name") {
            def.name = val.to_string();
        }
        if let Some(val) = metadata_map.get("description") {
            def.description = val.to_string();
        }
        if let Some(val) = metadata_map.get("priority") {
            if let Ok(p) = val.parse::<i32>() {
                def.priority = p;
            }
        }

        // 解析列表类型的元数据 (简单的逗号分隔或JSON数组格式暂不支持复杂解析，这里做简单处理)
        // 实际应用中建议引入 toml 或 serde_yaml 解析完整结构
        if let Some(tech) = metadata_map.get("tech_stack") {
            let techs: Vec<String> = tech
                .split(',')
                .map(|s| s.trim().trim_matches(['[', ']', '"', '\'']).to_string())
                .collect();
            def.tags.insert("tech_stack".to_string(), techs.clone());
            def.metadata.tech_stack = techs;
        }

        if let Some(patterns) = metadata_map.get("file_patterns") {
            let pats: Vec<String> = patterns
                .split(',')
                .map(|s| s.trim().trim_matches(['[', ']', '"', '\'']).to_string())
                .collect();
            def.metadata.file_patterns = pats;
        }

        if let Some(types) = metadata_map.get("project_types") {
            let types: Vec<String> = types
                .split(',')
                .map(|s| s.trim().trim_matches(['[', ']', '"', '\'']).to_string())
                .collect();
            def.metadata.project_types = types;
        }

        Ok(def)
    }

    fn parse_frontmatter(
        &self,
        content: &str,
    ) -> (std::collections::HashMap<String, String>, String) {
        let mut metadata = std::collections::HashMap::new();
        let mut final_content = content.to_string();

        // 简单的 Frontmatter 解析 (支持 --- 或 +++)
        if content.starts_with("---") || content.starts_with("+++") {
            if let Some(end_idx) = content[3..].find(&content[0..3]) {
                let frontmatter = &content[3..end_idx + 3];
                final_content = content[end_idx + 6..].trim().to_string();

                for line in frontmatter.lines() {
                    if let Some((key, value)) = line.split_once([':', '=']) {
                        let key = key.trim().to_string();
                        let value = value.trim().trim_matches(['"', '\'']).to_string();
                        if !key.is_empty() {
                            metadata.insert(key, value);
                        }
                    }
                }
            }
        }

        (metadata, final_content)
    }
}

pub struct ContextLoader;

impl ContextLoader {
    pub fn load_from_file(path: &Path) -> Result<ContextDefinition, Box<dyn std::error::Error>> {
        // ... implementation similar to above ...
        let content = std::fs::read_to_string(path)?;
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let id = uuid::Uuid::new_v4().to_string();
        Ok(ContextDefinition::new(id, name, content))
    }
}

pub struct ContextValidator;

impl ContextValidator {
    pub fn validate(_context: &ContextDefinition) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(true)
    }
}

fn is_supported_context_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext, "md" | "json" | "toml"))
        .unwrap_or(false)
}

fn is_workspace_context_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = file_name.to_lowercase();
    !matches!(lower.as_str(), "index.json" | "learned_rules.md")
        && !lower.starts_with("learned_rules_archive_")
}

fn has_context_files(dir: &Path, workspace_rules: bool) -> bool {
    if !dir.exists() || !dir.is_dir() {
        return false;
    }

    WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|entry| entry.into_path())
        .any(|path| {
            path.is_file()
                && is_supported_context_extension(&path)
                && (!workspace_rules || is_workspace_context_file(&path))
        })
}
