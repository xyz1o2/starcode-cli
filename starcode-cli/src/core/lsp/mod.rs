/// LSP Server Manager
/// 
/// 对标claude-code-main的src/services/lsp/
/// 提供完整的LSP服务器管理功能

pub mod client;
pub mod config;
pub mod diagnostic;
pub mod instance;
pub mod manager;
pub mod types;

pub use client::LspClient;
pub use config::LspConfig;
pub use diagnostic::{DiagnosticRegistry, Diagnostic};
pub use instance::LspServerInstance;
pub use manager::LspServerManager;
pub use types::*;

use serde::{Deserialize, Serialize};

/// LSP语言
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum LspLanguage {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    PHP,
    Swift,
    Kotlin,
    Scala,
    Haskell,
    OCaml,
    Elixir,
    Erlang,
    Clojure,
    Lua,
    Shell,
    SQL,
    Markdown,
    JSON,
    YAML,
    TOML,
    HTML,
    CSS,
    Other(String),
}

impl LspLanguage {
    /// 从文件扩展名推断语言
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => LspLanguage::Rust,
            "ts" | "tsx" => LspLanguage::TypeScript,
            "js" | "jsx" | "mjs" => LspLanguage::JavaScript,
            "py" => LspLanguage::Python,
            "go" => LspLanguage::Go,
            "java" => LspLanguage::Java,
            "c" | "h" => LspLanguage::C,
            "cpp" | "cc" | "cxx" | "hpp" => LspLanguage::Cpp,
            "cs" => LspLanguage::CSharp,
            "rb" => LspLanguage::Ruby,
            "php" => LspLanguage::PHP,
            "swift" => LspLanguage::Swift,
            "kt" | "kts" => LspLanguage::Kotlin,
            "scala" | "sc" => LspLanguage::Scala,
            "hs" => LspLanguage::Haskell,
            "ml" | "mli" => LspLanguage::OCaml,
            "ex" | "exs" => LspLanguage::Elixir,
            "erl" | "hrl" => LspLanguage::Erlang,
            "clj" | "cljs" => LspLanguage::Clojure,
            "lua" => LspLanguage::Lua,
            "sh" | "bash" | "zsh" => LspLanguage::Shell,
            "sql" => LspLanguage::SQL,
            "md" | "markdown" => LspLanguage::Markdown,
            "json" => LspLanguage::JSON,
            "yaml" | "yml" => LspLanguage::YAML,
            "toml" => LspLanguage::TOML,
            "html" | "htm" => LspLanguage::HTML,
            "css" => LspLanguage::CSS,
            other => LspLanguage::Other(other.to_string()),
        }
    }

    /// 获取语言名称
    pub fn name(&self) -> &str {
        match self {
            LspLanguage::Rust => "rust",
            LspLanguage::TypeScript => "typescript",
            LspLanguage::JavaScript => "javascript",
            LspLanguage::Python => "python",
            LspLanguage::Go => "go",
            LspLanguage::Java => "java",
            LspLanguage::C => "c",
            LspLanguage::Cpp => "cpp",
            LspLanguage::CSharp => "csharp",
            LspLanguage::Ruby => "ruby",
            LspLanguage::PHP => "php",
            LspLanguage::Swift => "swift",
            LspLanguage::Kotlin => "kotlin",
            LspLanguage::Scala => "scala",
            LspLanguage::Haskell => "haskell",
            LspLanguage::OCaml => "ocaml",
            LspLanguage::Elixir => "elixir",
            LspLanguage::Erlang => "erlang",
            LspLanguage::Clojure => "clojure",
            LspLanguage::Lua => "lua",
            LspLanguage::Shell => "shell",
            LspLanguage::SQL => "sql",
            LspLanguage::Markdown => "markdown",
            LspLanguage::JSON => "json",
            LspLanguage::YAML => "yaml",
            LspLanguage::TOML => "toml",
            LspLanguage::HTML => "html",
            LspLanguage::CSS => "css",
            LspLanguage::Other(name) => name,
        }
    }
}
