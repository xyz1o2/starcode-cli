//! `readability-rust` 的安全外壳。
//!
//! # 为什么需要它
//!
//! 上游 `Readability::parse` 会把页面里读到的元素名直接喂给 CSS 选择器解析器
//! （`readability-rust-0.1.0/src/lib.rs:406` 的 `Selector::parse(tag_name).unwrap()`）。
//! 抓回来的网页里只要有一个畸形标签名 —— 比如带 `%`、`{` 这类不是合法 CSS
//! 标识符的字符 —— `Selector::parse` 就返回 `Err(UnexpectedToken(..))`，那个
//! `unwrap()` 直接 panic：
//!
//! ```text
//! thread 'tokio-rt-worker' panicked at readability-rust-0.1.0/src/lib.rs:406:54:
//! called `Result::unwrap()` on an `Err` value: UnexpectedToken(Delim('%'))
//! ```
//!
//! 输入是外部网页，我们控制不了，也没法靠预先清洗穷举所有畸形标签。所以这里
//! 用 `catch_unwind` 兜住，抽取失败退化成 `None`，调用方各自走 html2text 兜底。
//! 同样的做法在 `core::context::tree_sitter_chunker` 和 `core::tools::semantic_search`
//! 里已经用过一次了。
//!
//! # 顺带解决的两件事
//!
//! - 解析是同步的 CPU 活（大页面能跑上百毫秒），原来直接在 async 任务里跑，
//!   会占着 tokio 工作线程。这里统一走 `spawn_blocking`。
//! - `Readability` 内部持有 `scraper::Html`，不适合跨 `.await` 存活；把解析
//!   整个关进同步函数后，async 侧只拿到纯数据的 `Article`。

use readability_rust::{Article, Readability};

/// 在阻塞线程池里抽取正文：panic 与长耗时都不会波及 tokio 工作线程或 UI。
///
/// 返回 `None` 表示抽取失败（解析器初始化失败、没找到正文、或上游 panic），
/// 调用方应退回自己的兜底路径。
pub async fn extract_article(html: String) -> Option<Article> {
    match tokio::task::spawn_blocking(move || extract_article_blocking(&html)).await {
        Ok(article) => article,
        Err(join_err) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Readability] blocking task failed: {}",
                join_err
            ));
            None
        }
    }
}

/// 同步版本：把上游 `unwrap()` 的 panic 变成 `None`。
///
/// 注意 `catch_unwind` 只阻止 panic 继续展开，**不会**阻止 panic hook 先跑一遍；
/// 所以 UI 的 hook 必须能分辨"非渲染线程的 panic"并且不去动终端，否则终端会被
/// 拽出 alternate screen（见 `ui::app::runtime` 里设置 hook 的地方）。
pub fn extract_article_blocking(html: &str) -> Option<Article> {
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Readability::new(html, None).ok().and_then(|mut p| p.parse())
    }));

    match parsed {
        Ok(article) => article,
        Err(_) => {
            crate::utils::logging::append_debug_log_line(
                "[Readability] parser panicked on malformed markup; falling back to plain text",
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 复现用户遇到的那次崩溃：`<100%>` 这种标签名会被 html5ever 当成元素留在
    /// 树里，再被上游塞进 `Selector::parse` —— 不兜住就是 `Delim('%')` panic。
    #[test]
    fn a_malformed_tag_name_does_not_take_the_process_down() {
        let html = r#"<html><body><article><100%>
            <p>Real article text that is long enough to be scored as content by the
            readability heuristics, so the parser actually reaches the candidate
            selection step where the offending selector is built.</p>
            <p>A second paragraph, also long enough to matter for scoring purposes,
            keeps the candidate from being discarded as too short.</p>
        </100%></article></body></html>"#;

        // 唯一的要求：不 panic。抽不出正文（None）也算通过。
        let _ = extract_article_blocking(html);
    }

    #[test]
    fn a_normal_article_still_extracts() {
        let html = r#"<html><head><title>Hello</title></head><body><article>
            <p>This is a reasonably long paragraph of real prose, present so that the
            readability scoring has something substantial to latch onto and returns an
            article rather than giving up immediately.</p>
            <p>And here is a second paragraph of equally real prose, because a single
            block of text is sometimes not enough to clear the content threshold.</p>
        </article></body></html>"#;

        let article = extract_article_blocking(html).expect("clean markup should extract");
        assert!(
            article.content.unwrap_or_default().contains("real prose"),
            "extracted content should carry the article body"
        );
    }

    #[test]
    fn garbage_input_returns_none_instead_of_panicking() {
        assert!(extract_article_blocking("").is_none());
        assert!(extract_article_blocking("\0\0\0 not html at all %{}[]<<<>>>").is_none());
    }
}
