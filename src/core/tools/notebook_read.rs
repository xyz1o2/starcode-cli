use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookReadParams {
    pub file_path: String,
    pub include_outputs: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NotebookCell {
    cell_type: String,
    execution_count: Option<i32>,
    metadata: Value,
    outputs: Option<Vec<Value>>,
    source: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Notebook {
    cells: Vec<NotebookCell>,
    metadata: Value,
    nbformat: i32,
    nbformat_minor: i32,
}

pub struct NotebookReadInvocation {
    config: Arc<crate::core::config::Config>,
    params: NotebookReadParams,
    global_state: Arc<crate::core::state::GlobalState>,
}

impl NotebookReadInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: NotebookReadParams,
        global_state: Arc<crate::core::state::GlobalState>,
    ) -> Self {
        Self {
            config,
            params,
            global_state,
        }
    }
}

impl ToolInvocation for NotebookReadInvocation {
    fn get_description(&self) -> String {
        format!("Read notebook {}", self.params.file_path)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        let path = self.config.target_dir().join(&self.params.file_path);
        vec![ToolLocation {
            path,
            location_type: crate::core::tools::tools::LocationType::Read,
        }]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let params = self.params.clone();
        let global_state = self.global_state.clone();
        // Clone self to call format_notebook method, or move logic to associated function.
        // To avoid lifetime issues with 'self' in async block, we'll just copy the format logic or move it.
        // Actually, we can't easily call self.format_notebook inside the async block if we move self.params.
        // Let's make format_notebook a standalone function or method on params/struct that doesn't capture self.

        let include_outputs = params.include_outputs;
        let file_path_str = params.file_path.clone();

        Box::pin(async move {
            let path = config.target_dir().join(&file_path_str);

            if !path.exists() {
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "file_not_found".to_string(),
                        message: format!("File not found: {}", file_path_str),
                    }),
                    data: None,
                });
            }

            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| e.to_string())?;

            // 解析失败必须走和上面"文件不存在"一样的 `ToolResult.error`，不能裸
            // `Err` 出去。
            //
            // 裸 `Err` 在执行器里和"工具内部错误"是两条不同的路：它拿不到这里的
            // 文件名和解析器给出的列号，模型看到的是一句没有上下文的字符串，
            // 只能反复重试同一个坏文件。而"文件损坏"恰恰是模型**可以据此行动**的
            // 情况（换个文件 / 告诉用户这个 notebook 已损坏）。
            let notebook: Notebook = match serde_json::from_str(&content) {
                Ok(notebook) => notebook,
                Err(error) => {
                    let message = format!(
                        "Failed to parse notebook JSON in '{}': {}",
                        path.display(),
                        error
                    );
                    return Ok(ToolResult {
                        llm_content: Some(message.clone()),
                        return_display: Some(message.clone()),
                        output: message.clone(),
                        error: Some(ToolError {
                            error_type: "invalid_notebook_json".to_string(),
                            message,
                        }),
                        data: None,
                    });
                }
            };

            let mut output = String::new();
            output.push_str(&format!("Notebook: {}\n", file_path_str));
            output.push_str(&format!(
                "Format: v{}.{}\n",
                notebook.nbformat, notebook.nbformat_minor
            ));
            output.push_str(&format!("Total Cells: {}\n", notebook.cells.len()));
            output.push_str("--------------------------------------------------\n");

            for (i, cell) in notebook.cells.iter().enumerate() {
                output.push_str(&format!("Cell {} [{}]:\n", i, cell.cell_type));

                let source = cell.source.join("");
                output.push_str("```\n");
                output.push_str(&source);
                if !source.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("```\n");

                if include_outputs.unwrap_or(true) {
                    if let Some(outputs) = &cell.outputs {
                        if !outputs.is_empty() {
                            output.push_str("Outputs:\n");
                            for out in outputs {
                                // Simple output extraction
                                if let Some(text) = out.get("text") {
                                    if let Some(lines) = text.as_array() {
                                        for line in lines {
                                            if let Some(s) = line.as_str() {
                                                output.push_str(s);
                                            }
                                        }
                                    } else if let Some(s) = text.as_str() {
                                        output.push_str(s);
                                    }
                                } else if let Some(data) = out.get("data") {
                                    if let Some(text_plain) = data.get("text/plain") {
                                        if let Some(lines) = text_plain.as_array() {
                                            for line in lines {
                                                if let Some(s) = line.as_str() {
                                                    output.push_str(s);
                                                }
                                            }
                                        } else if let Some(s) = text_plain.as_str() {
                                            output.push_str(s);
                                        }
                                    }
                                } else if let Some(output_type) = out.get("output_type") {
                                    if output_type == "error" {
                                        if let Some(ename) =
                                            out.get("ename").and_then(|v| v.as_str())
                                        {
                                            output.push_str(&format!("Error: {}\n", ename));
                                        }
                                        if let Some(evalue) =
                                            out.get("evalue").and_then(|v| v.as_str())
                                        {
                                            output.push_str(&format!("Value: {}\n", evalue));
                                        }
                                    }
                                }
                                output.push('\n');
                            }
                        }
                    }
                }
                output.push_str("--------------------------------------------------\n");
            }

            // 登记 read state，否则紧随其后的 `notebook_edit` 必然被
            // `[edit_file_not_read]` 拦下 —— 而"先 notebook_read 再 notebook_edit"
            // 正是 `notebook_edit` 的工具说明要求模型走的路径。
            //
            // 登记的是**原始 JSON 文本**而不是上面拼给人看的转储：`Edit`/`Write`
            // 之后会用存下来的 content 和磁盘内容比对来判断文件是否被改过，
            // 塞一份转储进去会让每一次比对都判定"内容变了"。
            {
                let abs_path = crate::core::utils::paths::read_state_key(&path);
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let file_system_timestamp = tokio::fs::metadata(&path)
                    .await
                    .and_then(|m| m.modified())
                    .map(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                    })
                    .unwrap_or(timestamp);
                crate::core::tools::read_file::record_file_read(
                    &global_state,
                    &abs_path,
                    content,
                    timestamp,
                    file_system_timestamp,
                )
                .await;
            }

            Ok(ToolResult {
                llm_content: Some(output.clone()),
                return_display: Some(output.clone()),
                output,
                error: None,
                data: None,
            })
        })
    }
}

