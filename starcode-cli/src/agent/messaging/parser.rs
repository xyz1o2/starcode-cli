use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// Stream Input Parser
/// StarCode stream input parser.
/// It reads from an AsyncRead stream, parses lines as JSON, and validates them.
pub struct StreamInputParser {
    // In a real implementation, we might want to be generic over AsyncRead.
    // But for CLI, we typically read from stdin.
    // However, Rust's stdin is global. We'll simulate this by accepting a receiver of strings (lines).
    // Or we can spawn a task that reads stdin and sends lines to a channel.
}

#[derive(Debug, Deserialize)]
pub struct StreamMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub message: StreamMessageContent,
}

#[derive(Debug, Deserialize)]
pub struct StreamMessageContent {
    pub role: String,
    pub content: Value, // Can be string or array of blocks
}

impl StreamInputParser {
    /// Spawns a task to read from stdin line by line, parse as JSON, and send to the output channel.
    pub fn start_stdin_reader(tx: mpsc::UnboundedSender<StreamMessage>) {
        tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let reader = BufReader::new(stdin);
            let mut lines = reader.lines();
            let mut buffer = String::new();

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }

                let text_to_parse = if !buffer.is_empty() {
                    buffer.push_str(&line);
                    &buffer
                } else {
                    &line
                };

                // Try to parse as JSON
                match serde_json::from_str::<StreamMessage>(text_to_parse) {
                    Ok(msg) => {
                        // Clear buffer on success
                        buffer.clear();

                        // Validate type and role (as per g2A logic)
                        if msg.msg_type == "user" && msg.message.role == "user" {
                            if tx.send(msg).is_err() {
                                break; // Receiver dropped
                            }
                        } else {
                            eprintln!("⚠️ Received invalid message type or role: {:?}", msg);
                        }
                    }
                    Err(_e) => {
                        // If parsing fails, check if it might be an incomplete JSON
                        // A simple heuristic is checking for unbalanced braces, but here we just buffer it
                        // if we assume it's a split line.
                        // However, we must distinguish between "invalid JSON (user text)" and "incomplete JSON".
                        // Since `StreamMessage` is strict JSON, anything else is treated as raw text input usually.
                        // But if we want to support split chunks, we need a way to know.
                        // For now, if the buffer gets too large, we assume it's raw text and flush it.

                        if buffer.len() > 10 * 1024 {
                            // 10KB safety limit
                            // Flush buffer as raw text
                            let raw_msg = std::mem::take(&mut buffer);
                            let msg = StreamMessage {
                                msg_type: "user".to_string(),
                                message: StreamMessageContent {
                                    role: "user".to_string(),
                                    content: Value::String(raw_msg),
                                },
                            };
                            if tx.send(msg).is_err() {
                                break;
                            }
                        } else if buffer.is_empty() {
                            // If this was a fresh line and failed parsing,
                            // we check if it looks like the start of a JSON object '{'
                            // If so, we buffer it. If not, we treat as raw text immediately.
                            if line.trim_start().starts_with('{') {
                                buffer.push_str(&line);
                            } else {
                                // Treat as raw text
                                let msg = StreamMessage {
                                    msg_type: "user".to_string(),
                                    message: StreamMessageContent {
                                        role: "user".to_string(),
                                        content: Value::String(line),
                                    },
                                };
                                if tx.send(msg).is_err() {
                                    break;
                                }
                            }
                        }
                        // If buffer was not empty, we already appended. We just wait for next line.
                    }
                }
            }
        });
    }
}
