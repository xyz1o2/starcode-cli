/// LSP诊断注册表

use serde::{Deserialize, Serialize};

/// 诊断严重性
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DiagnosticSeverity {
    /// 错误
    Error,
    /// 警告
    Warning,
    /// 信息
    Information,
    /// 提示
    Hint,
}

/// 诊断
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// 文件路径
    pub file: String,
    /// 行号
    pub line: u32,
    /// 列号
    pub column: u32,
    /// 结束行号
    pub end_line: Option<u32>,
    /// 结束列号
    pub end_column: Option<u32>,
    /// 严重性
    pub severity: DiagnosticSeverity,
    /// 消息
    pub message: String,
    /// 来源
    pub source: Option<String>,
    /// 代码
    pub code: Option<String>,
    /// 相关信息
    pub related_information: Vec<RelatedDiagnosticInformation>,
}

/// 相关诊断信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedDiagnosticInformation {
    /// 文件路径
    pub file: String,
    /// 行号
    pub line: u32,
    /// 列号
    pub column: u32,
    /// 消息
    pub message: String,
}

/// 诊断注册表
pub struct DiagnosticRegistry {
    /// 诊断存储
    diagnostics: std::collections::HashMap<String, Vec<Diagnostic>>,
}

impl DiagnosticRegistry {
    /// 创建新的诊断注册表
    pub fn new() -> Self {
        Self {
            diagnostics: std::collections::HashMap::new(),
        }
    }

    /// 添加诊断
    pub fn add_diagnostic(&mut self, file: &str, diagnostic: Diagnostic) {
        self.diagnostics
            .entry(file.to_string())
            .or_insert_with(Vec::new)
            .push(diagnostic);
    }

    /// 获取文件的诊断
    pub fn get_diagnostics(&self, file: &str) -> Vec<&Diagnostic> {
        self.diagnostics
            .get(file)
            .map(|diags| diags.iter().collect())
            .unwrap_or_default()
    }

    /// 获取所有诊断
    pub fn get_all_diagnostics(&self) -> Vec<&Diagnostic> {
        self.diagnostics.values().flatten().collect()
    }

    /// 清除文件的诊断
    pub fn clear_file_diagnostics(&mut self, file: &str) {
        self.diagnostics.remove(file);
    }

    /// 清除所有诊断
    pub fn clear_all_diagnostics(&mut self) {
        self.diagnostics.clear();
    }

    /// 获取诊断统计
    pub fn get_statistics(&self) -> DiagnosticStatistics {
        let mut stats = DiagnosticStatistics::default();

        for diag in self.diagnostics.values().flatten() {
            match diag.severity {
                DiagnosticSeverity::Error => stats.errors += 1,
                DiagnosticSeverity::Warning => stats.warnings += 1,
                DiagnosticSeverity::Information => stats.information += 1,
                DiagnosticSeverity::Hint => stats.hints += 1,
            }
        }

        stats.total = stats.errors + stats.warnings + stats.information + stats.hints;
        stats
    }
}

/// 诊断统计
#[derive(Debug, Default)]
pub struct DiagnosticStatistics {
    pub total: u32,
    pub errors: u32,
    pub warnings: u32,
    pub information: u32,
    pub hints: u32,
}
