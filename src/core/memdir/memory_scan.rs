/// 记忆扫描器

use super::MemoryEntry;

/// 记忆扫描器
pub struct MemoryScanner;

impl MemoryScanner {
    /// 创建新的记忆扫描器
    pub fn new() -> Self {
        Self
    }

    /// 扫描记忆内容
    pub fn scan_content(&self, content: &str) -> ScanResult {
        let word_count = content.split_whitespace().count();
        let has_code = content.contains("```") || content.contains("fn ") || content.contains("function ");
        let has_error = content.to_lowercase().contains("error") || content.to_lowercase().contains("failed");
        
        ScanResult {
            word_count,
            has_code,
            has_error,
            keywords: self.extract_keywords(content),
        }
    }

    /// 提取关键词
    fn extract_keywords(&self, content: &str) -> Vec<String> {
        let content_lower = content.to_lowercase();
        let mut keywords = Vec::new();
        
        let important_words = ["error", "fix", "bug", "test", "refactor", "optimize", "feature"];
        for word in &important_words {
            if content_lower.contains(word) {
                keywords.push(word.to_string());
            }
        }
        
        keywords
    }
}

/// 扫描结果
#[derive(Debug)]
pub struct ScanResult {
    pub word_count: usize,
    pub has_code: bool,
    pub has_error: bool,
    pub keywords: Vec<String>,
}
