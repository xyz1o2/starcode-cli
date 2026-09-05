use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult as CoreToolResult,
};
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use rand::seq::SliceRandom;
use reqwest::{Client, ClientBuilder};
use scraper::{Html, Selector};
use serde::Deserialize;
use std::time::Duration;
use tokio::time::sleep;

// Global client + UA pool
static CLIENT: Lazy<Client> = Lazy::new(|| {
    ClientBuilder::new()
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(10)
        .timeout(Duration::from_secs(25))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("Failed to build reqwest client")
});

static USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.3; rv:133.0) Gecko/20100101 Firefox/133.0",
];

/// 每条结果给模型的正文摘录长度（字符，不是字节）
const CONTENT_EXCERPT_CHARS: usize = 2000;

/// 搜索时顺带抓取正文的结果条数，默认 0。
///
/// 对标 Claude Code：WebSearch 只回标题 / URL / 摘要，正文由模型自己决定要不要用
/// WebFetch 去取。原来是固定抓前 3 条 —— 一次搜索就变成 3 次额外的整页请求（每次前面
/// 还有 0.5~1.8s 的反爬延迟），用户看到的就是"webfetch 这类一直在触发"，而这最多
/// 6000 字符的正文大多数时候模型压根不需要，纯烧 token。
///
/// 想恢复旧行为：`STAR_WEB_SEARCH_INLINE_PAGES=3`。
fn inline_content_pages() -> usize {
    std::env::var("STAR_WEB_SEARCH_INLINE_PAGES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .min(3)
}

fn random_ua() -> &'static str {
    USER_AGENTS
        .choose(&mut rand::thread_rng())
        .unwrap_or(&USER_AGENTS[0])
}

async fn random_delay(min_ms: u64, max_ms: u64) {
    let delay = rand::random::<u64>() % (max_ms - min_ms + 1) + min_ms;
    sleep(Duration::from_millis(delay)).await;
}

/// Simple URL encoding for query strings (percent-encoding)
fn encode_uri_component(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}

/// Simple URL decoding for percent-encoded strings
fn decode_uri_component(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '+' => result.push(' '),
            '%' => {
                let h1 = chars.next();
                let h2 = chars.next();
                if let (Some(h1), Some(h2)) = (h1, h2) {
                    let hex_str = format!("{}{}", h1, h2);
                    if let Ok(byte) = u8::from_str_radix(&hex_str, 16) {
                        result.push(byte as char);
                    } else {
                        result.push('%');
                        result.push(h1);
                        result.push(h2);
                    }
                } else {
                    result.push('%');
                    if let Some(h1) = h1 {
                        result.push(h1);
                    }
                }
            }
            _ => result.push(c),
        }
    }

    result
}

#[derive(Clone)]
pub struct WebSearchTool;

impl WebSearchTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
pub struct WebSearchParams {
    pub query: String,
    #[serde(default)]
    pub num: Option<u32>,
}

pub struct WebSearchInvocation {
    params: WebSearchParams,
}

