/// Syntax highlighting and UI enhancement modules.
///
/// Provides:
/// - Syntax highlighting for code blocks
/// - Structured diff rendering
/// - Global search dialog
/// - Context visualization
/// - Session management
/// - History search
/// - Quick open
/// - Compression visualization
/// - Theme picker
/// - Usage statistics
/// - Export functionality
use std::collections::HashMap;
use std::sync::OnceLock;

pub mod compression;
pub mod context_viz;
pub mod diff;
pub mod export;
pub mod history;
pub mod quick_open;
pub mod search;
pub mod session;
pub mod stats;
pub mod theme_picker;

/// ANSI color codes for syntax highlighting (256-color palette)
mod colors {
    pub const KEYWORD: &str = "\x1b[38;5;141m";     // Purple
    pub const STRING: &str = "\x1b[38;5;214m";      // Orange
    pub const NUMBER: &str = "\x1b[38;5;141m";      // Purple
    pub const COMMENT: &str = "\x1b[38;5;245m";     // Gray
    pub const FUNCTION: &str = "\x1b[38;5;75m";     // Blue
    pub const TYPE: &str = "\x1b[38;5;79m";         // Cyan
    pub const VARIABLE: &str = "\x1b[38;5;253m";    // Light gray
    pub const OPERATOR: &str = "\x1b[38;5;255m";    // White
    pub const PUNCTUATION: &str = "\x1b[38;5;245m"; // Gray
    pub const CONSTANT: &str = "\x1b[38;5;141m";    // Purple
    pub const PROPERTY: &str = "\x1b[38;5;75m";     // Blue
    pub const TAG: &str = "\x1b[38;5;75m";          // Blue
    pub const ATTRIBUTE: &str = "\x1b[38;5;79m";    // Cyan
    pub const RESET: &str = "\x1b[0m";
}

/// Token type for syntax highlighting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Keyword,
    String,
    Number,
    Comment,
    Function,
    Type,
    Variable,
    Operator,
    Punctuation,
    Constant,
    Property,
    Tag,
    Attribute,
    Plain,
}

impl TokenKind {
    pub fn ansi_code(&self) -> &'static str {
        match self {
            TokenKind::Keyword => colors::KEYWORD,
            TokenKind::String => colors::STRING,
            TokenKind::Number => colors::NUMBER,
            TokenKind::Comment => colors::COMMENT,
            TokenKind::Function => colors::FUNCTION,
            TokenKind::Type => colors::TYPE,
            TokenKind::Variable => colors::VARIABLE,
            TokenKind::Operator => colors::OPERATOR,
            TokenKind::Punctuation => colors::PUNCTUATION,
            TokenKind::Constant => colors::CONSTANT,
            TokenKind::Property => colors::PROPERTY,
            TokenKind::Tag => colors::TAG,
            TokenKind::Attribute => colors::ATTRIBUTE,
            TokenKind::Plain => "",
        }
    }
}

/// Simple regex-based syntax highlighter (fast, no tree-sitter dependency)
pub struct SyntaxHighlighter {
    /// Language-specific keyword sets
    keywords: HashMap<&'static str, Vec<&'static str>>,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        let mut keywords: HashMap<&'static str, Vec<&'static str>> = HashMap::new();

        // Rust keywords
        keywords.insert("rust", vec![
            "as", "async", "await", "break", "const", "continue", "crate", "dyn",
            "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
            "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
            "self", "Self", "static", "struct", "super", "trait", "true", "type",
            "unsafe", "use", "where", "while", "yield",
        ]);

        // Python keywords
        keywords.insert("python", vec![
            "and", "as", "assert", "async", "await", "break", "class", "continue",
            "def", "del", "elif", "else", "except", "False", "finally", "for",
            "from", "global", "if", "import", "in", "is", "lambda", "None",
            "nonlocal", "not", "or", "pass", "raise", "return", "True", "try",
            "while", "with", "yield",
        ]);

        // JavaScript/TypeScript keywords
        keywords.insert("javascript", vec![
            "async", "await", "break", "case", "catch", "class", "const", "continue",
            "debugger", "default", "delete", "do", "else", "enum", "export", "extends",
            "false", "finally", "for", "function", "if", "import", "in", "instanceof",
            "let", "new", "null", "of", "return", "super", "switch", "this", "throw",
            "true", "try", "typeof", "undefined", "var", "void", "while", "with", "yield",
        ]);
        keywords.insert("typescript", keywords.get("javascript").unwrap().clone());

        // Go keywords
        keywords.insert("go", vec![
            "break", "case", "chan", "const", "continue", "default", "defer", "else",
            "fallthrough", "for", "func", "go", "goto", "if", "import", "interface",
            "map", "package", "range", "return", "select", "struct", "switch", "type", "var",
        ]);

        // Java keywords
        keywords.insert("java", vec![
            "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char",
            "class", "const", "continue", "default", "do", "double", "else", "enum",
            "extends", "final", "finally", "float", "for", "goto", "if", "implements",
            "import", "instanceof", "int", "interface", "long", "native", "new",
            "package", "private", "protected", "public", "return", "short", "static",
            "strictfp", "super", "switch", "synchronized", "this", "throw", "throws",
            "transient", "try", "void", "volatile", "while",
        ]);

