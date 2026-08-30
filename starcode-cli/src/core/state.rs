use crate::types::ToolResult;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ReadFileState {
    pub content: String,
    pub timestamp: u128,             // Milliseconds timestamp
    pub file_system_timestamp: u128, // File system mtime
}

#[derive(Debug, Clone)]
pub struct FileSnapshot {
    pub content: String,
    pub timestamp: u128,
    pub hash: String, // SHA256 hash for detailed change tracking
}

/// ============ P1.1 改进：完善 execution_state 机制 ============
/// 追踪文件操作的完整生命周期
#[derive(Debug, Clone)]
pub struct FileOperationState {
    /// 文件是否已被读取
    pub was_read: bool,
    /// 最后一次读取的快照
    pub last_read_snapshot: Option<FileSnapshot>,
    /// 文件是否已被修改（通过本工具）
    pub was_modified: bool,
    /// 修改历史（用于验证）
    pub modification_count: u32,
}

impl Default for FileOperationState {
    fn default() -> Self {
        Self {
            was_read: false,
            last_read_snapshot: None,
            was_modified: false,
            modification_count: 0,
        }
    }
}

/// ============ P1.1 改进：execution_state 机制扩展 ============
/// 会话级别的执行状态追踪
#[derive(Debug, Clone)]
pub struct ExecutionState {
    /// 已读文件及其状态
    pub file_states: HashMap<String, FileOperationState>,
    /// 已执行的 shell 命令
    pub executed_commands: Vec<CommandExecution>,
    /// 会话创建的对象（文件、目录等）
    pub created_objects: HashMap<String, ObjectMetadata>,
}

#[derive(Debug, Clone)]
pub struct CommandExecution {
    pub command: String,
    pub timestamp: u128,
    pub exit_code: Option<i32>,
    pub output: String,
}

#[derive(Debug, Clone)]
pub struct ObjectMetadata {
    pub object_type: String, // "file" | "directory" | "notebook" | etc.
    pub path: String,
    pub created_at: u128,
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            file_states: HashMap::new(),
            executed_commands: Vec::new(),
            created_objects: HashMap::new(),
        }
    }

    /// 标记文件已被读取
    pub fn mark_file_read(&mut self, path: String, snapshot: FileSnapshot) {
        let entry = self.file_states.entry(path).or_default();
        entry.was_read = true;
        entry.last_read_snapshot = Some(snapshot);
    }

    /// 标记文件已被修改
    pub fn mark_file_modified(&mut self, path: String) {
        if let Some(state) = self.file_states.get_mut(&path) {
            state.was_modified = true;
            state.modification_count += 1;
        } else {
            let mut new_state = FileOperationState::default();
            new_state.was_modified = true;
            new_state.modification_count = 1;
            self.file_states.insert(path, new_state);
        }
    }

    /// 检查文件是否已被读取
    pub fn was_file_read(&self, path: &str) -> bool {
        self.file_states
            .get(path)
            .map(|s| s.was_read)
            .unwrap_or(false)
    }

    /// 注册创建的对象
    pub fn register_created_object(&mut self, path: String, object_type: String) {
        let metadata = ObjectMetadata {
            object_type,
            path: path.clone(),
            created_at: current_timestamp_ms(),
        };
        self.created_objects.insert(path, metadata);
    }

    /// 检查对象是否由本会话创建
    pub fn was_object_created(&self, path: &str) -> bool {
        self.created_objects.contains_key(path)
    }
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self::new()
    }
}

/// 获取当前时间戳（毫秒）
pub fn current_timestamp_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Debug, Clone)]
pub struct CachedToolResult {
    pub result: ToolResult,
    pub timestamp: u128,
}

#[derive(Debug, Clone)]
pub struct GlobalState {
    pub read_file_state: Arc<RwLock<HashMap<String, ReadFileState>>>,
    pub tool_cache: Arc<RwLock<HashMap<String, CachedToolResult>>>,
    /// ============ P1.1 改进：会话级执行状态 ============
    pub execution_state: Arc<RwLock<ExecutionState>>,
    /// Current UI message id for file-history checkpoint association.
    /// Set by the runtime layer at the start of each user message round;
    /// read by write tools (write_file / edit / multi_edit) inside `execute`
    /// before calling `checkpoint_manager::track_edit`. None when the tool
    /// runs outside a message context (e.g. tests, headless).
    pub current_message_id: Arc<RwLock<Option<u64>>>,
}

impl GlobalState {
    pub fn new() -> Self {
        Self {
            read_file_state: Arc::new(RwLock::new(HashMap::new())),
            tool_cache: Arc::new(RwLock::new(HashMap::new())),
            execution_state: Arc::new(RwLock::new(ExecutionState::new())),
            current_message_id: Arc::new(RwLock::new(None)),
        }
    }

    /// Read the current message id (best-effort; None if not in a message round).
    pub async fn current_message_id(&self) -> Option<u64> {
        *self.current_message_id.read().await
    }

    /// Set the current message id at the start of a user message round.
    pub async fn set_current_message_id(&self, id: Option<u64>) {
        *self.current_message_id.write().await = id;
    }
}

impl Default for GlobalState {
    fn default() -> Self {
        Self::new()
    }
}
