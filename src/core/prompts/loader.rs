//! 运行时提示词加载器
//!
//! 所有提示词均从 `.md` 文件加载，代码中不写死提示词文本。
//! 加载顺序（优先级从高到低）：
//! 1. 外部目录：`STAR_PROMPT_DIR` 环境变量指定的目录
//! 2. 外部目录：`~/.starcode/prompts/`
//! 3. 外部目录：当前项目 `.star/prompts/`（相对 cwd）
//! 4. 编译期内嵌资源（`SystemPrompts`，rust_embed）
//!
//! 外部文件支持热更新：缓存按文件修改时间（mtime）失效。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::core::prompts::SystemPrompts;

/// 外部文件缓存：filename → (mtime, content)
type PromptCache = Mutex<HashMap<String, (Option<SystemTime>, String)>>;

fn cache() -> &'static PromptCache {
    static CACHE: OnceLock<PromptCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 外部提示词目录（按优先级顺序）
fn external_prompt_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Some(dir) = std::env::var_os("STAR_PROMPT_DIR") {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir));
        }
    }

    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".starcode").join("prompts"));
    }

    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join(".star").join("prompts"));
    }

    dirs
}

/// 从编译期内嵌资源加载提示词
pub fn embedded_prompt(filename: &str) -> Option<String> {
    SystemPrompts::get(filename)
        .map(|file| String::from_utf8_lossy(file.data.as_ref()).into_owned())
}

/// 从外部目录加载提示词（带 mtime 缓存，支持热更新）
fn read_external_from_dirs(filename: &str, dirs: &[PathBuf]) -> Option<String> {
    for dir in dirs {
        let path = dir.join(filename);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().ok();

        if let Ok(guard) = cache().lock() {
            if let Some((cached_mtime, content)) = guard.get(filename) {
                if *cached_mtime == mtime {
                    return Some(content.clone());
                }
            }
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut guard) = cache().lock() {
                guard.insert(filename.to_string(), (mtime, content.clone()));
            }
            return Some(content);
        }
    }
    None
}

/// 从外部目录加载提示词（带 mtime 缓存，支持热更新）
fn read_external(filename: &str) -> Option<String> {
    read_external_from_dirs(filename, &external_prompt_dirs())
}

/// 按指定目录顺序加载提示词（测试/工具用，不依赖环境变量）
pub fn load_prompt_with_dirs(filename: &str, dirs: &[PathBuf]) -> String {
    read_external_from_dirs(filename, dirs)
        .or_else(|| embedded_prompt(filename))
        .unwrap_or_default()
}

/// 加载提示词：外部目录优先，编译期内嵌兜底
pub fn load_prompt(filename: &str) -> String {
    try_load_prompt(filename).unwrap_or_default()
}

/// 加载提示词，找不到返回 `None`
pub fn try_load_prompt(filename: &str) -> Option<String> {
    read_external(filename).or_else(|| embedded_prompt(filename))
}

/// 占位符替换：将模板中的 `{key}` 替换为对应的 value
pub fn render_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{}}}", key), value);
    }
    out
}