        // C/C++ keywords
        keywords.insert("c", vec![
            "auto", "break", "case", "char", "const", "continue", "default", "do",
            "double", "else", "enum", "extern", "float", "for", "goto", "if", "inline",
            "int", "long", "register", "restrict", "return", "short", "signed", "sizeof",
            "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while",
        ]);
        keywords.insert("cpp", {
            let mut kw = keywords.get("c").unwrap().clone();
            kw.extend(vec![
                "alignas", "alignof", "and", "and_eq", "asm", "bitand", "bitor", "bool",
                "catch", "class", "compl", "concept", "const_cast", "consteval", "constexpr",
                "constinit", "co_await", "co_return", "co_yield", "decltype", "delete",
                "dynamic_cast", "explicit", "export", "false", "friend", "mutable", "namespace",
                "new", "noexcept", "not", "not_eq", "nullptr", "operator", "or", "or_eq",
            ]);
            kw
        });

        Self { keywords }
    }

    /// Detect language from file extension
    pub fn detect_language(&self, filename: &str) -> &'static str {
        let ext = filename.rsplit('.').next().unwrap_or("");
        match ext {
            "rs" => "rust",
            "py" | "pyw" => "python",
            "js" | "jsx" | "mjs" | "cjs" => "javascript",
            "ts" | "tsx" | "mts" | "cts" => "typescript",
            "go" => "go",
            "java" => "java",
            "c" | "h" => "c",
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
            "rb" => "ruby",
            "php" => "php",
            "swift" => "swift",
            "kt" | "kts" => "kotlin",
            "scala" => "scala",
            "sh" | "bash" | "zsh" => "shell",
            "sql" => "sql",
            "html" | "htm" => "html",
            "css" => "css",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "toml" => "toml",
            "xml" => "xml",
            "md" | "markdown" => "markdown",
            _ => "unknown",
        }
    }

    /// Highlight a single line of code
    pub fn highlight_line(&self, line: &str, language: &str) -> String {
        if language == "unknown" || language == "markdown" {
            return line.to_string();
        }

        let mut result = String::with_capacity(line.len() * 2);
        let mut chars = line.char_indices().peekable();
        let keywords = self.keywords.get(language);

        while let Some((i, ch)) = chars.next() {
            match ch {
                // Comments
                '/' if line.get(i + 1..).map(|s| s.starts_with('/')).unwrap_or(false) => {
                    result.push_str(TokenKind::Comment.ansi_code());
                    result.push_str(&line[i..]);
                    result.push_str(colors::RESET);
                    return result;
                }
                '#' if language == "python" || language == "ruby" || language == "shell" => {
                    result.push_str(TokenKind::Comment.ansi_code());
                    result.push_str(&line[i..]);
                    result.push_str(colors::RESET);
                    return result;
                }
                // Strings
                '"' | '\'' | '`' => {
                    let quote = ch;
                    result.push_str(TokenKind::String.ansi_code());
                    result.push(ch);
                    while let Some((_, c)) = chars.next() {
                        result.push(c);
                        if c == '\\' {
                            if let Some((_, escaped)) = chars.next() {
                                result.push(escaped);
                            }
                        } else if c == quote {
                            break;
                        }
                    }
                    result.push_str(colors::RESET);
                }
                // Numbers
                '0'..='9' => {
                    result.push_str(TokenKind::Number.ansi_code());
                    result.push(ch);
                    while let Some(&(_, next)) = chars.peek() {
                        if next.is_ascii_digit() || next == '.' || next == '_' || next == 'x' || next == 'b' {
                            result.push(next);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    result.push_str(colors::RESET);
                }
                // Identifiers and keywords
                'a'..='z' | 'A'..='Z' | '_' => {
                    let start = i;
                    let mut end = i + ch.len_utf8();
                    while let Some(&(_, next)) = chars.peek() {
                        if next.is_alphanumeric() || next == '_' {
                            end += next.len_utf8();
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let word = &line[start..end];
                    let kind = if keywords.map(|kw| kw.contains(&word)).unwrap_or(false) {
                        TokenKind::Keyword
                    } else if word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                        TokenKind::Type
                    } else {
                        TokenKind::Plain
                    };
                    result.push_str(kind.ansi_code());
                    result.push_str(word);
                    result.push_str(colors::RESET);
                }
                // Operators
                '+' | '-' | '*' | '/' | '=' | '!' | '<' | '>' | '&' | '|' | '^' | '%' | '~' => {
                    result.push_str(TokenKind::Operator.ansi_code());
                    result.push(ch);
                    result.push_str(colors::RESET);
                }
                // Punctuation
                '(' | ')' | '[' | ']' | '{' | '}' | ';' | ':' | ',' | '.' => {
                    result.push_str(TokenKind::Punctuation.ansi_code());
                    result.push(ch);
                    result.push_str(colors::RESET);
                }
                // Default
                _ => {
                    result.push(ch);
                }
            }
        }

        result
    }

    /// Highlight an entire code block
    pub fn highlight_block(&self, code: &str, language: &str) -> String {
        code.lines()
            .map(|line| self.highlight_line(line, language))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Global syntax highlighter instance
static HIGHLIGHTER: OnceLock<SyntaxHighlighter> = OnceLock::new();

pub fn get_highlighter() -> &'static SyntaxHighlighter {
    HIGHLIGHTER.get_or_init(SyntaxHighlighter::new)
}

/// Highlight code with syntax highlighting
pub fn highlight_code(code: &str, language: &str) -> String {
    get_highlighter().highlight_block(code, language)
}

/// Detect language from filename
pub fn detect_language(filename: &str) -> &'static str {
    get_highlighter().detect_language(filename)
}
