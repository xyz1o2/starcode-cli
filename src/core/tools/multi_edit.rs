use crate::core::state::GlobalState;
use crate::core::tools::constants::ToolErrorType;
use crate::core::tools::edit::{
    apply_replacement, calculate_replacement, diagnose_replace_failure,
};
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiEditToolParams {
    pub edits: Vec<SingleFileEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleFileEdit {
    #[serde(rename = "file_path")]
    pub file_path: String,
    #[serde(rename = "old_string")]
    pub old_string: String,
    #[serde(rename = "new_string")]
    pub new_string: String,
}

pub struct MultiEditToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: MultiEditToolParams,
    global_state: Arc<GlobalState>,
}

impl MultiEditToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: MultiEditToolParams,
        _tool_name: Option<String>,
        _tool_display_name: Option<String>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            params,
            global_state,
        }
    }
}

impl ToolInvocation for MultiEditToolInvocation {
    fn get_description(&self) -> String {
        format!("Multi-file edit with {} changes", self.params.edits.len())
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        self.params
            .edits
            .iter()
            .map(|edit| {
                let path = self.config.target_dir().join(&edit.file_path);
                ToolLocation {
                    path,
                    location_type: crate::core::tools::tools::LocationType::Write,
                }
            })
            .collect()
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

        Box::pin(async move {
            let mut file_contents = HashMap::new();
            let final_contents;

            // ============ P1.2 改进：前置验证 - 检查所有编辑文件是否已被读取 ============
            let strict_read_check = std::env::var("STAR_DISABLE_READ_CHECK")
                .map(|v| v.to_lowercase() != "true" && v != "1")
                .unwrap_or(true);

            if strict_read_check {
                {
                    let read_state = global_state.read_file_state.read().await;
                    // 按**文件**去重，不是按编辑条目。
                    //
                    // 一次 multi_edit 常常对同一个文件做十几处替换（8 处很常见）。
                    // 按条目统计的话，未读的文件会被列出 8 遍、报错说"8 个文件"，
                    // 下一��还让模型连着调 8 次一模一样的 `Read` —— 看起来像 8 个
                    // 文件，其实只有 1 个。
                    let mut unread: Vec<&str> = Vec::new();
                    let mut seen_unread: HashSet<String> = HashSet::new();
                    for edit in &params.edits {
                        let path = config.target_dir().join(&edit.file_path);
                        // 键必须和写入端（`read_file::record_file_read`、`Glob` 的
                        // 浏览登记）用同一个函数算。这里原来自己 canonicalize 一次、
                        // 失败就退回未规范化的原路径，于是 `src/../lib.rs` 这类路径
                        // 在 Read 侧被折叠成 `lib.rs`、在这里却保持原样，查询必然落空。
                        //
                        // 更要紧的是查错了表：Glob / Grep / read_many / 批量 Read
                        // 都只写 `read_file_state`，只有单个 `Read` 才顺带写
                        // `execution_state`。查 `execution_state` 的话，
                        // "Grep 完再 multi_edit" 是 100% 误报。
                        let abs_path = crate::core::utils::paths::read_state_key(&path);

                        if path.exists()
                            && !read_state.contains_key(&abs_path)
                            && seen_unread.insert(abs_path.clone())
                        {
                            unread.push(&edit.file_path);
                        }
                    }
                    if !unread.is_empty() {
                        let file_list = unread
                            .iter()
                            .map(|p| format!("  - {}", p))
                            .collect::<Vec<_>>()
                            .join("\n");
                        let read_calls = unread
                            .iter()
                            .map(|p| format!("`Read('{}')`", p))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let msg = format!(
                            "Edit blocked [edit_file_not_read]: {count} file(s) must be read before `multi_edit`:\n{list}\n\
                             REQUIRED NEXT STEP: call {calls} (batch them in one response), then retry `multi_edit`. \
                             Do NOT retry until ALL listed files have been read.",
                            count = unread.len(),
                            list = file_list,
                            calls = read_calls,
                        );
                        return Ok(ToolResult {
                            llm_content: Some(msg.clone()),
                            return_display: Some(msg.clone()),
                            output: msg.clone(),
                            error: Some(ToolError {
                                error_type: ToolErrorType::EditFileNotRead.to_string(),
                                message: msg,
                            }),
                            data: None,
                        });
                    }
                } // 显式释放读锁
            }

            // 1. Read and Validate Phase
            for edit in &params.edits {
                let path = config.target_dir().join(&edit.file_path);

                // Read file if not already read
                if !file_contents.contains_key(&path) {
                    if path.exists() {
                        let content =
                            crate::core::utils::file_utils::read_file_with_encoding_async(&path)
                                .await
                                .map_err(|e| {
                                    format!("Failed to read file {}: {}", edit.file_path, e)
                                })?;
                        file_contents.insert(path.clone(), content);
                    } else {
                        // For new files, content is empty
                        file_contents.insert(path.clone(), String::new());
                    }
                }

                let current_content = file_contents.get(&path).unwrap();
                let is_new_file = current_content.is_empty() && !path.exists();

                // 预校验走和 Edit 相同的匹配级联（exact → 缩进/空白归一化 → 易混字符 → 首尾锚定 → 正则），
                // 旧实现只做 contains() 精确匹配，Edit 能改的编辑在 MultiEdit 里会被直接拒掉
                if !is_new_file
                    && calculate_replacement(current_content, &edit.old_string, &edit.new_string)
                        .occurrences
                        == 0
                {
                    let diagnosis =
                        diagnose_replace_failure(&path.to_string_lossy(), &edit.old_string);
                    return Ok(ToolResult {
                        llm_content: None,
                        return_display: None,
                        output: format!(
                            "Verification failed: could not find `old_string` in {}.",
                            edit.file_path
                        ),
                        error: Some(ToolError {
                            error_type: "edit_verification_failed".to_string(),
                            message: format!(
                                "Could not find `old_string` in {}. \
                                 All matching strategies were tried and failed.\n\n{}",
                                edit.file_path, diagnosis
                            ),
                        }),
                        data: None,
                    });
                }
            }

            // Re-do validation with sequential application
            let mut working_contents = file_contents.clone();

            for edit in &params.edits {
                let path = config.target_dir().join(&edit.file_path);
                // Important: Use working_contents here to pick up changes from previous edits in the same sequence!
                // Wait, if we edit file1 twice, the second edit should run against the output of the first edit.
                let current_content = working_contents.get(&path).unwrap();
                let is_new_file = current_content.is_empty() && !path.exists(); // Approximate check

                // Note: We remove debug printlns for production
                let new_content = if is_new_file {
                    apply_replacement(
                        Some(current_content),
                        &edit.old_string,
                        &edit.new_string,
                        true,
                    )
                } else {
                    let result =
                        calculate_replacement(current_content, &edit.old_string, &edit.new_string);
                    if result.occurrences == 0 {
                        return Ok(ToolResult {
                            llm_content: None,
                            return_display: None,
                            output: format!(
                                "Verification failed during sequence: `old_string` not found in {}.",
                                edit.file_path
                            ),
                            error: Some(ToolError {
                                error_type: "edit_verification_failed".to_string(),
                                message: format!(
                                    "Could not find `old_string` in {} while applying edits in order. \
                                     An earlier edit in this batch probably already rewrote that region — \
                                     check for overlapping edits.",
                                    edit.file_path
                                ),
                            }),
                            data: None,
                        });
                    }
                    result.new_content
                };
                working_contents.insert(path, new_content);
            }

            final_contents = working_contents;

            // 2. Write Phase (Atomic-ish)
            // Ideally we would use a transaction log or backup, but for now we rely on the pre-validation.
            // If validation passes, writes are likely to succeed unless FS issues occur.
            let mut modified_files = Vec::new();
            let mut backup_map: HashMap<PathBuf, String> = HashMap::new();
            let mut write_error: Option<String> = None;

            // Backup phase
            for (path, _) in &final_contents {
                let is_target = params
                    .edits
                    .iter()
                    .any(|e| config.target_dir().join(&e.file_path) == *path);
                if is_target && path.exists() {
                    // File-history checkpoint: snapshot before overwrite.
                    // track_edit is best-effort — failures must not block.
                    {
                        let msg_id = global_state.current_message_id().await;
                        if let Err(e) = crate::utils::checkpoint_manager::track_edit(
                            path,
                            msg_id,
                            Some("multi_edit"),
                            None, // session_id: per-cwd fallback, matches /undo and /rewind
                        )
                        .await
                        {
                            log::warn!(
                                "FileHistory: track_edit failed for {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                    match crate::core::utils::file_utils::read_file_with_encoding_async(path).await
                    {
                        Ok(content) => {
                            backup_map.insert(path.clone(), content);
                        }
                        Err(e) => {
                            write_error = Some(format!(
                                "Failed to create backup for {}: {}",
                                path.display(),
                                e
                            ));
                            break;
                        }
                    }
                }
            }

            if let Some(err) = write_error {
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: format!("MultiEdit Aborted: {}", err),
                    error: Some(ToolError {
                        error_type: "backup_failed".to_string(),
                        message: err,
                    }),
                    data: None,
                });
            }

            // Write phase
            for (path, content) in final_contents {
                // Only write if it was targeted
                let is_target = params
                    .edits
                    .iter()
                    .any(|e| config.target_dir().join(&e.file_path) == path);
                if is_target {
                    if let Some(parent) = path.parent() {
                        if let Err(e) = tokio::fs::create_dir_all(parent).await {
                            write_error = Some(format!(
                                "Failed to create directory {}: {}",
                                parent.display(),
                                e
                            ));
                            break;
                        }
                    }
                    if let Err(e) = tokio::fs::write(&path, content).await {
                        write_error =
                            Some(format!("Failed to write file {}: {}", path.display(), e));
                        break;
                    }
                    modified_files.push(path);
                }
            }

            // Rollback phase if error
            if let Some(err) = write_error {
                // Attempt rollback
                for (path, content) in backup_map {
                    // Ignore rollback errors for now, best effort
                    let _ = tokio::fs::write(path, content).await;
                }

                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: format!("MultiEdit Failed: {}. Rolled back changes.", err),
                    error: Some(ToolError {
                        error_type: "write_failed_rollback".to_string(),
                        message: format!("Write failed: {}. Changes have been rolled back.", err),
                    }),
                    data: None,
                });
            }

            let file_names: Vec<String> = modified_files
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect();

            let msg = format!(
                "Successfully modified {} files: {}",
                file_names.len(),
                file_names.join(", ")
            );

            Ok(ToolResult {
                llm_content: Some(msg.clone()),
                return_display: Some(msg.clone()),
                output: msg,
                error: None,
                data: Some(json!({ "modified_files": file_names })),
            })
        })
    }
}

