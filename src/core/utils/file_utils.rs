use crate::core::tools::constants::ToolErrorType;
use encoding_rs::GBK;
use std::path::Path;

pub const DEFAULT_MAX_LINES_TEXT_FILE: usize = 600;

/// Strip prompt injection tags from file content.
/// These tags might be embedded in files to trick the AI into following malicious instructions.
pub fn strip_prompt_injection_tags(content: &str) -> String {
    let mut result = content.to_string();

    // Tags to strip (with their content)
    let dangerous_tags = [
        ("<system-reminder>", "</system-reminder>"),
        ("<system>", "</system>"),
        ("<instructions>", "</instructions>"),
        ("<prompt>", "</prompt>"),
    ];

    for (open, close) in &dangerous_tags {
        while let Some(start) = result.find(open) {
            if let Some(end) = result[start..].find(close) {
                let end_pos = start + end + close.len();
                // Replace the entire tag with a notice
                let notice = format!(
                    "[REMOVED: embedded {} tag]",
                    open.trim_matches('<').trim_matches('>')
                );
                result.replace_range(start..end_pos, &notice);
            } else {
                // No closing tag found, just remove the opening tag
                result.replace_range(start..start + open.len(), "[REMOVED: incomplete tag]");
                break;
            }
        }
    }

    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileType {
    Text,
    Image,
    Audio,
    Video,
    PDF,
    Binary,
    SVG,
    Notebook,
}

#[derive(Debug, Clone)]
pub struct ProcessedFileReadResult {
    pub llm_content: String,
    pub return_display: String,
    pub error: Option<String>,
    pub error_type: Option<ToolErrorType>,
    pub is_truncated: Option<bool>,
    pub original_line_count: Option<usize>,
    pub lines_shown: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy)]
pub enum UnicodeEncoding {
    UTF8,
    UTF16LE,
    UTF16BE,
    UTF32LE,
    UTF32BE,
}

pub struct BOM {
    pub bom_length: usize,
    pub encoding: UnicodeEncoding,
}

pub fn detect_bom(bytes: &[u8]) -> Option<BOM> {
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        return Some(BOM {
            bom_length: 3,
            encoding: UnicodeEncoding::UTF8,
        });
    }
    if bytes.len() >= 4
        && bytes[0] == 0x00
        && bytes[1] == 0x00
        && bytes[2] == 0xFE
        && bytes[3] == 0xFF
    {
        return Some(BOM {
            bom_length: 4,
            encoding: UnicodeEncoding::UTF32BE,
        });
    }
    if bytes.len() >= 4
        && bytes[0] == 0xFF
        && bytes[1] == 0xFE
        && bytes[2] == 0x00
        && bytes[3] == 0x00
    {
        return Some(BOM {
            bom_length: 4,
            encoding: UnicodeEncoding::UTF32LE,
        });
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return Some(BOM {
            bom_length: 2,
            encoding: UnicodeEncoding::UTF16BE,
        });
    }
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return Some(BOM {
            bom_length: 2,
            encoding: UnicodeEncoding::UTF16LE,
        });
    }
    None
}

pub fn decode_utf16be(bytes: &[u8]) -> String {
    let u16_vec: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect();
    String::from_utf16_lossy(&u16_vec)
}

pub fn decode_utf32(bytes: &[u8], little_endian: bool) -> String {
    let u32_vec: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|chunk| {
            if little_endian {
                u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
            } else {
                u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
            }
        })
        .collect();

    u32_vec
        .into_iter()
        .map(|u| char::from_u32(u).unwrap_or('\u{FFFD}'))
        .collect()
}

pub fn get_specific_mime_type(path: &Path) -> Option<String> {
    mime_guess::from_path(path).first().map(|m| m.to_string())
}

pub fn convert_notebook_to_markdown(content: &str) -> Result<String, String> {
    let json: serde_json::Value = serde_json::from_str(content).map_err(|e| e.to_string())?;
    let mut markdown = String::new();

    if let Some(cells) = json["cells"].as_array() {
        for cell in cells {
            let cell_type = cell["cell_type"].as_str().unwrap_or("");
            let source_val = &cell["source"];

            let source_lines: Vec<String> = if let Some(arr) = source_val.as_array() {
                arr.iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            } else if let Some(s) = source_val.as_str() {
                s.lines().map(|l| l.to_string()).collect()
            } else {
                Vec::new()
            };

            let source = source_lines.join("");

            if cell_type == "markdown" {
                markdown.push_str(&source);
                markdown.push_str("\n\n");
            } else if cell_type == "code" {
                markdown.push_str("```python\n");
                markdown.push_str(&source);
                markdown.push_str("\n```\n\n");
            }
        }
    }

    Ok(markdown)
}

/// 格式化文件大小
pub fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Decode raw bytes to String with BOM detection, UTF-8, and GBK fallback
fn decode_bytes_to_string(bytes: Vec<u8>) -> String {
    // Check for BOM
    if let Some(bom) = detect_bom(&bytes) {
        // Skip BOM
        let content_bytes = &bytes[bom.bom_length..];
        match bom.encoding {
            UnicodeEncoding::UTF8 => {
                return String::from_utf8(content_bytes.to_vec())
                    .unwrap_or_else(|_| String::from_utf8_lossy(content_bytes).into_owned());
            }
            UnicodeEncoding::UTF16LE => {
                let u16_vec: Vec<u16> = content_bytes
                    .chunks_exact(2)
                    .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();
                return String::from_utf16_lossy(&u16_vec);
            }
            UnicodeEncoding::UTF16BE => return decode_utf16be(content_bytes),
            UnicodeEncoding::UTF32LE => return decode_utf32(content_bytes, true),
            UnicodeEncoding::UTF32BE => return decode_utf32(content_bytes, false),
        }
    }

    // Try UTF-8
    if let Ok(s) = String::from_utf8(bytes.clone()) {
        return s;
    }

    // Try GBK (fallback for Windows CN)
    let (cow, _encoding_used, _had_errors) = GBK.decode(&bytes);
    cow.into_owned()
}

