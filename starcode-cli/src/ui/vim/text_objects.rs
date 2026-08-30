#[derive(Debug, Clone)]
pub enum TextObject {
    InnerWord,
    AroundWord,
    InnerQuote(char),
    AroundQuote(char),
    InnerParen,
    AroundParen,
    InnerBrace,
    AroundBrace,
}

impl TextObject {
    pub fn from_key(key: &str) -> Option<TextObject> {
        match key {
            "iw" => Some(TextObject::InnerWord),
            "aw" => Some(TextObject::AroundWord),
            "i\"" => Some(TextObject::InnerQuote('"')),
            "a\"" => Some(TextObject::AroundQuote('"')),
            "i'" => Some(TextObject::InnerQuote('\'')),
            "a'" => Some(TextObject::AroundQuote('\'')),
            "i(" | "ib" => Some(TextObject::InnerParen),
            "a(" | "ab" => Some(TextObject::AroundParen),
            "i{" | "iB" => Some(TextObject::InnerBrace),
            "a{" | "aB" => Some(TextObject::AroundBrace),
            _ => None,
        }
    }

    pub fn get_range(&self, content: &str, cursor: usize) -> Option<(usize, usize)> {
        match self {
            TextObject::InnerWord => {
                let before = &content[..cursor];
                let after = &content[cursor..];
                
                let start = before.rfind(|c: char| !c.is_alphanumeric() && c != '_')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                
                let end = after.find(|c: char| !c.is_alphanumeric() && c != '_')
                    .map(|i| cursor + i)
                    .unwrap_or(content.len());
                
                Some((start, end))
            }
            TextObject::AroundWord => {
                let before = &content[..cursor];
                let after = &content[cursor..];
                
                let start = before.rfind(|c: char| !c.is_alphanumeric() && c != '_')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                
                let end = after.find(|c: char| !c.is_alphanumeric() && c != '_')
                    .map(|i| cursor + i)
                    .unwrap_or(content.len());
                
                // Include trailing whitespace for "around" word
                let end = content[end..].find(|c: char| !c.is_whitespace())
                    .map(|i| end + i)
                    .unwrap_or(content.len());
                
                Some((start, end))
            }
            TextObject::InnerQuote(quote_char) => {
                let before = &content[..cursor];
                let after = &content[cursor..];
                
                let start = before.rfind(*quote_char)
                    .map(|i| i + 1)
                    .unwrap_or(0);
                
                let end = after.find(*quote_char)
                    .map(|i| cursor + i)
                    .unwrap_or(content.len());
                
                Some((start, end))
            }
            TextObject::AroundQuote(quote_char) => {
                let before = &content[..cursor];
                let after = &content[cursor..];
                
                let start = before.rfind(*quote_char)
                    .unwrap_or(0);
                
                let end = after.find(*quote_char)
                    .map(|i| cursor + i + 1)
                    .unwrap_or(content.len());
                
                Some((start, end))
            }
            TextObject::InnerParen => {
                find_enclosing_delimiters(content, cursor, '(', ')', false)
            }
            TextObject::AroundParen => {
                find_enclosing_delimiters(content, cursor, '(', ')', true)
            }
            TextObject::InnerBrace => {
                find_enclosing_delimiters(content, cursor, '{', '}', false)
            }
            TextObject::AroundBrace => {
                find_enclosing_delimiters(content, cursor, '{', '}', true)
            }
        }
    }
}

fn find_enclosing_delimiters(
    content: &str,
    cursor: usize,
    open: char,
    close: char,
    include_delimiters: bool,
) -> Option<(usize, usize)> {
    let before = &content[..cursor];
    let after = &content[cursor..];
    
    let start = before.rfind(open)?;
    let end = after.find(close).map(|i| cursor + i)?;
    
    if include_delimiters {
        Some((start, end + 1))
    } else {
        Some((start + 1, end))
    }
}