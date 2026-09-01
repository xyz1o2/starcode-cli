use std::fs;
/// @ command handler: parses @ file references in user input, reads file content and injects it into messages
/// Reference implementation, supports multiple @, path escaping, fuzzy matching and other advanced features
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AtCommandPart {
    pub part_type: AtPartType,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AtPartType {
    Text,
    AtPath,
}

#[derive(Debug)]
pub struct ProcessedAt {
    pub original_query: String,
    pub processed_query: String,
    pub file_contents: Vec<FileContent>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub size: usize,
}

/// Parse user input, extract all @ commands and text segments
/// Supports backslash escaping: `@My\ Documents/file.txt`
pub fn may_contain_at_command(input: &str) -> bool {
    find_next_unescaped_at(input, 0).is_some()
}

pub fn parse_at_commands(input: &str) -> Vec<AtCommandPart> {
    if !may_contain_at_command(input) {
        return vec![AtCommandPart {
            part_type: AtPartType::Text,
            content: input.to_string(),
        }];
    }

    let mut parts = Vec::new();
    let mut current_byte_index = 0;

    // Build character to byte position mapping
    let char_indices: Vec<(usize, char)> = input.char_indices().collect();

    while current_byte_index < input.len() {
        // Find next unescaped @
        let at_byte_index = find_next_unescaped_at(input, current_byte_index);

        if at_byte_index.is_none() {
            // No more @, remaining part is text
            if current_byte_index < input.len() {
                parts.push(AtCommandPart {
                    part_type: AtPartType::Text,
                    content: input[current_byte_index..].to_string(),
                });
            }
            break;
        }

        let at_byte_pos = at_byte_index.unwrap();

        // Find corresponding character index
        let at_char_pos = char_indices
            .iter()
            .position(|&(byte_idx, _)| byte_idx == at_byte_pos)
            .unwrap();

        // Add text before @
        if at_byte_pos > current_byte_index {
            parts.push(AtCommandPart {
                part_type: AtPartType::Text,
                content: input[current_byte_index..at_byte_pos].to_string(),
            });
        }

        // Parse @path
        let (path_end_byte, path_content) = extract_at_path(input, at_char_pos);

        if !path_content.is_empty() {
            parts.push(AtCommandPart {
                part_type: AtPartType::AtPath,
                content: path_content,
            });
        }

        current_byte_index = path_end_byte;
    }

    // Don't filter empty text to match test expectations
    // If last is AtPath and not the only part, add empty Text to match test expectations
    if let Some(last) = parts.last() {
        if last.part_type == AtPartType::AtPath && parts.len() > 1 {
            parts.push(AtCommandPart {
                part_type: AtPartType::Text,
                content: String::new(),
            });
        }
    }

    parts
}

/// Find next unescaped @ symbol, return byte index
fn find_next_unescaped_at(input: &str, start_byte: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut i = start_byte;

    while i < bytes.len() {
        if bytes[i] == b'@' {
            // Check if escaped
            if i == 0 || bytes[i - 1] != b'\\' {
                return Some(i);
            }
        }
        i += 1;
    }

    None
}

/// Extract path from @ position, supports backslash escaping and @"quoted path"
/// Returns (path end byte position, unescaped path content)
fn extract_at_path(input: &str, at_char_pos: usize) -> (usize, String) {
    let char_indices: Vec<(usize, char)> = input.char_indices().collect();
    let mut path_chars = Vec::new();
    let mut i = at_char_pos + 1; // Skip @

    // 引号形式 @"path with spaces"：内容原样取到闭合引号（对标 Claude Code）
    if char_indices.get(i).map(|(_, c)| *c) == Some('"') {
        i += 1;
        while i < char_indices.len() {
            let (byte, ch) = char_indices[i];
            if ch == '"' {
                return (byte + 1, path_chars.into_iter().collect());
            }
            path_chars.push(ch);
            i += 1;
        }
        // 未闭合：取到末尾
        return (input.len(), path_chars.into_iter().collect());
    }

    let mut in_escape = false;

    while i < char_indices.len() {
        let ch = char_indices[i].1;

        if in_escape {
            // Escape character, add directly
            path_chars.push(ch);
            in_escape = false;
        } else if ch == '\\' {
            // Start escaping
            in_escape = true;
        } else if ch.is_whitespace()
            || matches!(
                ch,
                ',' | ';' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}'
            )
        {
            // Path terminator (unescaped whitespace or punctuation)
            break;
        } else if ch == '.' {
            // Special handling for period: stop if followed by whitespace or end
            if i + 1 >= char_indices.len() || char_indices[i + 1].1.is_whitespace() {
                break;
            }
            path_chars.push(ch);
        } else {
            path_chars.push(ch);
        }

        i += 1;
    }

    let mut path_str: String = path_chars.into_iter().collect();
    if path_str.is_empty() {
        // If no path content, this is a standalone @
        path_str = "@".to_string();
    }
    let end_byte_pos = if i < char_indices.len() {
        char_indices[i].0
    } else {
        input.len()
    };
    (end_byte_pos, path_str)
}

