//! RepoMap 工具：给模型一份"这个仓库里最重要的符号"清单。
//!
//! 对标 aider 的 repo map：按调用图中心性排序的符号表（名字 + 签名 + 位置）。
//! 它解决的是"中文需求落不到英文标识符上"—— 模型看到 `retry_backoff` 这类
//! **真实符号名**，才知道该拿什么去喂词法引擎。
//!
//! 只读、不改盘。

use crate::core::context::repo_map::{build_repo_map, RepoMapLimits};
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult as CoreToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepoMapParams {
    /// 提示词（可选）：用户提到的词，用来给相关文件/符号加权。
    #[serde(default)]
    pub focus: Option<String>,
    /// 搜索根目录（默认当前目录）。
    #[serde(default)]
    pub path: Option<String>,
    /// 最多列多少个符号（默认 60，上限 200）。
    #[serde(default)]
    pub max_entries: Option<usize>,
}

pub struct RepoMapTool;

impl RepoMapTool {
    pub fn new() -> Self {
        RepoMapTool
    }
}

impl Default for RepoMapTool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RepoMapInvocation {
    params: RepoMapParams,
}

impl ToolInvocation for RepoMapInvocation {
    fn get_description(&self) -> String {
        match self
            .params
            .focus
            .as_deref()
            .filter(|focus| !focus.trim().is_empty())
        {
            Some(focus) => format!("Repo map · {focus}"),
            None => "Repo map".to_string(),
        }
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
        let focus = self.params.focus.clone();
        let path = self.params.path.clone();
        let max_entries = self.params.max_entries.unwrap_or(60).clamp(1, 200);

        Box::pin(async move {
            let root = path
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let limits = RepoMapLimits {
                max_entries,
                ..RepoMapLimits::default()
            };

            // 构图要读库/解析，放到 blocking 线程，避免卡住 async 执行器。
            let joined = tokio::task::spawn_blocking(move || {
                build_repo_map(&root, focus.as_deref(), limits)
            })
            .await;

            let text = match joined {
                Ok(Ok(text)) => text,
                Ok(Err(err)) => format!("RepoMap error: {err}"),
                Err(err) => format!("RepoMap task failed: {err}"),
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

impl BaseDeclarativeTool for RepoMapTool {
    fn name(&self) -> &str {
        "RepoMap"
    }

    fn display_name(&self) -> &str {
        "Repo Map"
    }

    fn description(&self) -> &str {
        "Repository symbol map: the most important functions/methods/types, ranked by call-graph centrality, with their signatures and locations. Use it to learn the repo's REAL identifier names before searching — especially when the request is in a language the code isn't written in."
    }

    fn kind(&self) -> Kind {
        Kind::Search
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "focus": { "type": "string", "description": "Optional hint (word from the request) to bias the map towards related files/symbols" },
                "path": { "type": "string", "description": "Root directory to scan (default: current directory)" },
                "max_entries": { "type": "integer", "minimum": 1, "maximum": 200, "description": "How many symbols to list (default: 60)" }
            },
            "required": []
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: RepoMapParams = serde_json::from_value(params)?;
        Ok(Box::new(RepoMapInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
