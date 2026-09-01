/// LSP类型定义

use serde::{Deserialize, Serialize};

/// 位置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    /// 行号
    pub line: u32,
    /// 列号
    pub character: u32,
}

/// 范围
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Range {
    /// 开始位置
    pub start: Position,
    /// 结束位置
    pub end: Position,
}

/// 文本编辑
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEdit {
    /// 范围
    pub range: Range,
    /// 新文本
    pub new_text: String,
}

/// 代码补全项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    /// 标签
    pub label: String,
    /// 类型
    pub kind: Option<CompletionItemKind>,
    /// 详情
    pub detail: Option<String>,
    /// 文档
    pub documentation: Option<String>,
    /// 插入文本
    pub insert_text: Option<String>,
    /// 排序文本
    pub sort_text: Option<String>,
    /// 过滤文本
    pub filter_text: Option<String>,
}

/// 代码补全项类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompletionItemKind {
    Text,
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Interface,
    Module,
    Property,
    Unit,
    Value,
    Enum,
    Keyword,
    Snippet,
    Color,
    File,
    Reference,
    Folder,
    EnumMember,
    Constant,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

/// 悬停信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hover {
    /// 内容
    pub contents: HoverContents,
    /// 范围
    pub range: Option<Range>,
}

/// 悬停内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HoverContents {
    /// 纯文本
    PlainText(String),
    /// Markdown
    MarkedString(String),
    /// 多个标记字符串
    MarkedStrings(Vec<String>),
}

/// 位置信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    /// 文件URI
    pub uri: String,
    /// 范围
    pub range: Range,
}

/// 符号信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInformation {
    /// 名称
    pub name: String,
    /// 类型
    pub kind: SymbolKind,
    /// 位置
    pub location: Location,
    /// 容器名称
    pub container_name: Option<String>,
}

/// 符号类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SymbolKind {
    File,
    Module,
    Namespace,
    Package,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    Function,
    Variable,
    Constant,
    String,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
    EnumMember,
    Struct,
    Event,
    Operator,
    TypeParameter,
}