/// Read file with automatic encoding detection. Returns io::Result preserving NotFound.
pub fn read_file_with_encoding_io(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(decode_bytes_to_string(bytes))
}

/// Async version of read_file_with_encoding_io for use in async contexts.
pub async fn read_file_with_encoding_async(path: &Path) -> std::io::Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || read_file_with_encoding_io(&path))
        .await
        .map_err(|join_err| std::io::Error::new(std::io::ErrorKind::Other, join_err.to_string()))?
}

/// 读取文件并自动检测编码
pub fn read_file_with_encoding(path: &Path) -> Result<String, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(decode_bytes_to_string(bytes)),
        Err(e) => Err(e.to_string()),
    }
}

/// 阻塞式读取文件并处理（截断、分页等）
///
/// 策略对标 Claude Code:
/// - 默认尽可能多返回内容，以 token 限制为截断标准
/// - 超过限制时返回 PARTIAL view 通知，告知如何继续读取
/// - 用户显式指定 offset/limit 时直接使用，不再自动截断
pub fn process_file_read_blocking(
    path: &Path,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<ProcessedFileReadResult, String> {
    let content = read_file_with_encoding(path)?;

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let offset = offset.unwrap_or(0);

    // Token-based limit estimation (approximate: 1 token ≈ 4 chars for English, 2 chars for CJK)
    // Default: 100000 chars ≈ 25000-50000 tokens (increased to reduce truncation)
    let default_max_chars: usize = std::env::var("STAR_READ_FILE_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);

    // If user specifies a limit, use it directly (line-based)
    // If user doesn't specify, use dynamic char-based limit
    let (end_line, is_token_limited) = if let Some(user_limit) = limit {
        // User explicitly specified line limit — use it directly
        let end = (offset + user_limit).min(total_lines);
        (end, false)
    } else {
        // Dynamic: read as many lines as fit within char budget
        let mut char_count = 0usize;
        let mut end = offset;
        while end < total_lines {
            let line_chars = lines[end].len() + 1; // +1 for newline
            if char_count + line_chars > default_max_chars && end > offset {
                break;
            }
            char_count += line_chars;
            end += 1;
        }
        (end, end < total_lines)
    };

    if offset >= total_lines && total_lines > 0 {
        return Ok(ProcessedFileReadResult {
            llm_content: String::new(),
            return_display: String::new(),
            error: Some(format!(
                "Offset {} is beyond end of file ({} lines)",
                offset, total_lines
            )),
            error_type: Some(ToolErrorType::InvalidToolParams),
            is_truncated: Some(false),
            original_line_count: Some(total_lines),
            lines_shown: Some((0, 0)),
        });
    }

    // Extend past end_line if we'd cut inside an unbalanced brace block
    let end = if end_line < total_lines {
        let mut depth: i64 = 0;
        for line in &lines[offset..end_line] {
            for ch in line.chars() {
                match ch {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
            }
        }
        if depth > 0 {
            let mut extended = end_line;
            while extended < total_lines {
                for ch in lines[extended].chars() {
                    match ch {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                extended += 1;
                if depth <= 0 {
                    break;
                }
            }
            extended
        } else {
            end_line
        }
    } else {
        end_line
    };

    let selected_lines = &lines[offset..end];
    let output = selected_lines.join("\n");

    // Strip prompt injection tags from the output
    let output = strip_prompt_injection_tags(&output);

    let is_truncated = end < total_lines;

    // Add PARTIAL view notice (like Claude Code)
    let llm_content =
        if is_truncated {
            let remaining = total_lines - end;
            format!(
            "{}\n\n[PARTIAL VIEW] Showing lines {}-{} of {} total lines ({} lines remaining).\n\
             To read more, use: read_file(path=\"{}\", offset={}, limit={})",
            output,
            offset + 1, end, total_lines, remaining,
            path.display(), end, remaining.min(500)
        )
        } else {
            output.clone()
        };

    Ok(ProcessedFileReadResult {
        llm_content,
        return_display: output,
        error: None,
        error_type: None,
        is_truncated: Some(is_truncated),
        original_line_count: Some(total_lines),
        lines_shown: Some((offset + 1, end)),
    })
}

/// 检测文件编码（简单版，用于元数据报告）
pub fn detect_encoding_simple(bytes: &[u8]) -> String {
    // 简单检测：UTF-8 or Binary
    if std::str::from_utf8(bytes).is_ok() {
        "UTF-8".to_string()
    } else {
        "Binary".to_string()
    }
}

/// 检测是否为二进制文件
pub fn is_binary_file(bytes: &[u8]) -> bool {
    // 启发式：前 8KB 中如果有 NULL 字节，视为二进制
    let sample = &bytes[..bytes.len().min(8192)];
    sample.contains(&0)
}
