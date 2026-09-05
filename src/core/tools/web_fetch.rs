use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use crate::llm::client::StarClient;
use html2text;
use reqwest;
use serde::Deserialize;

#[derive(Clone)]
pub struct WebFetchTool {
    /// 用于 `prompt` 抽取的客户端。没有客户端时退化成"返回整页 markdown"。
    client: Option<StarClient>,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self { client: None }
    }

    /// 带 LLM 客户端构造 —— 有客户端才能支持 `prompt`（对标 Claude Code：整页交给一个
    /// 便宜模型，只把答案放进主上下文）
    pub fn with_client(client: StarClient) -> Self {
        Self {
            client: Some(client),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WebFetchParams {
    pub url: String,
    /// 想从页面里拿到什么。给了就只回答案，不回整页。
    #[serde(default)]
    pub prompt: Option<String>,
}

pub struct WebFetchInvocation {
    params: WebFetchParams,
    client: Option<StarClient>,
}

impl ToolInvocation for WebFetchInvocation {
    fn get_description(&self) -> String {
        match self.params.prompt.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => format!("Fetch {} and extract: {}", self.params.url, p),
            _ => format!("Fetch content from {}", self.params.url),
        }
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let url = self.params.url.clone();
        let prompt = self
            .params
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        let llm = self.client.clone();

        Box::pin(async move {
            // 离线模式：WebFetch 直接拒绝（对标 Claude Code /network）
            if crate::core::offline::is_offline() {
                crate::utils::logging::append_debug_log_line(
                    "[web_fetch] Refusing fetch (offline mode)",
                );
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: "Web fetch is unavailable because offline mode is ON.".to_string(),
                    error: Some(ToolError {
                        error_type: "offline".to_string(),
                        // 提示必须放在 message 里：error 非空时 output 会被执行器丢掉，
                        // 而 llm_content 全仓库没人读
                        message: "Web fetch refused: offline mode is ON. Do not retry — \
                                  it stays off until the user runs /network off."
                            .to_string(),
                    }),
                    data: None,
                });
            }

            // 1. Fetch HTML
            let client = reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
                .timeout(std::time::Duration::from_secs(30))
                .build()?;

            let response = client.get(&url).send().await?;

            if !response.status().is_success() {
                let status = response.status();
                // 注意：执行器在 `error.is_some()` 时会把 output 丢掉，只把 error.message
                // 交给模型（`StructuredError::format_display`）。所以有用的信息必须写进
                // message 里，写在 output 里等于没写。
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: format!("Fetch failed: {} returned HTTP {}", url, status),
                    error: Some(ToolError {
                        error_type: "http_error".to_string(),
                        message: format!("HTTP error: {} returned {}", url, status),
                    }),
                    data: None,
                });
            }

            let html_content = response.text().await?;

            // 2. Extract Content using Readability
            // 解析同步且会 panic 在畸形标签上，所以走 readability_safe 的
            // spawn_blocking + catch_unwind 外壳。
            let article =
                crate::core::tools::readability_safe::extract_article(html_content.clone()).await;

            if let Some(article) = article {
                // 3. Convert Extracted HTML to Markdown
                let title = article.title.unwrap_or_default();
                let byline = article.byline.unwrap_or_default();
                let excerpt = article.excerpt.unwrap_or_default();
                let content_html = article.content.unwrap_or_default();

                let markdown_body = html2text::from_read(content_html.as_bytes(), 80)
                    .unwrap_or_else(|_| content_html.clone());

                let final_output = render_article(
                    title.trim(),
                    byline.trim(),
                    excerpt.trim(),
                    &cap_fetched_text(markdown_body.trim()),
                );

                let final_output = extract_with_prompt(&url, final_output, prompt, llm).await;

                // 只存一份。执行器把结果折叠成 `return_display.unwrap_or(output)` 再同时喂给
                // 模型和 UI，`llm_content` 全仓库没人读 —— 三份克隆只是三倍内存。
                Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: final_output,
                    error: None,
                    data: None,
                })
            } else {
                // Fallback: If readability fails, just return raw text via html2text on the full body
                let text = html2text::from_read(html_content.as_bytes(), 80)
                    .unwrap_or_else(|e| format!("HTML parsing failed: {}", e));
                let body = format!(
                    "Readability could not identify a main article on this page; \
                     returning the raw page text instead.\n\n{}",
                    cap_fetched_text(text.trim())
                );
                Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: extract_with_prompt(&url, body, prompt, llm).await,
                    error: None,
                    data: None,
                })
            }
        })
    }
}

