use serde::de::DeserializeOwned;

pub fn strip_json_comments(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape = false;

    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escape {
                escape = false;
                continue;
            }
            match ch {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                output.push(ch);
            }
            '/' => match chars.peek() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    let mut prev = '\0';
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                        }
                        if prev == '*' && next == '/' {
                            break;
                        }
                        prev = next;
                    }
                }
                _ => output.push(ch),
            },
            _ => output.push(ch),
        }
    }

    output
}

pub fn parse_json_with_comments<T: DeserializeOwned>(
    content: &str,
) -> Result<T, serde_json::Error> {
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    let stripped = strip_json_comments(content);
    serde_json::from_str(&stripped)
}
 