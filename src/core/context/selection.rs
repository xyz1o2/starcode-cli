use std::collections::HashSet;
use std::path::Path;
use tokio::fs;

use super::types::{
    ContextDefinition, ContextMatch, ContextMatchScore, GitFeatures, MatchMetric, ProjectFeatures,
};

// ── Matching thresholds & weights ────────────────────────────────────────────
const SCORE_THRESHOLD: f64 = 0.3; // minimum score to keep a candidate

const W_TECH_STACK: f64 = 0.30;
const W_PROJECT_TYPE: f64 = 0.25;
const W_FILE_PATTERN: f64 = 0.20;
const W_DIRECTORY: f64 = 0.15;

const RECOMMEND_STRONG: f64 = 0.8;
const RECOMMEND_GOOD: f64 = 0.6;
const RECOMMEND_WEAK: f64 = 0.4;
const SIGNAL_LOW: f64 = 0.5; // metric score below this → emit improvement hint

pub struct ContextMatcher;

impl ContextMatcher {
    pub fn new() -> Self {
        Self
    }

    /// 查找最适合的上下文
    pub async fn find_best_contexts(
        &self,
        project_path: &Path,
        available_contexts: &[ContextDefinition],
    ) -> Result<Vec<ContextMatchScore>, Box<dyn std::error::Error>> {
        if available_contexts.is_empty() {
            return Ok(Vec::new());
        }

        // 步骤1: 提取项目特征
        let features = self.extract_project_features(project_path).await?;

        // 步骤2: 计算每个上下文的匹配分数
        let mut scores = Vec::new();
        for context in available_contexts {
            let score = self.calculate_match_score(&features, context).await?;
            if score.score > SCORE_THRESHOLD {
                // 阈值过滤
                scores.push(score);
            }
        }

        // 步骤3: 按分数排序
        scores.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 步骤4: 生成推荐建议
        for score in &mut scores {
            self.generate_recommendations(score)?;
        }

        Ok(scores)
    }

    /// 提取项目特征
    async fn extract_project_features(
        &self,
        project_path: &Path,
    ) -> Result<ProjectFeatures, Box<dyn std::error::Error>> {
        let mut features = ProjectFeatures {
            project_type: "unknown".to_string(),
            tech_stack: Vec::new(),
            file_patterns: Vec::new(),
            directory_structure: Vec::new(),
            git_history: None,
        };

        // 1. 分析项目类型
        features.project_type = self.detect_project_type(project_path).await?;

        // 2. 技术栈检测
        features.tech_stack = self.detect_tech_stack(project_path).await?;

        // 3. 文件模式扫描
        features.file_patterns = self.scan_file_patterns(project_path).await?;

        // 4. 目录结构分析
        features.directory_structure = self.analyze_directory_structure(project_path).await?;

        // 5. Git历史分析
        if project_path.join(".git").exists() {
            features.git_history = Some(self.analyze_git_history(project_path).await?);
        }

        Ok(features)
    }

    async fn detect_project_type(
        &self,
        project_path: &Path,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if project_path.join("Cargo.toml").exists() {
            return Ok("rust".to_string());
        }
        if project_path.join("package.json").exists() {
            // Further check for specific JS frameworks?
            return Ok("node".to_string());
        }
        if project_path.join("requirements.txt").exists()
            || project_path.join("pyproject.toml").exists()
        {
            return Ok("python".to_string());
        }
        if project_path.join("go.mod").exists() {
            return Ok("go".to_string());
        }
        if project_path.join("pom.xml").exists() || project_path.join("build.gradle").exists() {
            return Ok("java".to_string());
        }
        Ok("generic".to_string())
    }

