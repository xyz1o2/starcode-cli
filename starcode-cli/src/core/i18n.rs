use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use crate::core::config::json_with_comments::parse_json_with_comments;
use crate::core::config::storage::Storage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiLanguage {
    En,
    ZhCN,
}

impl Default for UiLanguage {
    fn default() -> Self {
        UiLanguage::En
    }
}

impl UiLanguage {
    pub fn as_code(&self) -> &'static str {
        match self {
            UiLanguage::En => "en-US",
            UiLanguage::ZhCN => "zh-CN",
        }
    }
}

#[derive(Debug, Default, Clone)]
struct I18nState {
    lang: UiLanguage,
    dict: HashMap<String, String>,
}

static I18N_STATE: OnceLock<RwLock<I18nState>> = OnceLock::new();

fn state() -> &'static RwLock<I18nState> {
    I18N_STATE.get_or_init(|| RwLock::new(I18nState::default()))
}

pub fn init(language_setting: Option<&str>, project_root: &Path) -> UiLanguage {
    let lang = resolve_ui_language(language_setting);
    let dict = load_dictionary(lang, project_root);
    if let Ok(mut guard) = state().write() {
        guard.lang = lang;
        guard.dict = dict;
    }
    lang
}

pub fn reload_for_language(lang: UiLanguage, project_root: &Path) -> Result<(), String> {
    let dict = load_dictionary(lang, project_root);
    let mut guard = state().write().map_err(|_| "i18n lock poisoned")?;
    guard.lang = lang;
    guard.dict = dict;
    Ok(())
}

pub fn current_language() -> UiLanguage {
    state().read().map(|g| g.lang).unwrap_or_default()
}

pub fn t(key: &str, zh: &str, en: &str) -> String {
    if let Ok(guard) = state().read() {
        if let Some(value) = guard.dict.get(key) {
            return value.clone();
        }
        return match guard.lang {
            UiLanguage::ZhCN => zh.to_string(),
            UiLanguage::En => en.to_string(),
        };
    }

    en.to_string()
}

/// Returns all supported language codes in the order they should be displayed.
pub fn available_languages() -> &'static [(&'static str, &'static str)] {
    &[
        ("auto", "System default"),
        ("en-US", "English"),
        ("zh-CN", "中文（简体）"),
    ]
}

pub fn status_prefixes() -> &'static [&'static str] {
    &[
        "状态：",
        "完成：",
        "异常：",
        "警告：",
        "Status: ",
        "Done: ",
        "Error: ",
        "Warning: ",
    ]
}

pub fn running_prefixes() -> &'static [&'static str] {
    &["⏳ 执行中：", "⏳ Running: ", "[RUN] ", "RUN "]
}

pub fn normalize_language_setting(input: &str) -> Option<&'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_auto_language_setting(trimmed) {
        return Some("auto");
    }
    parse_language_code(trimmed).map(|lang| lang.as_code())
}

pub fn resolve_ui_language(setting: Option<&str>) -> UiLanguage {
    if let Some(setting) = setting {
        let trimmed = setting.trim();
        if !trimmed.is_empty() {
            if is_auto_language_setting(trimmed) {
                return detect_language_from_env().unwrap_or_default();
            }
            if let Some(lang) = parse_language_code(trimmed) {
                return lang;
            }
        }
    }

    if let Ok(env_lang) = std::env::var("STAR_UI_LANGUAGE") {
        return resolve_ui_language(Some(env_lang.as_str()));
    }

    UiLanguage::En
}

fn is_auto_language_setting(input: &str) -> bool {
    let normalized = input.trim().to_lowercase();
    matches!(normalized.as_str(), "auto" | "system" | "default")
}

fn parse_language_code(input: &str) -> Option<UiLanguage> {
    let normalized = input.trim().to_lowercase().replace('_', "-");
    match normalized.as_str() {
        "en" | "en-us" | "en-us.utf8" | "en-us.utf-8" | "english" => Some(UiLanguage::En),
        "zh" | "zh-cn" | "zh-hans" | "zh-cn.utf8" | "zh-cn.utf-8" | "cn" | "chinese" => {
            Some(UiLanguage::ZhCN)
        }
        _ => None,
    }
}

fn detect_language_from_env() -> Option<UiLanguage> {
    let keys = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"];
    for key in keys {
        if let Ok(value) = std::env::var(key) {
            if let Some(lang) = parse_language_from_locale(&value) {
                return Some(lang);
            }
        }
    }
    None
}

fn parse_language_from_locale(value: &str) -> Option<UiLanguage> {
    let normalized = value.trim().to_lowercase().replace('_', "-");
    if normalized.starts_with("zh") {
        return Some(UiLanguage::ZhCN);
    }
    if normalized.starts_with("en") {
        return Some(UiLanguage::En);
    }
    None
}

fn load_dictionary(lang: UiLanguage, project_root: &Path) -> HashMap<String, String> {
    let mut dict = HashMap::new();
    let filename = format!("{}.json", lang.as_code());

    let repo_dir = project_root.join("i18n");
    merge_dictionary_file(&mut dict, &repo_dir.join(&filename));

    let global_dir = Storage::global_i18n_dir();
    merge_dictionary_file(&mut dict, &global_dir.join(&filename));

    let project_dir = Storage::new(project_root.to_path_buf()).project_i18n_dir();
    merge_dictionary_file(&mut dict, &project_dir.join(&filename));

    dict
}

fn merge_dictionary_file(dict: &mut HashMap<String, String>, path: &PathBuf) {
    if !path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return,
    };

    let parsed: HashMap<String, String> = match parse_json_with_comments(&content) {
        Ok(parsed) => parsed,
        Err(_) => return,
    };

    dict.extend(parsed);
}