pub struct NotebookReadTool {
    config: Arc<crate::core::config::Config>,
    global_state: Arc<crate::core::state::GlobalState>,
}

impl NotebookReadTool {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        global_state: Arc<crate::core::state::GlobalState>,
    ) -> Self {
        Self {
            config,
            global_state,
        }
    }
}

impl BaseDeclarativeTool for NotebookReadTool {
    fn name(&self) -> &str {
        "notebook_read"
    }

    fn display_name(&self) -> &str {
        "Notebook Read"
    }

    fn description(&self) -> &str {
        "Reads a Jupyter Notebook (.ipynb) file and returns a formatted text representation of its cells and outputs. Useful for understanding code and results in notebooks."
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the .ipynb file"
                },
                "include_outputs": {
                    "type": "boolean",
                    "description": "Whether to include cell outputs in the response (default: true)"
                }
            },
            "required": ["file_path"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: NotebookReadParams = serde_json::from_value(params)?;
        Ok(Box::new(NotebookReadInvocation::new(
            self.config.clone(),
            params,
            self.global_state.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 一个最小但结构完整的 notebook（nbformat 4 需要 cells/metadata/nbformat/nbformat_minor）。
    fn notebook_json() -> String {
        serde_json::json!({
            "cells": [
                { "cell_type": "code", "execution_count": 1, "metadata": {},
                  "outputs": [], "source": ["print('one')"] },
                { "cell_type": "markdown", "metadata": {},
                  "source": ["# heading"] }
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        })
        .to_string()
    }

    async fn harness(
        tag: &str,
    ) -> (
        tempfile::TempDir,
        Arc<crate::core::config::Config>,
        Arc<crate::core::state::GlobalState>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut params = crate::core::config::ConfigParameters::default();
        params.model = "test-model".to_string();
        params.core_tools = Some(Vec::new());
        params.mcp_enabled = Some(false);
        params.enable_agents = Some(false);
        // 工具用 `config.target_dir()` 解析相对路径，不是 `cwd`。
        params.target_dir = dir.path().to_path_buf();
        let mut config = crate::core::config::Config::new(params);
        config
            .initialize()
            .await
            .expect("test config must initialize");
        (
            dir,
            Arc::new(config),
            Arc::new(crate::core::state::GlobalState::new()),
        )
    }

    async fn run(
        config: &Arc<crate::core::config::Config>,
        global_state: &Arc<crate::core::state::GlobalState>,
        params: serde_json::Value,
    ) -> crate::core::tools::tools::ToolResult {
        NotebookReadTool::new(config.clone(), global_state.clone())
            .create_invocation(params)
            .expect("create_invocation should accept these params")
            .execute(None, None)
            .await
            .expect("execute should not return Err")
    }

    /// 完整读取：返回每个 cell 的内容，错误为空。
    #[tokio::test]
    async fn reads_every_cell_and_reports_no_error() {
        let (dir, config, gs) = harness("nb-read-ok").await;
        std::fs::write(dir.path().join("nb.ipynb"), notebook_json()).unwrap();

        let result = run(&config, &gs, json!({ "file_path": "nb.ipynb" })).await;

        assert!(result.error.is_none(), "不该报错：{:?}", result.error);
        let out = result.llm_content.expect("成功必须有 llm_content");
        assert!(out.contains("Total Cells: 2"), "应当数出 2 个 cell：{out}");
        assert!(out.contains("print('one')"), "代码内容必须在：{out}");
        assert!(out.contains("# heading"), "markdown 内容必须在：{out}");
        assert!(out.contains("Format: v4.5"), "nbformat 要报出来：{out}");
    }

    /// 回归：读完之后必须登记 read state，否则紧随其后的 `notebook_edit`
    /// 必然被 `[edit_file_not_read]` 拦下 —— 而"先 notebook_read 再
    /// notebook_edit"正是 `notebook_edit` 的工具说明要求模型走的路径。
    ///
    /// 修复前 `NotebookReadTool` 压根没有 `GlobalState`，这条路径 100% 走不通。
    #[tokio::test]
    async fn a_read_notebook_is_registered_in_the_read_state() {
        let (dir, config, gs) = harness("nb-read-records").await;
        let nb = dir.path().join("nb.ipynb");
        std::fs::write(&nb, notebook_json()).unwrap();

        run(&config, &gs, json!({ "file_path": "nb.ipynb" })).await;

        let key = crate::core::utils::paths::read_state_key(&nb);
        let state = gs.read_file_state.read().await;
        let recorded = state
            .get(&key)
            .unwrap_or_else(|| panic!("notebook 读过之后必须在 read_file_state 里，键：{key}"));
        assert_eq!(
            recorded.content,
            notebook_json(),
            "登记的必须是原始 JSON 全文 —— Edit/Write 之后靠它和磁盘比对，\
             塞一份转储进去会让每次比对都判'内容变了'"
        );
    }

    /// 文件不存在要报错，不能返回空成功。
    #[tokio::test]
    async fn a_missing_notebook_errors_instead_of_returning_empty() {
        let (_dir, config, gs) = harness("nb-read-missing").await;

        let result = run(&config, &gs, json!({ "file_path": "nope.ipynb" })).await;

        assert!(result.error.is_some(), "文件不存在必须报错");
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .message
                .contains("File not found"),
            "错误要说清是找不到文件：{:?}",
            result.error
        );
    }

    /// 损坏的 JSON 要报错（`Failed to parse notebook JSON`），不能 panic 也不能空成功。
    #[tokio::test]
    async fn a_corrupt_notebook_reports_a_parse_error() {
        let (dir, config, gs) = harness("nb-read-corrupt").await;
        std::fs::write(dir.path().join("bad.ipynb"), "{ not json").unwrap();

        let result = run(&config, &gs, json!({ "file_path": "bad.ipynb" })).await;

        let error = result
            .error
            .expect("坏 JSON 必须以错误形式失败，不能 panic 或空成功");
        assert!(
            error.message.contains("Failed to parse notebook JSON"),
            "错误要说清是解析失败：{error:?}"
        );
    }

    /// 缺 required 参数在 `create_invocation` 就被拒。
    #[tokio::test]
    async fn missing_required_params_are_rejected_at_creation() {
        let (_dir, config, gs) = harness("nb-read-badparams").await;
        assert!(
            NotebookReadTool::new(config, gs)
                .create_invocation(json!({}))
                .is_err(),
            "缺 file_path 必须在建 invocation 时报错"
        );
    }
}