    async fn detect_tech_stack(
        &self,
        project_path: &Path,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut stack = Vec::new();

        // Rust
        if project_path.join("Cargo.toml").exists() {
            stack.push("rust".to_string());
            // TODO: parse Cargo.toml for dependencies (tokio, serde, etc.)
            let content = fs::read_to_string(project_path.join("Cargo.toml"))
                .await
                .unwrap_or_default();
            if content.contains("tokio") {
                stack.push("tokio".to_string());
            }
            if content.contains("actix") {
                stack.push("actix".to_string());
            }
            if content.contains("axum") {
                stack.push("axum".to_string());
            }
            if content.contains("yew") {
                stack.push("yew".to_string());
            }
            if content.contains("leptos") {
                stack.push("leptos".to_string());
            }
            if content.contains("ratatui") {
                stack.push("ratatui".to_string());
            }
        }

        // Node
        if project_path.join("package.json").exists() {
            stack.push("node".to_string());
            let content = fs::read_to_string(project_path.join("package.json"))
                .await
                .unwrap_or_default();
            if content.contains("react") {
                stack.push("react".to_string());
            }
            if content.contains("vue") {
                stack.push("vue".to_string());
            }
            if content.contains("next") {
                stack.push("nextjs".to_string());
            }
            if content.contains("typescript") {
                stack.push("typescript".to_string());
            }
            if content.contains("tailwindcss") {
                stack.push("tailwindcss".to_string());
            }
        }

        // Python
        if project_path.join("requirements.txt").exists() {
            stack.push("python".to_string());
            let content = fs::read_to_string(project_path.join("requirements.txt"))
                .await
                .unwrap_or_default();
            if content.contains("django") {
                stack.push("django".to_string());
            }
            if content.contains("flask") {
                stack.push("flask".to_string());
            }
            if content.contains("fastapi") {
                stack.push("fastapi".to_string());
            }
            if content.contains("pandas") {
                stack.push("pandas".to_string());
            }
            if content.contains("numpy") {
                stack.push("numpy".to_string());
            }
        }

        Ok(stack)
    }

