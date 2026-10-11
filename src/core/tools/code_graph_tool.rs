//! CodeGraph 工具：把已有的调用图追踪（`codebase_search::trace_call_chain`）
//! 提升为一等公民工具——给一个符号名，返回 who-calls / callees / 调用链。
//!
//! 对标设计文档 L1 结构层：精确结构问题走这里，概念/关键词走 CodebaseSearch，
//! 精确字符串走 Grep。只读、不改盘。

use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult as CoreToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodeGraphParams {
    /// 要在调用图里追踪的函数/方法/类型名。
    pub symbol: String,
    /// 搜索根目录（默认当前目录）。
    #[serde(default)]
    pub path: Option<String>,
}

pub struct CodeGraphTool;

impl CodeGraphTool {
    pub fn new() -> Self {
        CodeGraphTool
    }
}

impl Default for CodeGraphTool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CodeGraphInvocation {
    params: CodeGraphParams,
}

impl ToolInvocation for CodeGraphInvocation {
    fn get_description(&self) -> String {
        format!("Call graph: {}", self.params.symbol)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
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
        let symbol = self.params.symbol.clone();
        let path = self.params.path.clone();
        Box::pin(async move {
            let root = path
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            // 构图会走全仓解析，放到 blocking 线程，避免卡住 async 执行器。
            let joined = tokio::task::spawn_blocking(move || {
                crate::core::tools::codebase_search::trace_call_chain(&root, &symbol)
            })
            .await;
            let text = match joined {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => format!("CodeGraph error: {e}"),
                Err(e) => format!("CodeGraph task failed: {e}"),
            };
            Ok(CoreToolResult {
                llm_content: Some(text.clone()),
                return_display: Some(text.clone()),
                output: text,
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for CodeGraphTool {
    fn name(&self) -> &str {
        "CodeGraph"
    }

    fn display_name(&self) -> &str {
        "Code Graph"
    }

    fn description(&self) -> &str {
        "Precise call-graph query: given a function/method/type name, returns who-calls, callees, and call chains via tree-sitter + resolved edges. Use for 'what calls X' / 'how does X reach Y' structural questions; for exact strings use Grep, for concepts use CodebaseSearch."
    }

    fn kind(&self) -> Kind {
        Kind::Search
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "Function/method/type name to trace in the call graph" },
                "path": { "type": "string", "description": "Root directory to scan (default: current directory)" }
            },
            "required": ["symbol"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: CodeGraphParams = serde_json::from_value(params)?;
        Ok(Box::new(CodeGraphInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