#[derive(Debug, Clone)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Search using Brave Search HTML version (free, no API key required)
async fn search_brave(query: &str, num: u32) -> Result<Vec<SearchResult>> {
    crate::utils::logging::append_debug_log_line(&format!(
        "[web_search] Brave Search started: query='{}', num={}",
        query, num
    ));

    random_delay(600, 2200).await;

    let url = format!(
        "https://search.brave.com/search?q={}",
        encode_uri_component(query)
    );

    let resp = CLIENT
        .get(&url)
        .header("User-Agent", random_ua())
        .send()
        .await
        .context("Brave request failed")?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("Brave HTTP {}", resp.status()));
    }

    let html = resp.text().await.context("Failed to read Brave HTML")?;
    let doc = Html::parse_document(&html);

    // Multiple selector attempts (common structures 2025-2026)
    let selector_patterns = vec![
        // 2025-2026 common structure 1
        (
            r#"article[data-testid="result"]"#,
            r#"[data-testid="result-title"] a"#,
            r#"[data-testid="result-excerpt"]"#,
            r#"[data-testid="result-url"] a[href]"#,
        ),
        // Legacy / variant
        (
            ".snippet, .search-result, article",
            ".snippet-title a, h3 a, .title a",
            ".snippet-description, .snippet-content, .description",
            ".snippet-url a[href], .url a[href]",
        ),
        // Most permissive fallback
        (
            "article, .result, .search-snippet, li",
            "h3 a, .title a, a[href]",
            ".description, .snippet, .content, p",
            "a[href]",
        ),
    ];

    let mut results = vec![];

    for (cont_sel, title_sel, desc_sel, url_sel) in selector_patterns {
        let cont = Selector::parse(cont_sel).unwrap();
        let title_s = Selector::parse(title_sel).unwrap();
        let desc_s = Selector::parse(desc_sel).unwrap();
        let url_s = Selector::parse(url_sel).unwrap();

        for elem in doc.select(&cont).take(num as usize) {
            let title = elem
                .select(&title_s)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let url = elem
                .select(&url_s)
                .next()
                .and_then(|e| e.value().attr("href"))
                .map(str::to_string)
                .unwrap_or_default();

            // Filter out Brave internal links, keep only real external links
            let url = if url.starts_with("http://") || url.starts_with("https://") {
                url
            } else if url.starts_with("/search")
                || url.starts_with("/ask")
                || url.starts_with("/images")
                || url.starts_with("/news")
                || url.starts_with("/videos")
            {
                // Brave internal search page, skip
                continue;
            } else if url.starts_with("//") {
                // Protocol-relative URL
                format!("https:{}", url)
            } else {
                // Other relative paths, skip
                continue;
            };

            let snippet = elem
                .select(&desc_s)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            results.push(SearchResult {
                title,
                url,
                snippet,
            });

            if results.len() >= num as usize {
                return Ok(results);
            }
        }

        if !results.is_empty() {
            crate::utils::logging::append_debug_log_line(&format!(
                "[web_search] Brave Search succeeded: {} results",
                results.len()
            ));
            break;
        }
    }

    crate::utils::logging::append_debug_log_line(&format!(
        "[web_search] Brave Search completed: {} total results",
        results.len()
    ));
    Ok(results)
}

