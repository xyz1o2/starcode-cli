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
                    output.push(' ');
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                        } else if !matches!(next, '\r') {
                            output.push(' ');
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

#[cfg(test)]
mod tests {
    use super::parse_json_with_comments;
    use serde_json::Value;

    #[test]
    fn block_comment_cannot_join_adjacent_json_tokens() {
        let result = parse_json_with_comments::<Value>(r#"{"value": 1/* comment */2}"#);

        assert!(
            result.is_err(),
            "comments must not join neighboring JSON tokens"
        );
    }

    #[test]
    fn comments_are_accepted_between_json_tokens_and_inside_strings_are_literal() {
        let value = parse_json_with_comments::<Value>(
            r#"{"left": 1, /* real comment */ "text": "/* literal */ // literal"}"#,
        )
        .expect("valid JSONC should parse");

        assert_eq!(value["left"], 1);
        assert_eq!(value["text"], "/* literal */ // literal");
    }
}
