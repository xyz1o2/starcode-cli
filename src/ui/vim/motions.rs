#[derive(Debug, Clone)]
pub enum Motion {
    Left,
    Down,
    Up,
    Right,
    WordForward,
    WordBackward,
    WordEnd,
    LineStart,
    LineEnd,
    FileStart,
    FileEnd,
    ParagraphPrev,
    ParagraphNext,
    MatchingBracket,
    FindCharForward(char),
    FindCharBackward(char),
    TillCharForward(char),
    TillCharBackward(char),
}

impl Motion {
    pub fn from_key(key: char, pending: &str) -> Option<Motion> {
        match key {
            'h' => Some(Motion::Left),
            'j' => Some(Motion::Down),
            'k' => Some(Motion::Up),
            'l' => Some(Motion::Right),
            'w' => Some(Motion::WordForward),
            'b' => Some(Motion::WordBackward),
            'e' => Some(Motion::WordEnd),
            '0' => Some(Motion::LineStart),
            '$' => Some(Motion::LineEnd),
            'g' => {
                if pending.ends_with('g') {
                    Some(Motion::FileStart)
                } else {
                    None
                }
            }
            'G' => Some(Motion::FileEnd),
            '{' => Some(Motion::ParagraphPrev),
            '}' => Some(Motion::ParagraphNext),
            '%' => Some(Motion::MatchingBracket),
            'f' => {
                if pending.len() >= 2 {
                    let ch = pending.chars().last()?;
                    Some(Motion::FindCharForward(ch))
                } else {
                    None
                }
            }
            'F' => {
                if pending.len() >= 2 {
                    let ch = pending.chars().last()?;
                    Some(Motion::FindCharBackward(ch))
                } else {
                    None
                }
            }
            't' => {
                if pending.len() >= 2 {
                    let ch = pending.chars().last()?;
                    Some(Motion::TillCharForward(ch))
                } else {
                    None
                }
            }
            'T' => {
                if pending.len() >= 2 {
                    let ch = pending.chars().last()?;
                    Some(Motion::TillCharBackward(ch))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn apply(&self, content: &str, cursor: usize) -> usize {
        match self {
            Motion::Left => {
                if cursor > 0 {
                    cursor - 1
                } else {
                    cursor
                }
            }
            Motion::Right => {
                if cursor < content.len() {
                    cursor + 1
                } else {
                    cursor
                }
            }
            Motion::Down => {
                let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = cursor - line_start;
                let next_line_start = content[cursor..].find('\n').map(|i| cursor + i + 1);
                if let Some(next_start) = next_line_start {
                    let next_line_end = content[next_start..]
                        .find('\n')
                        .map(|i| next_start + i)
                        .unwrap_or(content.len());
                    let next_line_len = next_line_end - next_start;
                    next_start + col.min(next_line_len)
                } else {
                    cursor
                }
            }
            Motion::Up => {
                let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = cursor - line_start;
                if line_start > 0 {
                    let prev_line_end = line_start - 1;
                    let prev_line_start = content[..prev_line_end]
                        .rfind('\n')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    let prev_line_len = prev_line_end - prev_line_start;
                    prev_line_start + col.min(prev_line_len)
                } else {
                    cursor
                }
            }
            Motion::WordForward => {
                let remaining = &content[cursor..];
                let mut found = false;
                let mut new_pos = cursor;
                for (i, ch) in remaining.char_indices() {
                    if ch.is_whitespace() {
                        found = true;
                    } else if found {
                        new_pos = cursor + i;
                        break;
                    }
                }
                new_pos
            }
            Motion::WordBackward => {
                let before = &content[..cursor];
                let mut chars: Vec<(usize, char)> = before.char_indices().collect();
                chars.reverse();
                let mut found_non_whitespace = false;
                let mut new_pos = cursor;
                for (i, ch) in chars {
                    if !ch.is_whitespace() {
                        found_non_whitespace = true;
                    } else if found_non_whitespace {
                        new_pos = i + 1;
                        break;
                    }
                }
                new_pos
            }
            Motion::WordEnd => {
                let remaining = &content[cursor..];
                let mut found = false;
                let mut new_pos = cursor;
                for (i, ch) in remaining.char_indices() {
                    if !ch.is_whitespace() {
                        if found {
                            new_pos = cursor + i - 1;
                            break;
                        }
                    } else {
                        found = true;
                    }
                }
                new_pos
            }
            Motion::LineStart => content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0),
            Motion::LineEnd => content[cursor..]
                .find('\n')
                .map(|i| cursor + i - 1)
                .unwrap_or(content.len() - 1),
            Motion::FileStart => 0,
            Motion::FileEnd => content.len(),
            Motion::ParagraphPrev => {
                let before = &content[..cursor];
                let mut lines: Vec<&str> = before.lines().collect();
                lines.reverse();
                let mut empty_line_found = false;
                let mut pos = cursor;
                for line in lines {
                    if line.trim().is_empty() {
                        empty_line_found = true;
                    } else if empty_line_found {
                        pos = content[..cursor].rfind(line).unwrap_or(cursor);
                        break;
                    }
                }
                pos
            }
            Motion::ParagraphNext => {
                let after = &content[cursor..];
                let lines: Vec<&str> = after.lines().collect();
                let mut empty_line_found = false;
                let mut pos = cursor;
                for line in lines {
                    if line.trim().is_empty() {
                        empty_line_found = true;
                    } else if empty_line_found {
                        pos = content[cursor..]
                            .find(line)
                            .map(|i| cursor + i)
                            .unwrap_or(cursor);
                        break;
                    }
                }
                pos
            }
            Motion::MatchingBracket => {
                let ch = content[cursor..].chars().next();
                match ch {
                    Some('(') => find_matching_bracket(content, cursor, '(', ')'),
                    Some(')') => find_matching_bracket(content, cursor, ')', '('),
                    Some('[') => find_matching_bracket(content, cursor, '[', ']'),
                    Some(']') => find_matching_bracket(content, cursor, ']', '['),
                    Some('{') => find_matching_bracket(content, cursor, '{', '}'),
                    Some('}') => find_matching_bracket(content, cursor, '}', '{'),
                    _ => cursor,
                }
            }
            Motion::FindCharForward(target) => content[cursor..]
                .find(*target)
                .map(|i| cursor + i)
                .unwrap_or(cursor),
            Motion::FindCharBackward(target) => content[..cursor].rfind(*target).unwrap_or(cursor),
            Motion::TillCharForward(target) => content[cursor..]
                .find(*target)
                .map(|i| if i > 0 { cursor + i - 1 } else { cursor })
                .unwrap_or(cursor),
            Motion::TillCharBackward(target) => content[..cursor]
                .rfind(*target)
                .map(|i| i + 1)
                .unwrap_or(cursor),
        }
    }
}

fn find_matching_bracket(content: &str, pos: usize, open: char, close: char) -> usize {
    let chars: Vec<char> = content.chars().collect();
    if pos >= chars.len() {
        return pos;
    }

    let current = chars[pos];
    let (target, direction) = if current == open {
        (close, 1i32)
    } else {
        (open, -1i32)
    };

    let mut depth = 0;
    let mut i = pos as i32;

    while i >= 0 && (i as usize) < chars.len() {
        if chars[i as usize] == current {
            depth += 1;
        } else if chars[i as usize] == target {
            depth -= 1;
            if depth == 0 {
                return i as usize;
            }
        }
        i += direction;
    }

    pos
}
