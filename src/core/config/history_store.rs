//! 命令历史持久化 — 按项目目录分桶存到 ~/.star/history/<cwd-hash>.json
//! （对标 Claude Code 的 ~/.claude/history.json 按项目存储，上限 100 条）

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

pub const HISTORY_MAX_ENTRIES: usize = 100;

fn history_file() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cwd.hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    Some(
        dirs::home_dir()?
            .join(".star")
            .join("history")
            .join(format!("{}.json", key)),
    )
}

/// 启动时加载当前项目的历史（ newest 在前）
pub fn load_history() -> VecDeque<String> {
    let Some(path) = history_file() else {
        return VecDeque::new();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return VecDeque::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<String>>(&content) else {
        return VecDeque::new();
    };
    entries.into_iter().take(HISTORY_MAX_ENTRIES).collect()
}

/// 历史变化后调用，写回磁盘（文件很小，同步写即可）
pub fn save_history(history: &VecDeque<String>) {
    let Some(path) = history_file() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let entries: Vec<&String> = history.iter().take(HISTORY_MAX_ENTRIES).collect();
    if let Ok(json) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::write(&path, json);
    }
}
