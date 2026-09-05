use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolError, ToolInvocation,
    ToolLocation, ToolResult as CoreToolResult,
};
use crate::core::utils::file_utils::format_file_size;
use crate::core::utils::paths::normalize_cross_platform_path;
use crate::utils::file_walk::{walk, WalkOptions};
use serde::Deserialize;
use serde_json::{json, Value};
/// List Directory Tool - AI-Friendly File Browsing
///
/// 类似 Unix `ls` / Windows `dir`，但对 AI 更友好
///
/// 特点：
/// - 结构化输出：文件/目录分开，带元数据
/// - 智能排序：文件夹优先、字母排序
/// - 灵活过滤：按扩展名、大小、时间过滤
/// - Tree 模式：递归显示目录树
///
/// 遍历口径统一走 [`crate::utils::file_walk`]：`.gitignore` / `.starignore`
/// 生效、dotfile 默认可见、6 个 VCS 目录永不进入。
use std::path::{Path, PathBuf};

/// 单次列举上限。之前没有上限，且完全不认 `.gitignore` —— 在本仓库对根
/// 目录跑一次 `recursive: true`，`target/` 会被整棵吐给模型。
const MAX_ENTRIES: usize = 1000;

#[derive(Clone)]
pub struct ListDirTool;

#[derive(Debug, Deserialize, Clone)]
pub struct ListDirParams {
    pub directory: String,
    pub recursive: Option<bool>,
    pub max_depth: Option<usize>,
    pub filter_ext: Option<Vec<String>>,
    pub include_hidden: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub extension: Option<String>,
    pub modified: Option<String>,
}

pub struct ListDirToolInvocation {
    tool: ListDirTool,
    params: ListDirParams,
}

impl ToolInvocation for ListDirToolInvocation {
    fn get_description(&self) -> String {
        format!("List directory: {}", self.params.directory)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![ToolLocation {
            path: normalize_cross_platform_path(&self.params.directory),
            location_type: crate::core::tools::tools::LocationType::Read,
        }]
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
        let dir = self.params.directory.clone();
        let recursive = self.params.recursive;
        let max_depth = self.params.max_depth;
        let filter = self.params.filter_ext.clone();
        let hidden = self.params.include_hidden;

        Box::pin(async move {
            let result = tool
                .list(&dir, recursive, max_depth, filter, hidden)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            Ok(result)
        })
    }
}

impl BaseDeclarativeTool for ListDirTool {
    fn name(&self) -> &str {
        "ListDir"
    }

    fn display_name(&self) -> &str {
        "List Directory"
    }

    fn description(&self) -> &str {
        "List files and directories with metadata. Supports recursive listing (tree view) and filtering."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Path to the directory to list"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "Whether to list recursively (tree view)",
                    "default": false
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum depth for recursive listing",
                    "default": 3
                },
                "filter_ext": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Filter by file extensions (e.g., ['rs', 'toml'])"
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "Whether to include dotfiles and dot-directories. Defaults to true; `.git`, `.svn`, `.hg`, `.bzr`, `.jj` and `.sl` are always skipped. Paths ignored by .gitignore / .starignore are never listed.",
                    "default": true
                }
            },
            "required": ["directory"]
        })
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ListDirParams = serde_json::from_value(params)?;
        Ok(Box::new(ListDirToolInvocation {
            tool: self.clone(),
            params,
        }))
    }
}

impl ListDirTool {
    pub fn new() -> Self {
        Self
    }