impl ToolInvocation for WebSearchInvocation {
    fn get_description(&self) -> String {
        format!("Search web for: {}", self.params.query)
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
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let query = self.params.query.clone();
        let num = self.params.num.unwrap_or(5).min(10);

        Box::pin(async move {
            // 离线模式：WebSearch 直接拒绝（对标 Claude Code /network）
            if crate::core::offline::is_offline() {
                crate::utils::logging::append_debug_log_line(
                    "[web_search] Refusing search (offline mode)",
                );
                return Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    // 提示写在 output 里（error 为空时 output 才是模型看到的那一份），
                    // llm_content 全仓库没人读
                    output: "Web search is unavailable because offline mode is ON. \
                             Do not retry — it stays off until the user runs /network off."
                        .to_string(),
                    error: None,
                    data: None,
                });
            }

            crate::utils::logging::append_debug_log_line(&format!(
                "[web_search] Starting search: query='{}', num={}",
                query, num
            ));

            // Three-engine fallback chain: Brave → DuckDuckGo → Startpage
            let mut search_results = vec![];
            let mut source = "unknown".to_string();

            // 1. Brave Search
            match search_brave(&query, num).await {
                Ok(results) if !results.is_empty() => {
                    search_results = results;
                    source = "Brave".to_string();
                }
                _ => {
                    crate::utils::logging::append_debug_log_line(
                        "[web_search] Brave failed or no results, switching to DuckDuckGo",
                    );
                    // 2. DuckDuckGo
                    match search_duckduckgo(&query, num).await {
                        Ok(results) if !results.is_empty() => {
                            search_results = results;
                            source = "DuckDuckGo".to_string();
                        }
                        _ => {
                            crate::utils::logging::append_debug_log_line(
                                "[web_search] DuckDuckGo failed or no results, switching to Startpage",
                            );
                            // 3. Startpage
                            match search_startpage(&query, num).await {
                                Ok(results) if !results.is_empty() => {
                                    search_results = results;
                                    source = "Startpage".to_string();
                                }
                                Ok(_) => {}  // Empty results
                                Err(_) => {} // Engine failed
                            }
                        }
                    }
                }
            }

            if search_results.is_empty() {
                crate::utils::logging::append_debug_log_line(
                    "[web_search] All engines returned no results",
                );
                return Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: format!(
                        "No results for \"{}\". All three engines (Brave, DuckDuckGo, Startpage) \
                         came back empty, which usually means the query is too narrow or the \
                         engines are rate-limiting. Rephrase before trying again — repeating this \
                         exact query will return nothing again.",
                        query
                    ),
                    error: None,
                    data: None,
                });
            }

            crate::utils::logging::append_debug_log_line(&format!(
                "[web_search] Used {} to return {} results",
                source,
                search_results.len()
            ));

            // Format output
            let inline_pages = inline_content_pages();
            let mut output = String::new();
            // 开头就报条数和引擎：这一行既是 UI 里唯一能看到的"到底找到没找到"，也是模型
            // 判断该不该再搜一次的依据
            output.push_str(&format!(
                "Found {} result(s) for \"{}\" via {}.\n\n",
                search_results.len(),
                query,
                source
            ));

            for (i, result) in search_results.iter().enumerate() {
                output.push_str(&format!(
                    "{}. {}\n   URL: {}\n   Snippet: {}\n",
                    i + 1,
                    result.title,
                    result.url,
                    result.snippet
                ));

                // 只在显式开启时才抓正文，见 inline_content_pages
                if i < inline_pages {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[web_search] Extracting content: {}",
                        result.url
                    ));
                    match fetch_and_extract(&result.url).await {
                        Ok(content) if !content.is_empty() => {
                            // 按字符截断。原来是 `&content[..2000]` —— 抓中文网页时
                            // 2000 字节几乎必然落在某个汉字中间，直接 panic，而工具
                            // 执行没有 catch_unwind 兜底，一崩整个 agent worker 就
                            // 停了：用户看到的是"搜了但系统不知道"。
                            let truncated = crate::utils::string_utils::truncate_chars(
                                &content,
                                CONTENT_EXCERPT_CHARS,
                            );
                            output.push_str(&format!("   Content excerpt: {}\n", truncated));
                        }
                        _ => {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[web_search] Content extraction failed: {}",
                                result.url
                            ));
                            output.push_str("   (Content extraction failed)\n");
                        }
                    }
                }
                output.push('\n');
            }

            if inline_pages == 0 {
                output.push_str(
                    "These are titles, URLs and snippets only. \
                     Call WebFetch on one of the URLs above if you need a page's full text; \
                     searching again for the same thing will return the same list.\n",
                );
            }

            Ok(CoreToolResult {
                llm_content: None,
                return_display: None,
                output,
                error: None,
                data: None,
            })
        })
    }
}