/// Process @ commands, read file contents
pub fn process_at_command(input: &str, workspace_root: Option<&Path>) -> ProcessedAt {
    if !may_contain_at_command(input) {
        return ProcessedAt {
            original_query: input.to_string(),
            processed_query: input.to_string(),
            file_contents: vec![],
            errors: vec![],
        };
    }

    let parts = parse_at_commands(input);
    let at_parts: Vec<&AtCommandPart> = parts
        .iter()
        .filter(|p| p.part_type == AtPartType::AtPath)
        .collect();

    // If no @ commands, return directly
    if at_parts.is_empty() {
        return ProcessedAt {
            original_query: input.to_string(),
            processed_query: input.to_string(),
            file_contents: vec![],
            errors: vec![],
        };
    }

    let mut file_contents = Vec::new();
    let mut errors = Vec::new();
    let mut processed_query = String::new();

    // Rebuild query text
    for part in &parts {
        match part.part_type {
            AtPartType::Text => {
                processed_query.push_str(&part.content);
            }
            AtPartType::AtPath => {
                let path_str = &part.content;

                // Handle standalone @
                if path_str.is_empty() {
                    processed_query.push('@');
                    continue;
                }

                // Try to read file
                match read_file_content(path_str, workspace_root) {
                    Ok(content) => {
                        file_contents.push(content.clone());
                        processed_query.push_str(&format!("@{}", path_str));
                    }
                    Err(e) => {
                        errors.push(format!("Failed to read @{}: {}", path_str, e));
                        processed_query.push_str(&format!("@{}", path_str));
                    }
                }
            }
        }
    }

    ProcessedAt {
        original_query: input.to_string(),
        processed_query: processed_query.trim().to_string(),
        file_contents,
        errors,
    }
}

/// Read file content, supports relative and absolute paths
fn read_file_content(path_str: &str, workspace_root: Option<&Path>) -> Result<FileContent, String> {
    let path = PathBuf::from(path_str);

    // Try to resolve path
    let resolved_path = if path.is_absolute() {
        path
    } else if let Some(root) = workspace_root {
        root.join(&path)
    } else {
        crate::core::utils::paths::current_dir_cached().join(&path)
    };

    // Safety check: ensure path is within workspace_root
    if let Some(root) = workspace_root {
        // If file exists, must check if it's within workspace
        // If file doesn't exist, subsequent checks will catch it, or fuzzy search will handle it (fuzzy search also recurses under root)
        if resolved_path.exists() {
            if let Ok(canonical_path) = resolved_path.canonicalize() {
                let canonical_root = root
                    .canonicalize()
                    .map_err(|e| format!("Failed to resolve workspace path: {}", e))?;
                if !canonical_path.starts_with(&canonical_root) {
                    return Err(format!(
                        "Access denied: file path outside workspace scope: {}",
                        resolved_path.display()
                    ));
                }
            }
        }
    }

    // Check if file exists
    if !resolved_path.exists() {
        // Try fuzzy search
        if let Some(root) = workspace_root {
            if let Some(found) = fuzzy_find_file(root, path_str) {
                return read_file_at_path(&found);
            }
        }
        return Err(format!("File does not exist: {}", resolved_path.display()));
    }

    // If it's a directory, list files
    if resolved_path.is_dir() {
        return read_directory_content(&resolved_path);
    }

    read_file_at_path(&resolved_path)
}