impl BaseDeclarativeTool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn display_name(&self) -> &str {
        "Web Fetch"
    }

    fn description(&self) -> &str {
        "Fetch and extract text content from a URL. Uses readability mode to extract main article content. Pass `prompt` to get just the answer instead of the whole page."
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch."
                },
                "prompt": {
                    "type": "string",
                    "description": "What to extract from the page. When set, the page is read by the model and only the answer comes back — far cheaper than pulling the whole page into context. Omit only when you need the full text."
                }
            },
            "required": ["url"]
        })
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: WebFetchParams = serde_json::from_value(params)?;
        Ok(Box::new(WebFetchInvocation {
            params,
            client: self.client.clone(),
        }))
    }
}

/// 抓取正文的字符上限。
///
/// 与 `ToolResultBudget::max_chars_per_result`（`agent/compact/tool_output_compact.rs`）
/// 对齐：再多给也会被那一层砍掉，只是白多一条截断标记。按字符切而不是按字节 —— 网页
/// 正文几乎不可能是纯 ASCII，`&s[..n]` 在这里等于随机 panic。
const MAX_FETCHED_CHARS: usize = 50_000;

/// 走 `prompt` 抽取时给主上下文留的答案上限（字符）
const MAX_EXTRACTED_CHARS: usize = 8_000;

/// 有 `prompt` 就把整页交给模型，只把答案带回主上下文。
///
/// 对标 Claude Code 的 WebFetch：页面本体只在这一次调用里出现，主上下文里留下的是几百
/// token 的答案，而不是几万 token 的整页 —— 而且整页不会随着后面每一轮对话被反复重发。
/// 没给 prompt、或者没有客户端、或者抽取失败时，原样返回页面内容。
async fn extract_with_prompt(
    url: &str,
    page: String,
    prompt: Option<String>,
    client: Option<StarClient>,
) -> String {
    let (Some(prompt), Some(client)) = (prompt, client) else {
        return page;
    };

    let request = format!(
        "You are reading a web page on behalf of a coding agent. Answer the request using \
         only what the page actually says.\n\n\
         URL: {}\n\nRequest: {}\n\n\
         Rules:\n\
         - Quote exact names, signatures, versions, commands and code verbatim.\n\
         - If the page does not contain the answer, say so in one line and list what it does \
         cover. Do not guess, and do not suggest fetching other pages.\n\
         - Be compact: no preamble, no restating the request.\n\n\
         --- PAGE ---\n{}\n--- END PAGE ---",
        url, prompt, page
    );

    match client.chat_completion_simple(&request).await {
        Ok(answer) if !answer.trim().is_empty() => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[web_fetch] Prompt extraction: {} page chars -> {} answer chars ({})",
                page.chars().count(),
                answer.chars().count(),
                url
            ));
            format!(
                "Extracted from {} for: {}\n\n{}",
                url,
                prompt,
                cap_extracted_text(answer.trim())
            )
        }
        Ok(_) | Err(_) => {
            // 抽取失败就退回整页 —— 宁可多花 token，也不能让模型以为"这页没东西"再抓一遍
            crate::utils::logging::append_debug_log_line(&format!(
                "[web_fetch] Prompt extraction failed, returning full page ({})",
                url
            ));
            page
        }
    }
}

/// 裁剪抽取结果
fn cap_extracted_text(text: &str) -> String {
    crate::utils::string_utils::truncate_chars(text, MAX_EXTRACTED_CHARS)
}

