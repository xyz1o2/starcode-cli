//! 编译期内嵌的 i18n 词典。
//!
//! 与系统提示词同一套模式（`rust_embed`）：release 构建把 `i18n/*.json`
//! 打进二进制，debug 构建仍从文件系统读（改词典立刻生效，不用重编）。
//!
//! **为什么必须内嵌**：release 产物只打包二进制 + LICENSE + README，
//! `i18n/` 目录并不在包里。词典不内嵌的话，用户机器上永远读不到词典，
//! 所有文案只能靠调用点写死的字面量兜底 —— 那正是 `t(key, zh, en)`
//! 三个参数的历史包袱。
//!
//! 词典优先于调用点字面量，所以内嵌之后调用点可以不写文案：
//! `i18n::t1("ui.input.placeholder")` 就够了。

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "i18n"]
pub struct I18nDictionaries;

/// 读取内嵌词典的原始 JSON 文本。
pub fn embedded_dictionary(lang_code: &str) -> Option<String> {
    let filename = format!("{}.json", lang_code);
    I18nDictionaries::get(&filename)
        .map(|file| String::from_utf8_lossy(file.data.as_ref()).into_owned())
}
