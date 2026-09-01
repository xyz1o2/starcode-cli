use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult as CoreToolResult,
};
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use rand::seq::SliceRandom;
use readability_rust::Readability;
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
                crate::utils::logging::append_debug_log_line("[web_search] All engines returned no results");
                return Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: "All search engines returned no valid results".to_string(),
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
            let mut output = String::new();
            output.push_str(&format!("Search results for \"{}\" (via {}):\n\n", query, source));

            for (i, result) in search_results.iter().enumerate() {
                output.push_str(&format!(
                    "{}. {}\n   URL: {}\n   Snippet: {}\n",
                    i + 1,
                    result.title,
                    result.url,
                    result.snippet
                ));

                // Fetch full content for top 3 results
                if i < 3 {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[web_search] Extracting content: {}",
                        result.url
                    ));
                    match fetch_and_extract(&result.url).await {
                        Ok(content) if !content.is_empty() => {
                            let truncated = if content.len() > 2000 {
                                format!("{}...", &content[..2000])
                            } else {
                                content
                            };
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

    // Prefer readability-rust
    if let Some(article) = Readability::new(&html, None)
        .ok()
        .and_then(|mut p| p.parse())
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

/// Cleanup function: remove common ads, tracking text, blank lines, etc.
fn clean_content(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.contains("广告")
                && !line.contains("Advertisement")
                && !line.contains("sponsored")
                && !line.contains("cookie")
                && !line.contains("Privacy Policy")
                && line.len() > 5 // Filter out short garbage lines
        })
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