/// 按字符裁剪抓取到的正文，并说明剩下的部分不会再有。
///
/// 标记明确写"别重抓"。之前工具结果被裁掉后留的标记是"如需完整内容请重新读取/搜索"，
/// 模型照做，同一个页面于是被反复抓 —— 这就是"他会不停的找"的一半原因。
fn cap_fetched_text(text: &str) -> String {
    let total = text.chars().count();
    if total <= MAX_FETCHED_CHARS {
        return text.to_string();
    }

    let head: String = text.chars().take(MAX_FETCHED_CHARS).collect();
    format!(
        "{}\n\n[... truncated: showing the first {} of {} characters of this page. \
         The remainder is not available, and re-fetching the same URL will return the same \
         prefix — work with what is above ...]",
        head, MAX_FETCHED_CHARS, total
    )
}

/// 拼装最终 Markdown。空字段不占行 —— 每个空的 `**Author:**` 都是白花的 token。
fn render_article(title: &str, byline: &str, excerpt: &str, body: &str) -> String {
    let mut header = String::new();
    if !title.is_empty() {
        header.push_str(&format!("# {}\n", title));
    }
    if !byline.is_empty() {
        header.push_str(&format!("**Author:** {}\n", byline));
    }
    // 摘要通常就是正文首段的复述，正文已经覆盖时不必再留一份
    if !excerpt.is_empty() && !body_covers_excerpt(body, excerpt) {
        header.push_str(&format!("**Excerpt:** {}\n", excerpt));
    }

    if header.is_empty() {
        return body.to_string();
    }
    format!("{}\n---\n\n{}", header, body)
}

/// 判断正文是否已经包含摘要。
///
/// html2text 按 80 列折行，摘要里的空格在正文里可能已经变成换行，所以先把空白归一化；
/// 摘要末尾常带省略号，只比对开头一段。
fn body_covers_excerpt(body: &str, excerpt: &str) -> bool {
    fn squash(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    let excerpt = squash(excerpt);
    let probe: String = excerpt.chars().take(60).collect();
    let probe = probe.trim_end_matches(['…', '.', ' ']);
    if probe.is_empty() {
        return true;
    }
    squash(body).contains(probe)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CJK: &str = "上网找的知识点";

    #[test]
    fn text_under_the_cap_is_returned_verbatim() {
        let text = CJK.repeat(10);
        assert_eq!(cap_fetched_text(&text), text);
    }

    /// 抓中文页面时旧代码是 `&s[..n]` —— 正好落在三字节字符中间就 panic。
    #[test]
    fn cjk_text_over_the_cap_is_cut_on_char_boundaries() {
        let text = CJK.repeat(20_000);
        let capped = cap_fetched_text(&text);
        assert!(capped.starts_with(CJK));
        assert!(capped.contains("truncated"));
        // 标记里要报出原始字符数，而不是字节数
        assert!(capped.contains(&format!("{}", text.chars().count())));
    }

    /// 截断标记不能再让模型回头重抓 —— 那正是"不停的找"的来源。
    #[test]
    fn the_truncation_marker_discourages_refetching() {
        let capped = cap_fetched_text(&"a".repeat(MAX_FETCHED_CHARS + 10));
        assert!(capped.contains("re-fetching the same URL will return the same"));
        assert!(!capped.contains("重新读取"));
        assert!(!capped.contains("重新搜索"));
    }

    #[test]
    fn empty_metadata_produces_no_header_lines() {
        assert_eq!(render_article("", "", "", "body"), "body");
    }

    #[test]
    fn present_metadata_is_kept() {
        let out = render_article("Title", "Jack", "Summary", "body");
        assert!(out.starts_with("# Title\n"));
        assert!(out.contains("**Author:** Jack"));
        assert!(out.contains("**Excerpt:** Summary"));
        assert!(out.ends_with("body"));
    }

    #[test]
    fn an_excerpt_the_body_already_contains_is_dropped() {
        let body = "上网找的知识点很多，这里是正文的第一段。\n后面还有别的内容。";
        let out = render_article("标题", "", "上网找的知识点很多，这里是正文的第一段。", body);
        assert!(!out.contains("**Excerpt:**"), "{}", out);
        assert!(out.starts_with("# 标题\n"));
    }

    /// html2text 的折行不该骗过去重判断。
    #[test]
    fn line_wrapping_does_not_defeat_the_excerpt_check() {
        let out = render_article(
            "",
            "",
            "the quick brown fox jumps…",
            "the quick brown\nfox jumps over it",
        );
        assert_eq!(out, "the quick brown\nfox jumps over it");
    }
}
