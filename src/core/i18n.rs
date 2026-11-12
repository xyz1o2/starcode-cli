use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use crate::core::config::json_with_comments::parse_json_with_comments;
use crate::core::config::storage::Storage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiLanguage {
    #[default]
    En,
    ZhCN,
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

/// 测试专用：把进程级 i18n 状态复位成「英文 + 内嵌英文词典」。
///
/// `I18N_STATE` 是 `OnceLock<RwLock<…>>` 的进程级全局量，任何一个测试调过
/// `init` / `reload_for_language` 都会改变之后所有 `t()` 的落点，而渲染快照
/// 基线要求同一个 key 永远解析成同一句文案。
///
/// 复位时装载**内嵌**英文词典，而不是清空 —— 快照基线该依赖真实文案，
/// 不该依赖「词典为空时调用点写死的 fallback」。装载内嵌词典后，
/// 基线同时校验了「词典里有这个 key」这件事：漏一个 key 就会在快照里
/// 显示成 key 本身，一眼看得出来。
#[cfg(test)]
pub fn reset_for_tests() {
    if let Ok(mut guard) = state().write() {
        *guard = I18nState::default();
    }
    let mut dict = HashMap::new();
    if let Some(content) = crate::core::i18n_dict::embedded_dictionary(UiLanguage::En.as_code()) {
        merge_dictionary_content(&mut dict, &content);
    }
    if let Ok(mut guard) = state().write() {
        guard.lang = UiLanguage::En;
        guard.dict = dict;
    }
}

/// 单参取文案：文案完全由词典拥有，调用点只给 key。
///
/// # 为什么只有 key
///
/// 曾经有过 `t(key, zh, en)` 三参形式，让调用点同时写中英文两份文案。
/// 两个问题：
///
/// 1. **两份字面量往往一模一样**（"Settings" / "Settings"），读代码的人得
///    逐字比对才能确认它们真的相同，改文案时也容易只改一处。
/// 2. **更糟的是它们根本不生效**。取文案是词典优先（`dict.get(key)` 命中
///    就直接返回），而词典已经编译进二进制（`core::i18n_dict`），所以线上
///    **一定**命中词典 —— 调用点写的是什么永远看不见。写两遍的代价就是
///    这种"改了个寂寞"：以为改了，其实改的是一行死代码。
///
/// 所以：**文案归词典，调用点只留 key。** 改文案只改 `i18n/<lang>.json`。
///
/// # 词典里没有这个 key 会怎样
///
/// 返回 key 本身（`"ui.input.placeholder"`），而不是空字符串。空字符串
/// 在界面上表现为「这一行什么都没写」，很难联想到是漏了词典；返回 key
/// 则一眼看得出「这个 key 没进词典」。
///
/// ```ignore
/// i18n::t1("ui.input.placeholder")
/// ```
pub fn t1(key: &str) -> String {
    if let Ok(guard) = state().read() {
        if let Some(value) = guard.dict.get(key) {
            return value.clone();
        }
    }
    key.to_string()
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
    &["执行中：", "Running: ", "[RUN] ", "RUN "]
}

/// 错误类状态前缀 —— `status_prefixes` 的红色子集。错误条目在聊天区
/// 以 `theme.error` 渲染，服务器原始反馈靠 `error_kind::diagnose` 保证
/// 完整落在正文里（`Original error:` 一段）。
pub fn error_status_prefixes() -> &'static [&'static str] {
    &["异常：", "Error: "]
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

    // 1) 编译期内嵌词典 —— release 产物里唯一一定存在的来源。
    if let Some(content) = crate::core::i18n_dict::embedded_dictionary(lang.as_code()) {
        merge_dictionary_content(&mut dict, &content);
    }

    // 2) 外部词典按优先级覆盖（用户自己的翻译）。
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

    merge_dictionary_content(dict, &content);
}