pub struct MultiEditTool {
    config: Arc<crate::core::config::Config>,
    global_state: Arc<GlobalState>,
}

impl MultiEditTool {
    pub fn new(config: Arc<crate::core::config::Config>, global_state: Arc<GlobalState>) -> Self {
        Self {
            config,
            global_state,
        }
    }
}

impl BaseDeclarativeTool for MultiEditTool {
    fn name(&self) -> &str {
        "multi_edit"
    }

    fn display_name(&self) -> &str {
        "MultiEdit"
    }

    fn description(&self) -> &str {
        "Edit multiple files in a single atomic transaction. Use this when you need to change multiple files that depend on each other (e.g. changing a function signature and its usages)."
    }

    fn kind(&self) -> Kind {
        Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": { "type": "string", "description": "Path to the file to edit" },
                            "old_string": { "type": "string", "description": "The exact string to replace" },
                            "new_string": { "type": "string", "description": "The new string to insert" }
                        },
                        "required": ["file_path", "old_string", "new_string"]
                    }
                }
            },
            "required": ["edits"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: MultiEditToolParams = serde_json::from_value(params)?;
        Ok(Box::new(MultiEditToolInvocation::new(
            self.config.clone(),
            params,
            Some(self.name().to_string()),
            Some(self.display_name().to_string()),
            self.global_state.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这组测试共用一个**进程级**环境变量（`STAR_DISABLE_READ_CHECK`）。
    ///
    /// `set_var` 是进程全局的：并行跑时"把门关掉"那个测试的窗口会漏给别的
    /// 测试，于是"该拦的没拦"——看起来像前置门坏了，其实是环境变量串了。
    /// 碰这个变量的测试必须拿同一把锁。
    fn env_gate() -> &'static tokio::sync::Mutex<()> {
        static GATE: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
        GATE.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    /// 生命周期测试的公共底座：临时目录 + 指向它的真实 `Config` + 真实
    /// `GlobalState`，并在结束时清掉自己造出来的所有东西。
    ///
    /// 刻意走 `create_invocation` + `execute` 的完整链路，而不是直接 new 一个
    /// Invocation：只有这样才会经过 serde 反序列化（`edits` 少字段要报错）、
    /// 前置 read-state 门、以及最终写盘。绕过 Invocation 层的测试，测的是内部
    /// 函数而不是工具。
    struct Harness {
        dir: tempfile::TempDir,
        config: Arc<crate::core::config::Config>,
        global_state: Arc<GlobalState>,
    }

    impl Harness {
        async fn new(tag: &str) -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let mut params = crate::core::config::ConfigParameters::default();
            params.model = "test-model".to_string();
            params.core_tools = Some(Vec::new());
            params.mcp_enabled = Some(false);
            params.enable_agents = Some(false);
            // 必须是 `target_dir` 而不是 `cwd`：所有工具都用 `config.target_dir()`
            // 解析相对路径，`cwd` 是另一个字段，设了它 target_dir 仍然是真实仓库目录。
            params.target_dir = dir.path().to_path_buf();
            let mut config = crate::core::config::Config::new(params);
            config
                .initialize()
                .await
                .expect("test config must initialize");
            assert_eq!(
                config.target_dir(),
                dir.path(),
                "harness 必须把 target_dir 指到临时目录，否则工具会在真实仓库里读写"
            );
            Self {
                dir,
                config: Arc::new(config),
                global_state: Arc::new(GlobalState::new()),
            }
        }

        fn write(&self, rel: &str, body: &str) -> std::path::PathBuf {
            let path = self.dir.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent dir");
            }
            std::fs::write(&path, body).expect("write fixture file");
            path
        }

        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.dir.path().join(rel)).expect("read back")
        }

        /// 模拟"这个文件被 Read 过"：走真实的登记函数，和工具内部写表的是同一条路。
        async fn mark_read(&self, rel: &str) {
            let path = self.dir.path().join(rel);
            let key = crate::core::utils::paths::read_state_key(&path);
            let content = std::fs::read_to_string(&path).expect("read for mark_read");
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            crate::core::tools::read_file::record_file_read(
                &self.global_state,
                &key,
                content,
                now,
                now,
            )
            .await;
        }

        /// 模拟 `Glob` 登记：只写 `read_file_state`，**不写** `execution_state`，
        /// 且 content 是空占位符（"列出来过"而不是"读过内容"）。
        ///
        /// 这个形状是判别式用例：前面那个 `mark_read` 两张表都写，所以哪怕门查错了
        /// 表也能过 —— 只有走这条路的文件（`Glob` / `Grep` / 批量 `Read` /
        /// `read_many_files`）才能区分门到底查的是哪张表。
        async fn mark_browsed(&self, rel: &str) {
            let path = self.dir.path().join(rel);
            let key = crate::core::utils::paths::read_state_key(&path);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let mut state = self.global_state.read_file_state.write().await;
            state.insert(
                key,
                crate::core::state::ReadFileState {
                    content: String::new(),
                    timestamp: now,
                    file_system_timestamp: now,
                },
            );
        }

        async fn run(&self, params: serde_json::Value) -> ToolResult {
            let tool = MultiEditTool::new(self.config.clone(), self.global_state.clone());
            tool.create_invocation(params)
                .expect("create_invocation must accept valid params")
                .execute(None, None)
                .await
                .expect("execute must not return Err for a valid call")
        }
    }

    /// 完整成功路径：前置门放行 → 真正改到磁盘 → 结果里带上改动摘要。
    #[tokio::test]
    async fn a_read_file_is_edited_on_disk_and_reported() {
        let h = Harness::new("multi-edit-ok").await;
        h.write("a.rs", "fn a() {}\n");
        h.mark_read("a.rs").await;

        let result = h
            .run(json!({
                "edits": [{
                    "file_path": "a.rs",
                    "old_string": "fn a() {}",
                    "new_string": "fn a() { todo!() }"
                }]
            }))
            .await;

        assert!(result.error.is_none(), "不该报错：{:?}", result.error);
        assert_eq!(h.read("a.rs"), "fn a() { todo!() }\n", "磁盘必须真被改了");
        let output = result.llm_content.expect("成功结果必须有 llm_content");
        assert!(
            output.contains("a.rs") || output.contains("1 file"),
            "结果里应当说明改了什么：{output:?}"
        );
    }

    /// 前置门：**读过的文件放行**。
    ///
    /// 这条在改动前就会挂 —— 门查的是 `execution_state`，而登记只写
    /// `read_file_state`，所以 `Read` → `multi_edit` 是 100% 误报。
    #[tokio::test]
    async fn a_read_file_passes_the_gate() {
        let h = Harness::new("multi-edit-gate-ok").await;
        h.write("a.rs", "alpha\n");
        h.mark_read("a.rs").await;

        let result = h
            .run(json!({
                "edits": [{ "file_path": "a.rs", "old_string": "alpha", "new_string": "beta" }]
            }))
            .await;

        assert!(
            result.error.is_none(),
            "登记过的文件必须放行：{:?}",
            result.error
        );
        assert_eq!(h.read("a.rs"), "beta\n");
    }

    /// 同一个文件被改多处时，未读清单**只列一次**。
    ///
    /// 回归：清单是按**编辑条目**统计的，一次 multi_edit 对同一个文件做 8 处
    /// 替换（很常见），清单就把同一个路径列 8 遍、报错说"8 个文件"，下一步还
    /// 让模型连着调 8 次一模一样的 `Read` —— 看起来像 8 个文件，其实只有 1 个。
    #[tokio::test]
    async fn repeated_edits_to_one_file_are_listed_once() {
        let _guard = env_gate().lock().await;
        let h = Harness::new("multi-edit-gate-dedup").await;
        h.write("a.rs", "alpha\nbeta\ngamma\n");
        // 刻意不 mark_read

        let edits: Vec<serde_json::Value> = ["alpha", "beta", "gamma"]
            .iter()
            .map(|token| {
                serde_json::json!({
                    "file_path": "a.rs",
                    "old_string": token,
                    "new_string": format!("{}!", token),
                })
            })
            .collect();

        let result = h.run(serde_json::json!({ "edits": edits })).await;

        let message = result.output.clone();
        assert!(
            message.contains("1 file(s)"),
            "只该算 1 个文件：{message:?}"
        );
        assert_eq!(
            message.matches("  - a.rs").count(),
            1,
            "同一个路径只能列一次：{message:?}"
        );
        assert_eq!(
            message.matches("Read('a.rs')").count(),
            1,
            "下一步也只能调一次 Read：{message:?}"
        );
    }

    /// 前置门：**没读过的文件拦下**，且错误里带得上模型能照做的下一步。
    #[tokio::test]
    async fn an_unread_file_is_blocked_with_an_actionable_message() {
        let _guard = env_gate().lock().await;
        let h = Harness::new("multi-edit-gate-blocked").await;
        h.write("a.rs", "alpha\n");
        // 刻意不 mark_read

        let result = h
            .run(json!({
                "edits": [{ "file_path": "a.rs", "old_string": "alpha", "new_string": "beta" }]
            }))
            .await;

        let err = result.error.expect("未读取必须报错");
        assert_eq!(
            err.error_type,
            crate::core::tools::constants::EDIT_FILE_NOT_READ_MARKER,
            "error_type 必须是 agent 那边的标记，否则它识别不出要补哪一步"
        );
        let message = result.output.clone();
        assert!(
            message.contains("[edit_file_not_read]"),
            "正文要带标记：{message:?}"
        );
        assert!(
            message.contains("Read('a.rs')"),
            "必须告诉模型具体要读哪个文件：{message:?}"
        );
        assert_eq!(h.read("a.rs"), "alpha\n", "被拦下时磁盘不能被改");
    }

    /// 判别式回归：门必须查 `read_file_state`，不能查 `execution_state`。
    ///
    /// `Glob` / `Grep` / 批量 `Read` / `read_many_files` **只**写 `read_file_state`；
    /// 只有单个 `Read` 和 `@` 内联会顺带写 `execution_state`。门查错表的后果是
    /// "Glob 完再 multi_edit" 100% 误报 `[edit_file_not_read]` —— 而这正是
    /// agent 最常见的用法：先搜出文件，再批量改。
    ///
    /// 这条在修复前会挂；同文件里 `a_read_file_passes_the_gate` 挂不了（它走
    /// `record_file_read`，两张表都写），所以两条缺一不可。
    #[tokio::test]
    async fn a_file_only_glob_browsed_passes_the_gate() {
        let h = Harness::new("multi-edit-gate-glob").await;
        h.write("a.rs", "alpha\n");
        h.mark_browsed("a.rs").await;

        let result = h
            .run(json!({
                "edits": [{ "file_path": "a.rs", "old_string": "alpha", "new_string": "beta" }]
            }))
            .await;

        assert!(
            result.error.is_none(),
            "只被 Glob 列出的文件也必须放行（门只能查 read_file_state）：{:?}",
            result.error
        );
        assert_eq!(h.read("a.rs"), "beta\n");
    }

    /// `STAR_DISABLE_READ_CHECK=1` 是逃生口，必须真的关掉这道门。
    #[tokio::test]
    async fn the_read_gate_can_be_disabled_by_env() {
        let _guard = env_gate().lock().await;
        let h = Harness::new("multi-edit-gate-off").await;
        h.write("a.rs", "alpha\n");

        std::env::set_var("STAR_DISABLE_READ_CHECK", "true");
        let result = h
            .run(json!({
                "edits": [{ "file_path": "a.rs", "old_string": "alpha", "new_string": "beta" }]
            }))
            .await;
        std::env::remove_var("STAR_DISABLE_READ_CHECK");

        assert!(
            result.error.is_none(),
            "逃生口打开后不该再拦：{:?}",
            result.error
        );
        assert_eq!(h.read("a.rs"), "beta\n");
    }

    /// 多文件批量：一次调用改多个文件，全部都要落盘。
    #[tokio::test]
    async fn several_files_are_edited_in_one_call() {
        let h = Harness::new("multi-edit-multi").await;
        h.write("a.rs", "alpha\n");
        h.write("nested/b.rs", "beta\n");
        h.mark_read("a.rs").await;
        h.mark_read("nested/b.rs").await;

        let result = h
            .run(json!({ "edits": [
                { "file_path": "a.rs", "old_string": "alpha", "new_string": "A" },
                { "file_path": "nested/b.rs", "old_string": "beta", "new_string": "B" }
            ]}))
            .await;

        assert!(result.error.is_none(), "不该报错：{:?}", result.error);
        assert_eq!(h.read("a.rs"), "A\n");
        assert_eq!(h.read("nested/b.rs"), "B\n");
    }

    /// 参数校验属于生命周期的一部分：缺 `edits` 必须在 `create_invocation` 就被拒。
    #[tokio::test]
    async fn missing_required_params_are_rejected_at_creation() {
        let h = Harness::new("multi-edit-badparams").await;
        let tool = MultiEditTool::new(h.config.clone(), h.global_state.clone());
        assert!(
            tool.create_invocation(json!({})).is_err(),
            "缺 required 字段必须在建 invocation 时就报错"
        );
        assert!(
            tool.create_invocation(json!({ "edits": [{ "file_path": "a.rs" }] }))
                .is_err(),
            "edits 里少 old_string/new_string 也必须报错"
        );
    }

    /// `old_string` 匹配不上必须报错，且磁盘保持原样 —— 不能"半成功"。
    #[tokio::test]
    async fn a_non_matching_old_string_errors_without_touching_the_file() {
        let h = Harness::new("multi-edit-nomatch").await;
        h.write("a.rs", "alpha\n");
        h.mark_read("a.rs").await;

        let result = h
            .run(json!({
                "edits": [{ "file_path": "a.rs", "old_string": "zzz-not-there", "new_string": "x" }]
            }))
            .await;

        assert!(result.error.is_some(), "匹配不上必须报错");
        assert_eq!(h.read("a.rs"), "alpha\n", "失败时磁盘必须原封不动");
    }
}
