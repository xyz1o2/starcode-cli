use super::motions::Motion;

#[derive(Debug, Clone)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Paste,
    Undo,
    Redo,
    Repeat,
}

impl Operator {
    pub fn from_key(key: char) -> Option<Operator> {
        match key {
            'd' => Some(Operator::Delete),
            'c' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            'p' => Some(Operator::Paste),
            'u' => Some(Operator::Undo),
            'r' => Some(Operator::Redo),
            '.' => Some(Operator::Repeat),
            _ => None,
        }
    }

    pub fn apply(
        &self,
        content: &mut String,
        cursor: &mut usize,
        register: &mut String,
        motion: Option<&Motion>,
    ) {
        match self {
            Operator::Delete => {
                if let Some(motion) = motion {
                    let end = motion.apply(content, *cursor);
                    let start = (*cursor).min(end);
                    let end = (*cursor).max(end);
                    if start < end && end <= content.len() {
                        let deleted = content[start..end].to_string();
                        *register = deleted;
                        content.replace_range(start..end, "");
                        *cursor = start;
                    }
                }
            }
            Operator::Change => {
                if let Some(motion) = motion {
                    let end = motion.apply(content, *cursor);
                    let start = (*cursor).min(end);
                    let end = (*cursor).max(end);
                    if start < end && end <= content.len() {
                        let deleted = content[start..end].to_string();
                        *register = deleted;
                        content.replace_range(start..end, "");
                        *cursor = start;
                    }
                }
            }
            Operator::Yank => {
                if let Some(motion) = motion {
                    let end = motion.apply(content, *cursor);
                    let start = (*cursor).min(end);
                    let end = (*cursor).max(end);
                    if start < end && end <= content.len() {
                        *register = content[start..end].to_string();
                    }
                }
            }
            Operator::Paste => {
                if !register.is_empty() {
                    content.insert_str(*cursor, register);
                    *cursor += register.len();
                }
            }
            Operator::Undo => {
                // Undo functionality would need to be integrated with the editor's undo stack
                // For now, this is a placeholder
            }
            Operator::Redo => {
                // Redo functionality would need to be integrated with the editor's redo stack
                // For now, this is a placeholder
            }
            Operator::Repeat => {
                // Repeat last change would need to store the last operation
                // For now, this is a placeholder
            }
        }
    }
}