fn merge_dictionary_content(dict: &mut HashMap<String, String>, content: &str) {
    let parsed: HashMap<String, String> = match parse_json_with_comments(content) {
        Ok(parsed) => parsed,
        Err(_) => return,
    };

    dict.extend(parsed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// 词典是文案的唯一真相源 —— 那它就**必须**覆盖每个被用到的 key。
    ///
    /// 漏一个 key 的表现是界面上一行字变成了 key 本身（`t1` 的兜底就是
    /// 返回 key），但那得用户碰巧走到那条路径才看得见。这里把整棵树扫一遍，
    /// 任何一个 key 缺失就在编译期附近失败，而不是等线上。
    ///
    /// 判据用**窄集** [`called_keys`]：真正流进 `t1` 实参的字面量。宽集
    /// [`referenced_keys`] 是源码里一切点分小写字面量 —— `user.name`、
    /// `t.log.4`、`api.anthropic.com`、`0.0.0.0` 都长得像 key，拿它做
    /// 「缺失」判据会误报一片；它只配给孤儿门用（那边宽松方向是对的，
    /// 这边恰恰相反）。
    #[test]
    fn every_key_used_in_the_source_tree_exists_in_the_dictionary() {
        let en = embedded_dict(UiLanguage::En);
        let zh = embedded_dict(UiLanguage::ZhCN);
        assert!(!en.is_empty(), "内嵌英文词典为空 —— rust_embed 没生效？");
        assert!(!zh.is_empty(), "内嵌中文词典为空 —— rust_embed 没生效？");

        let mut missing = BTreeSet::new();
        for key in called_keys() {
            if !en.contains_key(&key) || !zh.contains_key(&key) {
                missing.insert(key);
            }
        }

        assert!(
            missing.is_empty(),
            "这些 key 没进词典（界面会显示 key 本身）:\n  {}",
            missing.iter().cloned().collect::<Vec<_>>().join("\n  ")
        );
    }

    /// 词典里不该有没人引用的条目。
    ///
    /// 一个孤儿条目的代价不是「多占几行 JSON」—— 是**它看起来还活着**。
    /// 有人改文案时去改那个条目，改完什么都不会变；有人删掉调用点，
    /// 词典里的条目还留着，下一个人以为那个界面还在用那句话。
    ///
    /// 整套 `mode_yolo` 家族就是这么来的：审批模式早改成 `bypass` 了，
    /// `ui.mode.yolo` / `palette.label.mode_yolo` / `palette.desc.mode_yolo`
    /// 却一直躺在词典里。
    #[test]
    fn the_dictionary_has_no_unreferenced_entries() {
        let en = embedded_dict(UiLanguage::En);
        let referenced = referenced_keys();

        let orphans: Vec<String> = en
            .keys()
            .filter(|k| !referenced.contains(*k))
            .cloned()
            .collect();

        assert!(
            orphans.is_empty(),
            "词典里这些 key 没有任何调用点引用（删掉，或让源码引用它们）:\n  {}",
            orphans.join("\n  ")
        );
    }

    /// 两份词典的 key 集合必须完全一致。
    ///
    /// 差一个 key 的意思是：那个界面在英文下有文案、在中文下显示 key（或
    /// 反过来）。这不会让任何测试变红 —— 直到有人切到那种语言。
    #[test]
    fn the_two_dictionaries_cover_the_same_keys() {
        let en = embedded_dict(UiLanguage::En);
        let zh = embedded_dict(UiLanguage::ZhCN);

        let en_only: Vec<_> = en.keys().filter(|k| !zh.contains_key(*k)).collect();
        let zh_only: Vec<_> = zh.keys().filter(|k| !en.contains_key(*k)).collect();

        assert!(
            en_only.is_empty() && zh_only.is_empty(),
            "两份词典的 key 集合不一致 —— en 独有: {en_only:?}, zh 独有: {zh_only:?}"
        );
    }

    /// 词典里的值不能是「字面量 \n」。
    ///
    /// 灌词典时用 `json.dumps` 会把真换行编码成 `\n` 两字符，而解析出来
    /// 仍是两字符 —— 屏幕上就显示成 `# Provider Doctor\n\n` 而不是两个换行。
    /// 这个坑踩过一次，所以钉死。
    #[test]
    fn dictionary_values_use_real_newlines_not_escaped_sequences() {
        for lang in [UiLanguage::En, UiLanguage::ZhCN] {
            let dict = embedded_dict(lang);
            let bad: Vec<_> = dict
                .iter()
                .filter(|(_, v)| v.contains("\\n"))
                .map(|(k, _)| k.clone())
                .collect();
            assert!(
                bad.is_empty(),
                "{:?} 词典里这些条目的换行是字面量 `\\n` 而非真换行: {bad:?}",
                lang
            );
        }
    }

    fn embedded_dict(lang: UiLanguage) -> HashMap<String, String> {
        let mut dict = HashMap::new();
        if let Some(content) = crate::core::i18n_dict::embedded_dictionary(lang.as_code()) {
            merge_dictionary_content(&mut dict, &content);
        }
        dict
    }

    /// `src/` 下全部 .rs 源文件。
    ///
    /// 走文件系统而不是 `include_str!` 清单：清单要人维护，漏一个文件
    /// 就等于那个文件里的 key 逃过检查 —— 而这正是这个门要防的事。
    fn source_files() -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        collect_rs_under(std::path::Path::new("src"), &mut out);
        out
    }

    fn collect_rs_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_under(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    /// 源码里引用到的所有词典 key —— **宽集**，孤儿门的判据。
    ///
    /// 判据是「源码里出现 `"<key>"` 字面量」，而不是「解析 `i18n::t1(...)`
    /// 的调用点」。两个原因：
    ///
    /// 1. **动态来源也算引用**。`i18n::t1(d.key)`（按键注册表）、
    ///    `i18n::t1(key)`（循环里的档位表）、三元分支 —— 这些地方的 key
    ///    是变量，调用点处没有字面量引用，但 key 确实写在源码里的表上。
    ///    按字面量扫就认得出来，按调用点扫就漏。
    /// 2. **不脆**。手写一个 Rust 词法分析器去配括号，遇上注释、字符串
    ///    里的括号就会错位 —— 之前那版就把自己文件里的 `i18n::t1("` 误当成
    ///    调用点，凭空造出 `) {` 这种垃圾 key。
    ///
    /// 代价：`user.name`、`api.anthropic.com` 这种普通字符串也算「引用」。
    /// 对孤儿门这是**正确方向的宽松** —— 宁可放过一个真孤儿，不可把还活着
    /// 的 key 误删。反过来拿它做「缺失」判据就全错了（那边方向是从严），
    /// 所以缺失门用的是窄集 [`called_keys`]。
    fn referenced_keys() -> BTreeSet<String> {
        let mut referenced = BTreeSet::new();
        for path in source_files() {
            // 本文件里有 `i18n::t1("` 的字面量（就是这个门自己写的），
            // 扫自己会把那些行当成引用。
            if path.ends_with("core/i18n.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for key in quoted_literals(&text) {
                referenced.insert(key);
            }
        }
        referenced
    }

    /// 真正流进 `t1` 实参的 key —— **窄集**，缺失门的判据。
    ///
    /// 单遍词法扫描 [`T1Scanner`]：扫到 `t1(` 就配平实参括号，收集其中的
    /// 字符串字面量 —— 直接传 key 的覆盖；三元分支（`t1(if x { "a" } else
    /// { "b" })`）的两个分支也在实参括号内，一并覆盖。
    ///
    /// 实参是**变量**的调用点（`t1(d.key)`、`t1(key)`）解析不出 key 本身，
    /// 由扫描器计数，[`called_keys`] 断言总数与 [`DYNAMIC_KEY_SOURCES`]
    /// 一致 —— 新增 `t1(var)` 而不登记，测试直接红，堵住静默漏检。
    fn called_keys() -> BTreeSet<String> {
        let mut called = BTreeSet::new();
        for path in source_files() {
            // 本文件里的 `i18n::t1("` 全在 doc 注释和测试里，跳过
            if path.ends_with("core/i18n.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let registered = DYNAMIC_KEY_SOURCES.iter().any(|p| path.ends_with(p));
            let mut scanner = T1Scanner::new(&text);
            scanner.run();
            called.extend(scanner.literal_keys);
            // 有动态调用点（`t1(var)`）就必须已登记 —— 那种调用点窄集
            // 解析不出 key，登记文件整体按宽集规则兜住。
            assert!(
                scanner.dynamic_sites == 0 || registered,
                "发现未登记的动态 t1 调用点（实参是变量，窄集扫不出 key）：\
                 把它所在的文件加进 DYNAMIC_KEY_SOURCES，并核对其中点分小写\
                 字面量全是词典 key"
            );
            if registered {
                for key in quoted_literals(&text) {
                    called.insert(key);
                }
            }
        }
        called
    }

    /// 实参是变量的 `t1` 调用点所在的文件 —— 窄集的登记表。
    ///
    /// 规则：任何含 `t1(var)` 调用点的文件**必须**出现在此（`called_keys`
    /// 里断言）；反过来本表可以多列没有调用点的文件 —— `keymap.rs` 就是
    /// 这种：`BindingDoc` 表持有 key，真正的调用在 `help_popup.rs` 的
    /// `t1(d.key)`。
    ///
    /// 登记文件整体按宽集规则收集，所以每列一个都有代价（点分小写普通
    /// 字符串混进窄集）—— 只列必要的：
    /// - `keymap.rs` —— `BindingDoc` 表（key 真身所在）
    /// - `items.rs` —— 思考档位表的循环变量 `t1(key)`
    /// - `help_popup.rs` —— 调用点 `t1(d.key)`
    /// - `palette_action.rs` 的 `t1(if … { "a" } else { "b" })` —— 词法上
    ///   窄集本可解析（字面量在实参括号内），整文件纳入是保险；代价是该
    ///   文件多收几个普通字符串
    const DYNAMIC_KEY_SOURCES: &[&str] = &[
        "ui/events/keymap.rs",
        "ui/components/palette/items.rs",
        "ui/components/help_popup.rs",
        "ui/events/input/palette_action.rs",
    ];

    /// `t1(...)` 调用点的单遍词法扫描器。
    ///
    /// 为什么不用「剥注释 → `match_indices("t1(")` → 手算偏移」：那一版
    /// 两个 bug 叠着 —— `t1(` 是 3 个字符，左括号在 `idx+2`，手算成 `idx+3`
    /// 之后**每个**调用点都配平失败；而剥注释改变了字节偏移，二次扫描的
    /// 索引又对不回原文。单遍扫描把「哪里是注释 / 字符串 / 字符字面量」和
    /// 「找 `t1(` 调用点」放在同一个循环里，索引天然一致。
    struct T1Scanner<'a> {
        bytes: &'a [u8],
        text: &'a str,
        i: usize,
        /// 实参以字符串字面量开头（或三元分支里含字面量）的调用点：
        /// 实参括号内抽到的 key。
        literal_keys: Vec<String>,
        /// 实参**不**以 `"` 开头的调用点个数（`t1(d.key)`、`t1(key)`）。
        dynamic_sites: usize,
    }

    impl<'a> T1Scanner<'a> {
        fn new(text: &'a str) -> Self {
            Self {
                bytes: text.as_bytes(),
                text,
                i: 0,
                literal_keys: Vec::new(),
                dynamic_sites: 0,
            }
        }

        fn run(&mut self) {
            while self.i < self.bytes.len() {
                match self.bytes[self.i] {
                    b'/' if self.peek(1) == Some(b'/') => self.skip_line_comment(),
                    b'/' if self.peek(1) == Some(b'*') => self.skip_block_comment(),
                    b'"' => {
                        self.skip_string();
                    }
                    b'\'' => match char_literal_end(self.bytes, self.i) {
                        Some(end) => self.i = end,
                        None => self.i += 1,
                    },
                    b't' if self.at_t1_call() => self.consume_t1_call(),
                    _ => self.i += 1,
                }
            }
        }

        fn peek(&self, off: usize) -> Option<u8> {
            self.bytes.get(self.i + off).copied()
        }

        /// 判断当前位置是否是一个真正的 `t1(` 调用：本字节是 `t`，接下来
        /// 恰好是 `1(`，且**前面不是标识符字符**（排除 `fmt1(`、`x_t1(` 这类
        /// 同形前缀）。`1` 后面必须紧跟 `(` —— `t1` 作为标识符的一部分
        /// （`str1`）由前面那个「不是标识符字符」守住。
        fn at_t1_call(&self) -> bool {
            if self.peek(1) != Some(b'1') || self.peek(2) != Some(b'(') {
                return false;
            }
            let prev_ok = self.i == 0 || {
                let p = self.bytes[self.i - 1];
                !(p.is_ascii_alphanumeric() || p == b'_')
            };
            prev_ok
        }

        fn skip_line_comment(&mut self) {
            while self.i < self.bytes.len() && self.bytes[self.i] != b'\n' {
                self.i += 1;
            }
        }

        fn skip_block_comment(&mut self) {
            self.i += 2;
            while self.i + 1 < self.bytes.len()
                && !(self.bytes[self.i] == b'*' && self.bytes[self.i + 1] == b'/')
            {
                self.i += 1;
            }
            self.i = (self.i + 2).min(self.bytes.len());
        }

        fn skip_string(&mut self) {
            self.i += 1;
            while self.i < self.bytes.len() && self.bytes[self.i] != b'"' {
                if self.bytes[self.i] == b'\\' {
                    self.i += 1;
                }
                self.i += 1;
            }
            self.i = (self.i + 1).min(self.bytes.len());
        }

        /// 消费一个 `t1(...)` 调用点：配平实参括号，按实参首字符分流。
        ///
        /// 括号配不平（宏、生成代码等怪异输入）就当普通字符跳过 —— 门要
        /// 防的是漏 key，不是当语法分析器。
        fn consume_t1_call(&mut self) {
            let open = self.i + 2; // 指向 '('
            let mut depth = 1i32;
            let mut j = open + 1;
            let mut closed = false;
            while j < self.bytes.len() {
                match self.bytes[j] {
                    b'/' if j + 1 < self.bytes.len() && self.bytes[j + 1] == b'/' => {
                        while j < self.bytes.len() && self.bytes[j] != b'\n' {
                            j += 1;
                        }
                    }
                    b'/' if j + 1 < self.bytes.len() && self.bytes[j + 1] == b'*' => {
                        j += 2;
                        while j + 1 < self.bytes.len()
                            && !(self.bytes[j] == b'*' && self.bytes[j + 1] == b'/')
                        {
                            j += 1;
                        }
                        j = (j + 2).min(self.bytes.len());
                    }
                    b'"' => {
                        j += 1;
                        while j < self.bytes.len() && self.bytes[j] != b'"' {
                            if self.bytes[j] == b'\\' {
                                j += 1;
                            }
                            j += 1;
                        }
                        j = (j + 1).min(self.bytes.len());
                    }
                    b'\'' => match char_literal_end(self.bytes, j) {
                        Some(end) => j = end,
                        None => j += 1,
                    },
                    b'(' => {
                        depth += 1;
                        j += 1;
                    }
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            closed = true;
                            break;
                        }
                        j += 1;
                    }
                    _ => j += 1,
                }
            }
            self.i = if closed { j + 1 } else { open + 1 };

            let arg = if closed {
                self.text[open + 1..j].trim().to_string()
            } else {
                String::new()
            };
            if arg.starts_with('"') {
                for key in quoted_literals(&arg) {
                    self.literal_keys.push(key);
                }
            } else if !arg.is_empty() {
                self.dynamic_sites += 1;
            }
        }
    }

    /// 抽出文本里所有**字符串字面量**的内容（跳过注释）。
    ///
    /// 必须跳过注释：源码注释里常出现 `"No model selected"` 这种带引号的
    /// 说明文字，按朴素的「找下一个 `"`」配对，注释里的引号会把后续所有
    /// 字符串的边界搞乱 —— 真实的 key 就这么漏掉了。
    ///
    /// 只取看起来像 i18n key 的（含点、且各段都是标识符字符）——
    /// 否则会把源码里所有字符串都收进来，集合大到没法用。
    fn quoted_literals(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let bytes = text.as_bytes();
        let mut i = 0usize;

        while i < bytes.len() {
            match bytes[i] {
                // 行注释：跳过到行尾
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                // 块注释：跳过到 */
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                    i += 2;
                    while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                        i += 1;
                    }
                    i = (i + 2).min(bytes.len());
                }
                // 字符字面量：'x' —— 跳过，避免里面的 '"' 干扰。
                // 但 Rust 的**生命周期**（`&'static str`）长得像字符字面量，
                // 必须区分：字符字面量的闭合引号在几个字符之内，生命周期
                // 没有（`'static str` 会一路吞到下一个 `'`，
                // 把中间所有字符串都吞掉）。
                b'\'' => {
                    if let Some(end) = char_literal_end(bytes, i) {
                        i = end;
                    } else {
                        // 生命周期：跳��引号本身
                        i += 1;
                    }
                }
                // 字符串字面量
                b'"' => {
                    i += 1;
                    let start = i;
                    while i < bytes.len() && bytes[i] != b'"' {
                        if bytes[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    let candidate = &text[start..i.min(text.len())];
                    if looks_like_i18n_key(candidate) {
                        out.push(candidate.to_string());
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        out
    }

    /// 判断 `'` 开头的是字符字面量还是生命周期。
    ///
    /// 字符字面量：`'a'` / `'\n'` / `'\''` —— 闭合引号在几个字符内。
    /// 生命周期：`'static` / `'a` —— 没有闭合引号。
    ///
    /// 判据：跳过转义后，在 8 个字符内遇到 `'` 就是字符字面量。
    /// Rust 的字符字面量最长也就 4 个字符（`'\u{1F600}'` 会长一点，
    /// 但那种写法不会出现在这个仓库里）。
    fn char_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
        let mut i = start + 1;
        while i < bytes.len() && i - start <= 8 {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'\'' {
                return Some(i + 1);
            }
            // 字符字面量的本体：字母数字、下划线，或**标点**
            // （`'"'` / `'{'` / `'!'` 都合法）。标点里要排除字母 ——
            // 生命周期（`'static`）由字母组成。
            let c = bytes[i];
            let is_punct = !c.is_ascii_alphanumeric() && c != b'_' && c != b' ';
            if is_punct && c != b'"' && c != b'{' && c != b'}' {
                return None;
            }
            if c.is_ascii_alphanumeric() || c == b'_' {
                i += 1;
                continue;
            }
            // 是标点：只有当下一个字符就是闭合引号时才当成字符字面量
            if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                return Some(i + 2);
            }
            return None;
        }
        None
    }

    /// 像不像一个 i18n key：点分段，每段是 `[a-z0-9_]+`，且**至少两段**。
    ///
    /// 至少两段这条是必须的：单段（`worker`）会误伤一堆普通字符串，
    /// 而带扩展名的（`worker.rs`）会因为 `rs` 全是小写字母而被当成 key
    /// —— 源码里出现的文件名、路径、配置键都会混进来。
    fn looks_like_i18n_key(text: &str) -> bool {
        let parts: Vec<_> = text.split('.').collect();
        if parts.len() < 2 {
            return false;
        }
        // 末段不能是常见文件扩展名
        const FILE_EXTS: &[&str] = &[
            "rs", "md", "json", "toml", "txt", "yml", "yaml", "lock", "py", "sh", "js", "ts",
            "html", "css", "png", "jpg", "svg",
        ];
        if FILE_EXTS.contains(&parts[parts.len() - 1]) {
            return false;
        }
        parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
    }
}