    async fn list(
        &self,
        directory: &str,
        recursive: Option<bool>,
        max_depth: Option<usize>,
        filter_ext: Option<Vec<String>>,
        include_hidden: Option<bool>,
    ) -> Result<CoreToolResult, Box<dyn std::error::Error>> {
        let recursive = recursive.unwrap_or(false);
        let max_depth = max_depth.unwrap_or(3);
        // dotfile 默认可见 —— 对齐 Claude Code。`.github/workflows`、
        // `.env.example` 这类文件本来就是要看的。
        let include_hidden = include_hidden.unwrap_or(true);

        let dir_path = normalize_cross_platform_path(directory);
        let display_dir = dir_path.to_string_lossy().to_string();

        // 检查是否为目录
        if !dir_path.exists() {
            let msg = format!("Directory not found: {}", display_dir);
            return Ok(CoreToolResult {
                llm_content: None,
                return_display: None,
                output: msg.clone(),
                error: Some(ToolError {
                    error_type: "execution_error".to_string(),
                    message: msg,
                }),
                data: None,
            });
        }

        if !dir_path.is_dir() {
            let msg = format!("'{}' is not a directory", display_dir);
            return Ok(CoreToolResult {
                llm_content: None,
                return_display: None,
                output: msg.clone(),
                error: Some(ToolError {
                    error_type: "execution_error".to_string(),
                    message: msg,
                }),
                data: None,
            });
        }

        // 列出条目。`list_recursive(depth=0, max_depth=N)` 以前能下到相对
        // 深度 N+1，这里保持同一口径。
        let walk_depth = if recursive {
            max_depth.saturating_add(1)
        } else {
            1
        };
        let scan_root = dir_path.clone();
        let (entries, truncated) = tokio::task::spawn_blocking(move || {
            Self::collect(&scan_root, walk_depth, &filter_ext, include_hidden)
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

        if entries.is_empty() {
            let msg = format!(
                "📁 Directory '{}' is empty (or all files filtered out)",
                display_dir
            );
            return Ok(CoreToolResult {
                llm_content: Some(msg.clone()),
                return_display: Some(msg.clone()),
                output: msg,
                error: None,
                data: None,
            });
        }

        // 格式化输出
        let output = self.format_entries(&entries, &display_dir, recursive, truncated);

        Ok(CoreToolResult {
            llm_content: Some(output.clone()),
            return_display: Some(output.clone()),
            output: output.clone(),
            error: None,
            data: None,
        })
    }

    /// 收集条目。`walk_depth` 是相对根目录的最大深度（1 = 只列直接子项）。
    ///
    /// 以前这里是两份 `fs::read_dir` 实现 + `name.starts_with('.')`：不认任何
    /// ignore 文件，`recursive: true` 会一头钻进 `target/`；而 `.github/`
    /// 这类真正要看的目录反倒被当"隐藏文件"跳掉。
    ///
    /// 返回值第二项表示是否因为 [`MAX_ENTRIES`] 截断。
    fn collect(
        dir_path: &Path,
        walk_depth: usize,
        filter_ext: &Option<Vec<String>>,
        include_hidden: bool,
    ) -> (Vec<DirEntry>, bool) {
        let opts = WalkOptions::new()
            .hidden(include_hidden)
            .max_depth(walk_depth);
        let mut entries: Vec<DirEntry> = Vec::new();
        let mut truncated = false;

        for entry in walk(dir_path, &opts).flatten() {
            if entry.path() == dir_path {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let path = entry.path().to_path_buf();

            // 扩展名过滤（仅对文件）
            if !metadata.is_dir() {
                if let Some(exts) = filter_ext {
                    match path.extension() {
                        Some(ext) => {
                            let ext_str = ext.to_string_lossy().to_lowercase();
                            if !exts.iter().any(|e| e.to_lowercase() == ext_str) {
                                continue;
                            }
                        }
                        // 设了过滤器时，无扩展名的文件不算命中
                        None => continue,
                    }
                }
            }

            if entries.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            entries.push(Self::create_entry(name, path, metadata));
        }

        // 排序：目录在前，然后按名称排序
        entries.sort_by(|a, b| {
            if a.is_dir != b.is_dir {
                b.is_dir.cmp(&a.is_dir) // 目录优先
            } else {
                a.name.cmp(&b.name)
            }
        });

        (entries, truncated)
    }

    fn create_entry(name: String, path: PathBuf, metadata: std::fs::Metadata) -> DirEntry {
        let size = if metadata.is_file() {
            Some(metadata.len())
        } else {
            None
        };

        let extension = path.extension().map(|e| e.to_string_lossy().to_string());

        // 简化时间显示
        let modified = metadata.modified().ok().map(|t| {
            let datetime: chrono::DateTime<chrono::Local> = t.into();
            datetime.format("%Y-%m-%d %H:%M:%S").to_string()
        });

        DirEntry {
            name,
            path: path.to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
            size,
            extension,
            modified,
        }
    }

    fn format_entries(
        &self,
        entries: &[DirEntry],
        root_dir: &str,
        recursive: bool,
        truncated: bool,
    ) -> String {
        let mut output = String::new();
        output.push_str(&format!("📁 Directory Listing: {}\n", root_dir));
        output.push_str("--------------------------------------------------\n");
        output.push_str("Type  | Size       | Modified            | Name\n");
        output.push_str("------+------------+---------------------+------\n");

        for entry in entries {
            let type_icon = if entry.is_dir { "📂" } else { "📄" };
            let size_str = entry
                .size
                .map(|s| format_file_size(s))
                .unwrap_or_else(|| "-".to_string());
            let mod_str = entry.modified.as_deref().unwrap_or("-");

            // 如果是递归，显示相对路径；否则显示文件名
            let display_name = if recursive {
                // 尝试计算相对路径
                if let Some(rel) = pathdiff::diff_paths(&entry.path, root_dir) {
                    rel.to_string_lossy().to_string()
                } else {
                    entry.name.clone()
                }
            } else {
                entry.name.clone()
            };

            output.push_str(&format!(
                "{} | {:<10} | {:<19} | {}\n",
                type_icon, size_str, mod_str, display_name
            ));
        }

        output.push_str("--------------------------------------------------\n");
        output.push_str(&format!("Total: {} items", entries.len()));
        if truncated {
            output.push_str(&format!(
                " (truncated at {}; narrow the directory or lower max_depth to see the rest)",
                MAX_ENTRIES
            ));
        }

        output
    }
}