    async fn scan_file_patterns(
        &self,
        project_path: &Path,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut patterns = HashSet::new();
        // 三层 ignore（`~/.star/ignore` → `.starignore` → `.gitignore`）已经
        // 收进 `utils::file_walk`，这里只声明深度。
        let opts = crate::utils::file_walk::WalkOptions::new().max_depth(3);

        for result in crate::utils::file_walk::walk(project_path, &opts) {
            let Ok(entry) = result else { continue };
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                if let Some(dot) = name.rfind('.') {
                    let ext = &name[dot + 1..];
                    if !ext.is_empty() {
                        patterns.insert(format!("*.{}", ext));
                    }
                }
            }
        }
        Ok(patterns.into_iter().collect())
    }

    async fn analyze_directory_structure(
        &self,
        project_path: &Path,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut dirs = Vec::new();
        if let Ok(mut entries) = fs::read_dir(project_path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(ft) = entry.file_type().await {
                    if ft.is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            if !name.starts_with('.') && name != "target" && name != "node_modules"
                            {
                                dirs.push(name.to_string());
                            }
                        }
                    }
                }
            }
        }
        Ok(dirs)
    }

    async fn analyze_git_history(
        &self,
        _project_path: &Path,
    ) -> Result<GitFeatures, Box<dyn std::error::Error>> {
        // Placeholder
        Ok(GitFeatures {
            main_branch: "main".to_string(),
            recent_commits: vec![],
            frequent_files: vec![],
            languages: vec![],
        })
    }

    /// 计算匹配分数
    async fn calculate_match_score(
        &self,
        features: &ProjectFeatures,
        context: &ContextDefinition,
    ) -> Result<ContextMatchScore, Box<dyn std::error::Error>> {
        let mut score = 0.0;
        let mut matches = Vec::new();

        // 1. 技术栈匹配 (30%权重)
        let tech_score = self.calculate_tech_stack_score(
            &features.tech_stack,
            context.tags.get("tech_stack").unwrap_or(&Vec::new()),
        );
        score += tech_score * W_TECH_STACK;
        matches.push(ContextMatch {
            metric: MatchMetric::TechStackOverlap(tech_score),
            score: tech_score * W_TECH_STACK,
            details: format!("Tech stack overlap: {:.2}%", tech_score * 100.0),
            weight: W_TECH_STACK,
        });

        // 2. 项目类型匹配
        let type_score = self
            .calculate_project_type_score(&features.project_type, &context.metadata.project_types);
        score += type_score * W_PROJECT_TYPE;
        matches.push(ContextMatch {
            metric: MatchMetric::ProjectTypeMatch(type_score),
            score: type_score * W_PROJECT_TYPE,
            details: format!("Project type match: {:.2}%", type_score * 100.0),
            weight: W_PROJECT_TYPE,
        });

        // 3. 文件模式匹配
        let file_score = self
            .calculate_file_pattern_score(&features.file_patterns, &context.metadata.file_patterns);
        score += file_score * W_FILE_PATTERN;
        matches.push(ContextMatch {
            metric: MatchMetric::FilePatternMatch(file_score),
            score: file_score * W_FILE_PATTERN,
            details: format!("File pattern match: {:.2}%", file_score * 100.0),
            weight: W_FILE_PATTERN,
        });

        // 4. 目录结构匹配
        let dir_score = self.calculate_directory_score(
            &features.directory_structure,
            &context.metadata.directory_patterns,
        );
        score += dir_score * W_DIRECTORY;
        matches.push(ContextMatch {
            metric: MatchMetric::DirectoryStructureMatch(dir_score),
            score: dir_score * W_DIRECTORY,
            details: format!("Directory structure match: {:.2}%", dir_score * 100.0),
            weight: W_DIRECTORY,
        });

        Ok(ContextMatchScore {
            context_id: context.id.clone(),
            context_name: context.name.clone(),
            score,
            matches,
            project_features: features.clone(),
            recommendations: Vec::new(),
        })
    }

    /// 计算技术栈分数
    fn calculate_tech_stack_score(
        &self,
        project_stack: &[String],
        context_stack: &[String],
    ) -> f64 {
        if context_stack.is_empty() {
            return 1.0; // 无限制，默认匹配
        }

        let project_set: HashSet<&String> = project_stack.iter().collect();
        let context_set: HashSet<&String> = context_stack.iter().collect();

        let overlap: HashSet<_> = project_set.intersection(&context_set).collect();

        if context_stack.is_empty() {
            0.0
        } else {
            overlap.len() as f64 / context_stack.len() as f64
        }
    }

    /// 计算项目类型分数
    fn calculate_project_type_score(&self, project_type: &str, context_types: &[String]) -> f64 {
        if context_types.is_empty() {
            return 1.0;
        }

        if context_types
            .iter()
            .any(|t| t.to_lowercase() == project_type.to_lowercase())
        {
            1.0
        } else {
            0.0
        }
    }

    /// 计算文件模式分数
    fn calculate_file_pattern_score(
        &self,
        project_patterns: &[String],
        context_patterns: &[String],
    ) -> f64 {
        if context_patterns.is_empty() {
            return 1.0;
        }

        let mut matched = 0;
        for ctx_pattern in context_patterns {
            if let Ok(glob) = glob::Pattern::new(ctx_pattern) {
                if project_patterns.iter().any(|p| glob.matches(p)) {
                    matched += 1;
                }
            }
        }

        matched as f64 / context_patterns.len() as f64
    }

    /// 计算目录结构分数
    fn calculate_directory_score(&self, project_dirs: &[String], context_dirs: &[String]) -> f64 {
        if context_dirs.is_empty() {
            return 1.0;
        }

        let mut matched = 0;
        for ctx_dir in context_dirs {
            if project_dirs.iter().any(|d| d.contains(ctx_dir)) {
                matched += 1;
            }
        }

        matched as f64 / context_dirs.len() as f64
    }

    /// 生成推荐建议
    fn generate_recommendations(
        &self,
        score: &mut ContextMatchScore,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if score.score > RECOMMEND_STRONG {
            score
                .recommendations
                .push("✅ Strongly recommended context".to_string());
        } else if score.score > RECOMMEND_GOOD {
            score
                .recommendations
                .push("⚡ Recommended context".to_string());
        } else if score.score > RECOMMEND_WEAK {
            score
                .recommendations
                .push("ℹ️ Consider using this context".to_string());
        } else {
            score
                .recommendations
                .push("❌ Low match, consider other context".to_string());
        }

        // 添加具体建议
        for (idx, m) in score.matches.iter().enumerate() {
            if m.score < SIGNAL_LOW {
                score.recommendations.push(format!(
                    "{}. {}匹配度较低（{:.0}%）考虑优化相关配置",
                    idx + 1,
                    m.metric.get_name(),
                    m.score / m.weight * 100.0
                ));
            }
        }

        Ok(())
    }
}
