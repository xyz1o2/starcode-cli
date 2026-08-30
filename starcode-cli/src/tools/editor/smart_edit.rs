// ============================================================================
// Smart Edit 工具
// ============================================================================
//
// 智能代码编辑工具，使用多策略自动回退机制
//
// 核心特性：
// 1. 策略链：精确 → 弹性 → 正则 → LLM（自动回退）
// 2. 遥测：记录策略使用情况
// 3. 可配置：通过环境变量控制策略启用/禁用

use super::strategies::{EditStrategy, StrategyFactory};
use super::{EditContext, EditResult};
use crate::core::tools::tools::{BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation};
use crate::core::tools::tools::{ToolError, ToolResult as CoreToolResult};
use crate::llm::client::StarClient;
use crate::types::ToolResult;
use serde::Deserialize;
use similar::TextDiff;
use std::sync::Arc;
use tokio::fs;

#[derive(Clone)]
pub struct SmartEditTool {
    strategies: Arc<Vec<Box<dyn EditStrategy>>>,
}

#[derive(Debug, Deserialize)]
pub struct SmartEditParams {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    pub dry_run: Option<bool>,
}

pub struct SmartEditToolInvocation {
    tool: SmartEditTool,
    params: SmartEditParams,
}

impl ToolInvocation for SmartEditToolInvocation {
    fn get_description(&self) -> String {
        format!("Smart edit: {}", self.params.file_path)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![ToolLocation {
            path: std::path::PathBuf::from(&self.params.file_path),
            location_type: crate::core::tools::tools::LocationType::Write,
        }]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        let path = self.params.file_path.clone();
        let old = self.params.old_string.clone();
        let new = self.params.new_string.clone();

        Box::pin(async move {
            let old_preview = old.lines().next().unwrap_or("").trim();
            let new_preview = new.lines().next().unwrap_or("").trim();
            Ok(Some(
                crate::core::tools::tools::ToolCallConfirmationDetails {
                    confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                    title: "Edit Confirmation".to_string(),
                    prompt: format!(
                        "Editing file: {}\n- old: {}\n- new: {}\n(full diff will be shown after execution)",
                        path, old_preview, new_preview
                    ),
                    on_confirm: std::sync::Arc::new(|_| {}),
                },
            ))
        })
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let tool = self.tool.clone();
        let path = self.params.file_path.clone();
        let old = self.params.old_string.clone();
        let new = self.params.new_string.clone();
        let dry_run = self.params.dry_run.unwrap_or(false);

        Box::pin(async move {
            let result = tool.edit(&path, &old, &new, dry_run).await?;
            Ok(CoreToolResult {
                llm_content: None,
                return_display: None,
                output: result.output.unwrap_or_default(),
                error: result.error.map(|msg| ToolError {
                    error_type: "execution_error".to_string(),
                    message: msg,
                }),
                data: result.data,
            })
        })
    }
}

impl BaseDeclarativeTool for SmartEditTool {
    fn name(&self) -> &str {
        "smart_edit"
    }

    fn display_name(&self) -> &str {
        "SmartEdit"
    }

    fn description(&self) -> &str {
        "Smartly edit a file using multiple strategies (Exact -> Flexible -> Regex -> LLM)"
    }

    fn kind(&self) -> Kind {
        Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "dry_run": { "type": "boolean", "description": "If true, only returns the diff without modifying the file" }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SmartEditParams = serde_json::from_value(params)?;
        Ok(Box::new(SmartEditToolInvocation {
            tool: self.clone(),
            params,
        }))
    }
}

impl SmartEditTool {
    /// 创建 Smart Edit 工具（包含 LLM 策略）
    pub fn new(llm_client: StarClient) -> Self {
        Self {
            strategies: Arc::new(StrategyFactory::create_all_strategies(Some(llm_client))),
        }
    }

    /// 创建 Smart Edit 工具（不包含 LLM 策略）
    pub fn new_basic() -> Self {
        Self {
            strategies: Arc::new(StrategyFactory::create_basic_strategies()),
        }
    }