/// Search DuckDuckGo HTML version and parse results
async fn search_duckduckgo(query: &str, num: u32) -> Result<Vec<SearchResult>> {
    crate::utils::logging::append_debug_log_line(&format!(
        "[web_search] DuckDuckGo started: query='{}', num={}",
        query, num
    ));

    random_delay(800, 2500).await;

    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        encode_uri_component(query)
    );

    let resp = CLIENT
        .get(&url)
        .header("User-Agent", random_ua())
        .send()
        .await
        .context("DDG request failed")?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "DDG HTTP {}: {}",
            resp.status(),
            resp.text().await?
        ));
    }

    let html = resp.text().await?;
    let document = Html::parse_document(&html);

    // DuckDuckGo HTML selectors
    let result_selector = Selector::parse(".result").unwrap();
    let title_selector = Selector::parse(".result__a").unwrap();
    let snippet_selector = Selector::parse(".result__snippet").unwrap();
    let url_selector = Selector::parse(".result__url").unwrap();

    let mut results = Vec::new();

    for result in document.select(&result_selector).take(num as usize) {
        let title = result
            .select(&title_selector)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = result
            .select(&title_selector)
            .next()
            .and_then(|e| e.value().attr("href"))
            .map(|s| s.to_string())
            .unwrap_or_default();

        // Clean up DuckDuckGo redirect URLs
        let url = if url.starts_with("//duckduckgo.com/l/") {
            extract_url_from_ddg_redirect(&url)
        } else {
            url
        };

        let snippet = result
            .select(&snippet_selector)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .or_else(|| {
                result
                    .select(&url_selector)
                    .next()
                    .map(|e| e.text().collect::<String>().trim().to_string())
            })
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
    }

    crate::utils::logging::append_debug_log_line(&format!(
        "[web_search] DuckDuckGo completed: {} total results",
        results.len()
    ));
    Ok(results)
}

/// Startpage search (privacy proxy for Google results)
async fn search_startpage(query: &str, num: u32) -> Result<Vec<SearchResult>> {
    crate::utils::logging::append_debug_log_line(&format!(
        "[web_search] Startpage started: query='{}', num={}",
        query, num
    ));

    random_delay(1000, 3000).await;

    let url = format!(
        "https://www.startpage.com/do/search?q={}",
        encode_uri_component(query)
    );

    let resp = CLIENT
        .get(&url)
        .header("User-Agent", random_ua())
        .send()
        .await
        .context("Startpage request failed")?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("Startpage HTTP {}", resp.status()));
    }

    let html = resp.text().await?;
    let doc = Html::parse_document(&html);

    let result_sel = Selector::parse(".w-gl__result, .search-result, article").unwrap();
    let title_sel = Selector::parse(".w-gl__title a, h3 a, .title a").unwrap();
    let url_sel = Selector::parse(".w-gl__title a, h3 a, a[href]").unwrap();
    let desc_sel = Selector::parse(".w-gl__description, .description, p").unwrap();

    let mut results = vec![];

    for elem in doc.select(&result_sel).take(num as usize) {
        let title = elem
            .select(&title_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = elem
            .select(&url_sel)
            .next()
            .and_then(|e| e.value().attr("href"))
            .map(str::to_string)
            .unwrap_or_default();

        let snippet = elem
            .select(&desc_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
    }

    crate::utils::logging::append_debug_log_line(&format!(
        "[web_search] Startpage completed: {} total results",
        results.len()
    ));
    Ok(results)
}

/// Extract actual URL from DuckDuckGo redirect URL
fn extract_url_from_ddg_redirect(redirect_url: &str) -> String {
    // URL format: //duckduckgo.com/l/?uddg=ENCODED_URL&rut=...
    if let Some(start) = redirect_url.find("uddg=") {
        let after_uddg = &redirect_url[start + 5..];
        let end = after_uddg.find('&').unwrap_or(after_uddg.len());
        let encoded = &after_uddg[..end];

        // URL decode (DuckDuckGo may double-encode)
        let decoded = decode_uri_component(encoded);
        // Try decoding again in case of double encoding
        let decoded = decode_uri_component(&decoded);
        return decoded;
    }
    redirect_url.to_string()
}

/// Fetch webpage and extract main content using readability
async fn fetch_and_extract(url: &str) -> Result<String> {
    random_delay(500, 1800).await;

    let resp = CLIENT
        .get(url)
        .header("User-Agent", random_ua())
        .send()
        .await
        .context("Page request failed")?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", resp.status()));
    }

    let html = resp.text().await.context("Failed to read page")?;

    // Prefer readability-rust（外壳会兜住畸形标签引发的 panic，见 readability_safe）
    if let Some(article) = crate::core::tools::readability_safe::extract_article(html.clone()).await
    {
        if let Some(content) = article.content.as_ref() {
            if !content.is_empty() {
                return Ok(clean_content(content));
            }
        }
    }

    // Simple fallback: html2text for full page
    let text = html2text::from_read(html.as_bytes(), 80).unwrap_or_else(|_| html.clone());

    Ok(clean_content(&text))
}