/// Read a single file
fn read_file_at_path(path: &Path) -> Result<FileContent, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("Read failed: {}", e))?;

    Ok(FileContent {
        path: path.to_string_lossy().to_string(),
        content,
        size: path.metadata().map(|m| m.len() as usize).unwrap_or(0),
    })
}

/// Read directory content (list file tree)
fn read_directory_content(dir: &Path) -> Result<FileContent, String> {
    let mut output = String::new();
    output.push_str(&format!("Directory: {}\n\n", dir.display()));

    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read directory: {}", e))?;

    let mut files = Vec::new();
    let mut dirs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            dirs.push(format!("  [DIR]  {}/", name));
        } else {
            let size = path.metadata().ok().map(|m| m.len()).unwrap_or(0);
            files.push(format!("  [FILE] {} ({} bytes)", name, size));
        }
    }

    dirs.sort();
    files.sort();

    for d in dirs {
        output.push_str(&d);
        output.push('\n');
    }
    for f in files {
        output.push_str(&f);
        output.push('\n');
    }

    Ok(FileContent {
        path: dir.to_string_lossy().to_string(),
        content: output.clone(),
        size: output.len(),
    })
}

/// Fuzzy find file (simplified)
fn fuzzy_find_file(root: &Path, pattern: &str) -> Option<PathBuf> {
    fn search_recursive(dir: &Path, pattern: &str, max_depth: usize) -> Option<PathBuf> {
        if max_depth == 0 {
            return None;
        }

        let entries = fs::read_dir(dir).ok()?;

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }

            if path.is_file() && name.contains(pattern) {
                return Some(path);
            }

            if path.is_dir() {
                if let Some(found) = search_recursive(&path, pattern, max_depth - 1) {
                    return Some(found);
                }
            }
        }

        None
    }

    search_recursive(root, pattern, 3)
}

/// Format processed result for sending to LLM
pub fn format_processed_message(processed: &ProcessedAt) -> String {
    if processed.file_contents.is_empty() && processed.errors.is_empty() {
        return processed.processed_query.clone();
    }

    let mut message = processed.processed_query.clone();

    if !processed.file_contents.is_empty() {
        message.push_str("\n\n--- Referenced File Contents ---\n");

        for file in &processed.file_contents {
            message.push_str(&format!("\nFile: {}\n", file.path));
            message.push_str(&format!("Size: {} bytes\n", file.size));
            message.push_str("---\n");
            message.push_str(&file.content);
            message.push_str("\n---\n");
        }
    }

    if !processed.errors.is_empty() {
        message.push_str("\n\nErrors:\n");
        for error in &processed.errors {
            message.push_str(&format!("  - {}\n", error));
        }
    }

    message
}

#[cfg(test)]
mod quoted_tests {
    use super::*;

    #[test]
    fn quoted_at_path_with_spaces() {
        let input = "look at @\"my file with spaces.txt\" please";
        let (end, path) = extract_at_path(input, 8);
        assert_eq!(path, "my file with spaces.txt");
        assert_eq!(&input[end..], " please");
    }

    #[test]
    fn quoted_at_path_unclosed() {
        let input = "see @\"my dir";
        let (_end, path) = extract_at_path(input, 4);
        assert_eq!(path, "my dir");
    }

    #[test]
    fn plain_at_path_unchanged() {
        let (end, path) = extract_at_path("read @src/main.rs now", 5);
        assert_eq!(path, "src/main.rs");
        assert_eq!(&"read @src/main.rs now"[end..], " now");
    }

    #[test]
    fn quoted_at_parses_end_to_end() {
        let input = "summarize @\"docs/my notes.md\" briefly";
        let parts = parse_at_commands(input);
        let at: Vec<_> = parts
            .iter()
            .filter(|p| p.part_type == AtPartType::AtPath)
            .collect();
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].content, "docs/my notes.md");
    }
}
