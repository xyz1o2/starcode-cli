use crate::types::StarMessage;

pub struct ContextCollapse {
    pub collapse_threshold: usize,
    pub keep_recent: usize,
}

impl ContextCollapse {
    pub fn new() -> Self {
        Self {
            collapse_threshold: 8000,
            keep_recent: 10,
        }
    }

    pub fn with_threshold(mut self, threshold: usize) -> Self {
        self.collapse_threshold = threshold;
        self
    }

    pub fn with_keep_recent(mut self, keep_recent: usize) -> Self {
        self.keep_recent = keep_recent;
        self
    }

    pub fn should_collapse(&self, messages: &[StarMessage], token_count: usize) -> bool {
        token_count > self.collapse_threshold && messages.len() > self.keep_recent * 2
    }

    pub fn collapse(&self, messages: &[StarMessage]) -> Vec<StarMessage> {
        if messages.len() <= self.keep_recent {
            return messages.to_vec();
        }

        let split_point = messages.len() - self.keep_recent;
        let old_messages = &messages[..split_point];
        let recent_messages = &messages[split_point..];

        let summary = self.summarize_messages(old_messages);

        let mut result = Vec::new();

        // Add summary
        result.push(StarMessage::system(&summary));

        // Preserve ALL user messages from old messages
        for msg in old_messages {
            if msg.role == "user" {
                result.push(msg.clone());
            }
        }

        result.extend(recent_messages.iter().cloned());

        result
    }

    fn summarize_messages(&self, messages: &[StarMessage]) -> String {
        let mut summary = String::from("[Previous conversation summary]\n");

        for msg in messages {
            match msg.role.as_str() {
                "user" => {
                    if let Some(content) = &msg.content {
                        let preview: String = content.chars().take(100).collect();
                        summary.push_str(&format!("User: {}...\n", preview));
                    }
                }
                "assistant" => {
                    if let Some(content) = &msg.content {
                        let preview: String = content.chars().take(100).collect();
                        summary.push_str(&format!("Assistant: {}...\n", preview));
                    }
                }
                "tool" => {
                    summary.push_str("Tool result: [executed]\n");
                }
                "system" => {
                    if let Some(content) = &msg.content {
                        let preview: String = content.chars().take(80).collect();
                        summary.push_str(&format!("System: {}...\n", preview));
                    }
                }
                _ => {}
            }
        }

        summary
    }

    pub fn collapse_with_preservation(
        &self,
        messages: &[StarMessage],
        preserve_system: bool,
    ) -> Vec<StarMessage> {
        if messages.len() <= self.keep_recent {
            return messages.to_vec();
        }

        let mut system_messages = Vec::new();
        let mut other_messages = Vec::new();

        if preserve_system {
            for msg in messages {
                if msg.role == "system" {
                    system_messages.push(msg.clone());
                } else {
                    other_messages.push(msg.clone());
                }
            }
        } else {
            other_messages = messages.to_vec();
        }

        if other_messages.len() <= self.keep_recent {
            let mut result = system_messages;
            result.extend(other_messages);
            return result;
        }

        let split_point = other_messages.len() - self.keep_recent;
        let old_messages = &other_messages[..split_point];
        let recent_messages = &other_messages[split_point..];

        let summary = self.summarize_messages(old_messages);

        let mut result = system_messages;

        result.push(StarMessage::system(&summary));

        // Preserve ALL user messages from old messages
        for msg in old_messages {
            if msg.role == "user" {
                result.push(msg.clone());
            }
        }

        result.extend(recent_messages.iter().cloned());

        result
    }
}

impl Default for ContextCollapse {
    fn default() -> Self {
        Self::new()
    }
}
 