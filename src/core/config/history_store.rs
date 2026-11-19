//! 输入历史按项目分桶；新格式保存解释意图，旧字符串只能恢复为正文。

use crate::types::UserInput;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const HISTORY_MAX_ENTRIES: usize = 100;

pub(crate) fn history_file() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cwd.hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    Some(
        dirs::home_dir()?
            .join(".star")
            .join("history")
            .join(format!("{key}.json")),
    )
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredInput {
    Structured(UserInput),
    Legacy(String),
}

fn decode_history(content: &str) -> VecDeque<UserInput> {
    serde_json::from_str::<Vec<StoredInput>>(content)
        .unwrap_or_default()
        .into_iter()
        .take(HISTORY_MAX_ENTRIES)
        .map(|entry| match entry {
            StoredInput::Structured(input) => input,
            StoredInput::Legacy(text) => text.into(),
        })
        .collect()
}

/// 加载当前项目历史，最新的输入在前。
pub fn load_history() -> VecDeque<UserInput> {
    history_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| decode_history(&content))
        .unwrap_or_default()
}

pub fn save_history(history: &VecDeque<UserInput>) {
    if let Some(path) = history_file() {
        save_history_to(&path, history);
    }
}

pub(crate) fn save_history_to(path: &Path, history: &VecDeque<UserInput>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let entries: Vec<_> = history.iter().take(HISTORY_MAX_ENTRIES).collect();
    if let Ok(json) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InputIntent;

    #[test]
    fn history_preserves_intent_and_legacy_is_literal() {
        let old = decode_history(r#"["!not-a-command", "/memory add text"]"#);
        assert!(old.iter().all(|input| input.intent == InputIntent::Literal));
        let input = UserInput::command_eligible("/help".into());
        let history = VecDeque::from([input.clone(), "/help".to_owned().into()]);
        let saved = serde_json::to_string(&history).unwrap();
        assert_eq!(decode_history(&saved), history);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        save_history_to(&path, &history);
        assert_eq!(
            decode_history(&std::fs::read_to_string(path).unwrap()),
            history
        );
    }
}