    /// 执行智能编辑
    ///
    /// # Arguments
    /// * `file_path` - 文件路径
    /// * `old_string` - 要替换的旧字符串
    /// * `new_string` - 新字符串
    ///
    /// # Returns
    /// * `ToolResult` - 包含编辑结果和详细信息
    pub async fn edit(
        &self,
        file_path: &str,
        old_string: &str,
        new_string: &str,
        dry_run: bool,
    ) -> Result<ToolResult, Box<dyn std::error::Error>> {
        // 读取文件内容
        let content = crate::core::utils::file_utils::read_file_with_encoding_async(std::path::Path::new(file_path)).await?;

        // 构建编辑上下文
        let context = EditContext::new(
            file_path.to_string(),
            content.clone(),
            old_string.to_string(),
            new_string.to_string(),
        );

        // 依次尝试每个策略
        for strategy in self.strategies.iter() {
            crate::utils::logging::append_debug_log_line(&format!(
                "[SmartEdit] trying strategy: {} (priority: {})",
                strategy.name(),
                strategy.priority()
            ));

            match strategy.try_edit(&context).await {
                Ok(Some(result)) => {
                    if result.success {
                        if dry_run {
                            // Generate Diff
                            let diff = TextDiff::from_lines(&context.content, &result.new_content);
                            let diff_output =
                                format!("{}", diff.unified_diff().header(file_path, file_path));

                            return Ok(ToolResult {
                                success: true,
                                output: Some("(dry run)".to_string()),
                                error: None,
                                data: Some(serde_json::json!({
                                    "diff": diff_output,
                                    "strategy": result.strategy,
                                    "file_path": file_path
                                })),
                            });
                        }

                        // 策略成功，保存文件
                        fs::write(file_path, &result.new_content).await?;

                        return Ok(self.format_success_result(
                            &result,
                            file_path,
                            &context.content,
                        ));
                    } else {
                        // 策略返回了失败结果（例如 LLM 处理失败）
                        return Ok(self.format_failure_result(&result));
                    }
                }
                Ok(None) => {
                    // 此策略不适用，尝试下一个
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[SmartEdit] strategy {} not applicable, trying next",
                        strategy.name()
                    ));
                    continue;
                }
                Err(e) => {
                    // 策略执行出错，记录并尝试下一个
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[SmartEdit] strategy {} failed: {}",
                        strategy.name(),
                        e
                    ));
                    continue;
                }
            }
        }

        // 所有策略都失败
        Ok(ToolResult {
            success: false,
            output: None,
            error: Some("Edit failed: no strategy matched the target content".to_string()),
            data: None,
        })
    }

    /// 格式化成功结果
    fn format_success_result(
        &self,
        result: &EditResult,
        file_path: &str,
        original_content: &str,
    ) -> ToolResult {
        // Generate Diff
        let diff = TextDiff::from_lines(original_content, &result.new_content);
        let diff_output = format!("{}", diff.unified_diff().header(file_path, file_path));

        // output 只保留 diff 摘要，冗余状态信息（策略、替换次数等）不传给 UI
        let mut added = 0usize;
        let mut removed = 0usize;
        for line in diff_output.lines() {
            if line.starts_with('+') && !line.starts_with("+++") { added += 1; }
            else if line.starts_with('-') && !line.starts_with("---") { removed += 1; }
        }
        let brief = format!("+{} -{}", added, removed);

        ToolResult {
            success: true,
            output: Some(brief),
            error: None,
            data: Some(serde_json::json!({
                "strategy": result.strategy,
                "occurrences": result.occurrences,
                "file_path": file_path,
                "diff": diff_output
            })),
        }
    }

    /// 格式化失败结果
    fn format_failure_result(&self, result: &EditResult) -> ToolResult {
        ToolResult {
            success: false,
            output: None,
            error: Some(format!(
                "{}",
                result.details.as_deref().unwrap_or("edit failed")
            )),
            data: None,
        }
    }

    /// 获取启用的策略列表（用于调试）
    pub fn get_enabled_strategies(&self) -> Vec<String> {
        self.strategies
            .iter()
            .map(|s| s.name().to_string())
            .collect()
    }
}
 