/// 样板行的长度上限（字符）
///
/// cookie 提示条、广告标签这类都很短；真正在讲 cookie 的技术段落不会。原来只要一行里
/// 出现 "cookie" 就整行删掉 —— 搜 HTTP cookie / session 相关资料时正文会被删成骨头，
/// 模型拿不到东西就再搜一次。
const BOILERPLATE_MAX_CHARS: usize = 120;

/// 判断是否是网页样板（广告 / cookie 提示 / 隐私政策）
fn is_boilerplate_line(line: &str) -> bool {
    const MARKERS: &[&str] = &[
        "广告",
        "Advertisement",
        "sponsored",
        "Sponsored",
        "cookie",
        "Cookie",
        "Privacy Policy",
    ];

    if line.chars().count() > BOILERPLATE_MAX_CHARS {
        return false;
    }
    MARKERS.iter().any(|marker| line.contains(marker))
}

/// Cleanup function: remove common ads, tracking text, blank lines, etc.
fn clean_content(text: &str) -> String {
    text.lines()
        .map(str::trim)
        // 按字符数过滤碎行 —— 两个汉字就有 6 字节，按字节算等于放行
        .filter(|line| line.chars().count() > 5 && !is_boilerplate_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

impl BaseDeclarativeTool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn display_name(&self) -> &str {
        "Web Search"
    }

    fn description(&self) -> &str {
        "Search the web using Brave, DuckDuckGo, or Startpage (auto fallback). No API key required. Features random UA, anti-bot delays, and ad-filtered content extraction."
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query."
                },
                "num": {
                    "type": "integer",
                    "description": "Number of results to return (default: 5, max: 10)."
                }
            },
            "required": ["query"]
        })
    }

    fn kind(&self) -> Kind {
        Kind::Search
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: WebSearchParams = serde_json::from_value(params.clone())?;
        Ok(Box::new(WebSearchInvocation { params }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 搜 HTTP cookie 相关资料时，正文段落不能因为出现 "cookie" 就被整段删掉。
    #[test]
    fn prose_about_cookies_survives_cleaning() {
        let prose = "A cookie is a small piece of data that a server sends to a user's web \
                     browser, which the browser may store and send back with later requests to \
                     the same server so that the server can tell requests apart.";
        assert!(prose.chars().count() > BOILERPLATE_MAX_CHARS);
        assert_eq!(clean_content(prose), prose);
    }

    #[test]
    fn short_boilerplate_lines_are_dropped() {
        let text =
            "This site uses cookies.\nAdvertisement\n广告\nPrivacy Policy\nReal content here.";
        let cleaned = clean_content(text);
        assert_eq!(cleaned, "Real content here.");
    }

    /// 碎行过滤按字符算：两个汉字 6 字节，按字节算会被放行。
    #[test]
    fn short_cjk_lines_are_dropped_by_char_count() {
        let cleaned = clean_content("知识点\n上网找的知识点很多很多");
        assert_eq!(cleaned, "上网找的知识点很多很多");
    }

    #[test]
    fn a_long_line_mentioning_ads_is_not_treated_as_boilerplate() {
        let line = format!(
            "Advertisement systems are described in detail here: {}",
            "x".repeat(150)
        );
        assert!(!is_boilerplate_line(&line));
    }
}
