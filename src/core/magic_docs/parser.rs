/// 代码解析器

/// 代码解析器
pub struct CodeParser;

impl CodeParser {
    /// 创建新的代码解析器
    pub fn new() -> Self {
        Self
    }

    /// 解析代码文件
    pub fn parse_file(&self, file_path: &str) -> Result<CodeInfo, ParseError> {
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| ParseError::IoError(e.to_string()))?;

        let extension = std::path::Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        Ok(CodeInfo {
            file_path: file_path.to_string(),
            language: self.detect_language(extension),
            lines: content.lines().count(),
            functions: self.extract_functions(&content),
            classes: self.extract_classes(&content),
            imports: self.extract_imports(&content),
        })
    }

    /// 检测语言
    fn detect_language(&self, extension: &str) -> String {
        match extension {
            "rs" => "Rust",
            "ts" | "tsx" => "TypeScript",
            "js" | "jsx" => "JavaScript",
            "py" => "Python",
            "go" => "Go",
            "java" => "Java",
            "c" | "h" => "C",
            "cpp" | "cc" | "cxx" => "C++",
            _ => "Unknown",
        }.to_string()
    }

    /// 提取函数
    fn extract_functions(&self, content: &str) -> Vec<String> {
        let mut functions = Vec::new();
        
        for line in content.lines() {
            let trimmed = line.trim();
            
            // Rust函数
            if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") || trimmed.starts_with("async fn ") {
                if let Some(name) = trimmed.split('(').next() {
                    let name = name.split_whitespace().last().unwrap_or("");
                    if !name.is_empty() {
                        functions.push(name.to_string());
                    }
                }
            }
            
            // TypeScript/JavaScript函数
            if trimmed.starts_with("function ") || trimmed.contains("function ") {
                if let Some(name) = trimmed.split('(').next() {
                    let name = name.split_whitespace().last().unwrap_or("");
                    if !name.is_empty() {
                        functions.push(name.to_string());
                    }
                }
            }
        }

        functions
    }

    /// 提取类
    fn extract_classes(&self, content: &str) -> Vec<String> {
        let mut classes = Vec::new();
        
        for line in content.lines() {
            let trimmed = line.trim();
            
            if trimmed.starts_with("class ") || trimmed.starts_with("pub class ") {
                if let Some(name) = trimmed.split('{').next() {
                    let name = name.split_whitespace().nth(1).unwrap_or("");
                    if !name.is_empty() {
                        classes.push(name.to_string());
                    }
                }
            }
        }

        classes
    }

    /// 提取导入
    fn extract_imports(&self, content: &str) -> Vec<String> {
        let mut imports = Vec::new();
        
        for line in content.lines() {
            let trimmed = line.trim();
            
            if trimmed.starts_with("use ") || trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                imports.push(trimmed.to_string());
            }
        }

        imports
    }
}

/// 代码信息
#[derive(Debug)]
pub struct CodeInfo {
    pub file_path: String,
    pub language: String,
    pub lines: usize,
    pub functions: Vec<String>,
    pub classes: Vec<String>,
    pub imports: Vec<String>,
}

/// 解析错误
#[derive(Debug)]
pub enum ParseError {
    IoError(String),
    ParseError(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::IoError(e) => write!(f, "IO error: {}", e),
            ParseError::ParseError(e) => write!(f, "Parse error: {}", e),
        }
    }
}
