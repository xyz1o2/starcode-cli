#[derive(Debug, Clone, PartialEq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    Command,
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

    pub fn is_command_mode(&self) -> bool {
        self.mode == VimMode::Command
    }
}
