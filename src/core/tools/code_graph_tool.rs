//! CodeGraph 工具：复合代码调查（explore）。
//!
//! 给一个符号名，一次返回：命中符号的**逐字源码** + 直接 callers/callees +
//! 上下游影响面（深度可调）。对标 codegraph 的 `explore`，把原来
//! `CodeGraph` → `Read` → 再查调用链的三次往返压成一次。底层仍是
//! `.star/context/callgraph.db` 的持久化调用图。
//!
//! 对标设计文档 L1 结构层：精确结构问题走这里，概念/关键词走 CodebaseSearch，
//! 精确字符串走 Grep。只读、不改盘。

use crate::core::tools::codebase_search::{explore_symbol, ExploreLimits};
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
    /// 调用链深度（默认 2，上限 4）。
    #[serde(default)]
    pub depth: Option<usize>,
    /// 最多展开多少个匹配符号（默认 3，上限 5）。
    #[serde(default)]
    pub max_symbols: Option<usize>,
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
        format!("Explore: {}", self.params.symbol)
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
        let limits = ExploreLimits {
            max_symbols: self.params.max_symbols.unwrap_or(3).clamp(1, 5),
            depth: self.params.depth.unwrap_or(2).clamp(1, 4),
            max_source_lines: ExploreLimits::default().max_source_lines,
        };
        Box::pin(async move {
            let root = path
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            // 构图会走全仓解析，放到 blocking 线程，避免卡住 async 执行器。
            let joined =
                tokio::task::spawn_blocking(move || explore_symbol(&root, &symbol, limits)).await;
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
        "Compound code investigation: given a function/method/type name, returns its verbatim source, direct callers and callees, and the upstream/downstream impact in one call. Use for 'what calls X', 'what does X call', and 'what breaks if I change X' instead of chaining Grep + Read + call-graph queries."
    }

    fn kind(&self) -> Kind {
        Kind::Search
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "Function/method/type name to explore. Ranked: exact > camel/snake word boundary > prefix > substring > typo tolerance (1-2 edits for queries of 4+ chars)" },
                "path": { "type": "string", "description": "Root directory to scan (default: current directory)" },
                "depth": { "type": "integer", "minimum": 1, "maximum": 4, "description": "Call-chain depth for the impact summary (default: 2)" },
                "max_symbols": { "type": "integer", "minimum": 1, "maximum": 5, "description": "How many matching symbols to expand with source and call flow (default: 3)" }
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
