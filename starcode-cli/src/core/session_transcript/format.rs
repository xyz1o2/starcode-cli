/// 转录格式化

use super::{SessionTranscript, TranscriptEntry, EntryType};

/// 转录格式
pub enum TranscriptFormat {
    /// Markdown格式
    Markdown,
    /// JSON格式
    Json,
    /// 纯文本格式
    PlainText,
}

/// 转录格式化器
pub struct TranscriptFormatter;

impl TranscriptFormatter {
    /// 格式化转录
    pub fn format(transcript: &SessionTranscript, format: &TranscriptFormat) -> String {
        match format {
            TranscriptFormat::Markdown => Self::format_markdown(transcript),
            TranscriptFormat::Json => Self::format_json(transcript),
            TranscriptFormat::PlainText => Self::format_plain_text(transcript),
        }
    }

    /// Markdown格式
    fn format_markdown(transcript: &SessionTranscript) -> String {
        let mut output = String::new();

        output.push_str(&format!("# Session Transcript\n\n"));
        output.push_str(&format!("**Session ID:** {}\n", transcript.session_id));
        output.push_str(&format!("**Started:** {}\n", Self::format_timestamp(transcript.started_at)));
        
        if let Some(ended_at) = transcript.ended_at {
            output.push_str(&format!("**Ended:** {}\n", Self::format_timestamp(ended_at)));
        }

        output.push_str("\n---\n\n");

        for entry in &transcript.entries {
            let (prefix, icon) = match entry.entry_type {
                EntryType::UserMessage => ("User", "👤"),
                EntryType::AssistantResponse => ("Assistant", "🤖"),
                EntryType::ToolCall => ("Tool Call", "🔧"),
                EntryType::ToolResult => ("Tool Result", "📋"),
                EntryType::SystemMessage => ("System", "⚙️"),
                EntryType::Error => ("Error", "❌"),
            };

            output.push_str(&format!("### {} {}\n\n", icon, prefix));
            output.push_str(&format!("{}\n\n", entry.content));
        }

        output
    }

    /// JSON格式
    fn format_json(transcript: &SessionTranscript) -> String {
        serde_json::to_string_pretty(transcript).unwrap_or_default()
    }

    /// 纯文本格式
    fn format_plain_text(transcript: &SessionTranscript) -> String {
        let mut output = String::new();

        output.push_str(&format!("Session: {}\n", transcript.session_id));
        output.push_str(&format!("Started: {}\n", Self::format_timestamp(transcript.started_at)));
        output.push_str("\n");

        for entry in &transcript.entries {
            let prefix = match entry.entry_type {
                EntryType::UserMessage => "User",
                EntryType::AssistantResponse => "Assistant",
                EntryType::ToolCall => "Tool Call",
                EntryType::ToolResult => "Tool Result",
                EntryType::SystemMessage => "System",
                EntryType::Error => "Error",
            };

            output.push_str(&format!("{}: {}\n\n", prefix, entry.content));
        }

        output
    }

    /// 格式化时间戳
    fn format_timestamp(timestamp: i64) -> String {
        chrono::DateTime::from_timestamp(timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    }
}
