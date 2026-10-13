/// vim 模式。
///
/// 只有 Normal / Insert / Visual 三个：**Command 是空壳** —— 按 `:` 会切进去，
/// 下一个键无论是什么都直接回 Normal（`events/input.rs`），`:` 之后的命令解析
/// 从未实现。与其留一个会骗人的模式，不如把分支删掉，让 `:` 落回未知键处理。
#[derive(Debug, Clone, PartialEq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
}

pub struct VimState {
    pub mode: VimMode,
    pub pending_keys: String,
    pub register: String,
    pub last_motion: Option<String>,
    pub count: usize,
}

impl VimState {
    pub fn new() -> Self {
        Self {
            mode: VimMode::Normal,
            pending_keys: String::new(),
            register: String::new(),
            last_motion: None,
            count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.pending_keys.clear();
        self.count = 0;
    }

    pub fn is_normal_mode(&self) -> bool {
        self.mode == VimMode::Normal
    }

    pub fn is_insert_mode(&self) -> bool {
        self.mode == VimMode::Insert
    }

    pub fn is_visual_mode(&self) -> bool {
        self.mode == VimMode::Visual
    }
}
