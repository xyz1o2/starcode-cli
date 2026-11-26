/// Global search dialog — ripgrep-based workspace search with preview.
///
/// Provides:
/// - Real-time search as you type
/// - File path and line number display
/// - Syntax-highlighted preview
/// - Keyboard navigation
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::{path::Path, process::Stdio};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

/// 搜索结果由 UI/worker 协议定义；本组件只负责状态与渲染。
pub use crate::runtime::messages::GlobalSearchMatch as SearchResult;

/// Global search state
#[derive(Debug)]
pub struct GlobalSearchState {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub selected_index: usize,
    pub is_searching: bool,
    pub search_error: Option<String>,
    /// 当前生效搜索的全局唯一标识；关闭或替换弹窗会使旧请求失效。
    pub active_request_id: Option<u64>,
    /// 当前 `rg` 子进程的取消令牌，归 UI 状态所有。
    cancel_token: Option<CancellationToken>,
    /// 结果是否被截断（达到 MAX_TOTAL_MATCHES）
    pub truncated: bool,
}

/// 每个文件最大匹配数
const MAX_MATCHES_PER_FILE: usize = 10;
/// 全局最大匹配数
const MAX_TOTAL_MATCHES: usize = 500;

impl GlobalSearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
            is_searching: false,
            search_error: None,
            active_request_id: None,
            cancel_token: None,
            truncated: false,
        }
    }

    pub fn reset(&mut self) {
        self.cancel_active_request();
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
        self.search_error = None;
        self.truncated = false;
    }

    /// 取消旧请求并开始一个新的搜索；请求标识由 ChatState 分配，跨弹窗不复用。
    pub fn begin_request(&mut self, request_id: u64) -> CancellationToken {
        self.cancel_active_request();
        let token = CancellationToken::new();
        self.active_request_id = Some(request_id);
        self.cancel_token = Some(token.clone());
        self.is_searching = true;
        token
    }

    /// 使当前请求失效，并让正在运行的 `rg` 子进程及时退出。
    pub fn cancel_active_request(&mut self) {
        if let Some(token) = self.cancel_token.take() {
            token.cancel();
        }
        self.active_request_id = None;
        self.is_searching = false;
    }

    /// 应用当前请求的批量结果；过期结果不可改变界面。
    pub fn apply_results(
        &mut self,
        request_id: u64,
        results: Vec<SearchResult>,
        truncated: bool,
    ) -> bool {
        if self.active_request_id != Some(request_id) {
            return false;
        }
        self.results = results;
        self.truncated = truncated;
        self.selected_index = 0;
        self.cancel_token = None;
        self.active_request_id = None;
        self.is_searching = false;
        true
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected_index < self.results.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected_index)
    }
}

/// Execute a cancellable ripgrep search, returning `None` when superseded.
///
/// 对标 Claude Code GlobalSearchDialog：
/// - `--json --line-number --ignore-case --max-count {MAX_MATCHES_PER_FILE} --fixed-strings`
/// - 搜索整个工作区，包含尚未被 Git 跟踪的新文件
/// - `rg` 不可用时使用内嵌后端，不让搜索功能静默失效
/// - 结果去重（key = "file:line"）
/// - 达到 MAX_TOTAL_MATCHES 时截断
pub async fn execute_search(
    query: &str,
    cwd: &Path,
    cancellation: &CancellationToken,
) -> Option<(Vec<SearchResult>, bool)> {
    if query.is_empty() || cancellation.is_cancelled() {
        return None;
    }

    let max_per_file = MAX_MATCHES_PER_FILE.to_string();
    let mut command = tokio::process::Command::new("rg");
    command
        .args([
            "--json",
            "--ignore-case",
            "--max-count",
            &max_per_file,
            "--fixed-strings",
            "-e",
            query,
            ".",
        ])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return execute_native_search(query, cwd, cancellation).await,
    };
    let stdout = child.stdout.take();
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut stdout) = stdout {
            let _ = stdout.read_to_end(&mut bytes).await;
        }
        bytes
    });

    let status = tokio::select! {
        result = child.wait() => match result {
            Ok(status) => status,
            Err(_) => {
                let _ = stdout_task.await;
                return execute_native_search(query, cwd, cancellation).await;
            }
        },
        _ = cancellation.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            return None;
        }
    };

    let stdout = stdout_task.await.unwrap_or_default();
    if cancellation.is_cancelled() {
        return None;
    }
    // Exit code 1 means "no matches"; other failures use the built-in backend.
    if status.code() != Some(0) && status.code() != Some(1) {
        return execute_native_search(query, cwd, cancellation).await;
    }
    let stdout = String::from_utf8_lossy(&stdout);
    let results = parse_rg_json_output(&stdout, MAX_TOTAL_MATCHES);
    let truncated = results.len() >= MAX_TOTAL_MATCHES;
    Some((results, truncated))
}

/// 在 `rg` 不可用或子进程失败时，使用内嵌的 Rust grep 后端。
///
/// 这不是调用外部二进制，因此不会因为用户没有安装 ripgrep 而失效。
async fn execute_native_search(
    query: &str,
    cwd: &Path,
    cancellation: &CancellationToken,
) -> Option<(Vec<SearchResult>, bool)> {
    let query = query.to_string();
    let cwd = cwd.to_path_buf();
    let search_cwd = cwd.clone();
    let result = tokio::task::spawn_blocking(move || {
        let config = crate::core::tools::ripgrep::RipgrepConfig {
            case_sensitive: false,
            whole_word: false,
            regex: false,
            max_results: Some(MAX_TOTAL_MATCHES as u32),
            file_types: None,
            exclude_patterns: None,
            include_patterns: None,
            include_base: None,
            include_hidden: true,
            max_file_size: Some(2 * 1024 * 1024),
        };
        crate::core::tools::ripgrep::search_with_ripgrep_stats(
            &query,
            &search_cwd.to_string_lossy(),
            config,
        )
        .map(|(matches, stats)| (matches, stats.hit_limit))
        .map_err(|error| error.to_string())
    })
    .await;

    if cancellation.is_cancelled() {
        return None;
    }

    let (matches, truncated) = result.ok()?.ok()?;
    let results = matches
        .into_iter()
        .filter_map(|matched| {
            let file = Path::new(&matched.file)
                .strip_prefix(&cwd)
                .unwrap_or(Path::new(&matched.file))
                .to_string_lossy()
                .trim_start_matches(std::path::MAIN_SEPARATOR)
                .to_string();
            Some(SearchResult {
                file,
                line_number: matched.line.unwrap_or_default() as usize,
                content: matched.text.unwrap_or_default(),
                score: 0,
            })
        })
        .collect();
    Some((results, truncated))
}

/// 将新结果合并到已有结果中（追加 + 去重，对标 Claude Code 的 append+dedup 策略）。
pub fn merge_results(existing: &mut Vec<SearchResult>, new_results: Vec<SearchResult>) {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = existing
        .iter()
        .map(|r| format!("{}:{}", r.file, r.line_number))
        .collect();
    for r in new_results {
        let key = format!("{}:{}", r.file, r.line_number);
        if seen.insert(key) {
            existing.push(r);
        }
    }
}

/// 路径截断：保留两端（对标 CCB truncatePathMiddle）。
///
/// 当路径过长时，保留目录开头和文件名，中间用 `...` 连接。
/// 例如: `/very/long/path/to/file.rs` → `/very/.../file.rs`
pub fn truncate_path_middle(path: &str, max_width: usize) -> String {
    use crate::ui::utils::render::{char_display_width, display_width};

    if display_width(path) <= max_width {
        return path.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    // 找到最后一个 / 分隔目录和文件名
    let last_sep = path.rfind('/').unwrap_or(0);
    let (dir_part, file_name) = path.split_at(last_sep);
    let budget = max_width - 3;
    let file_width = display_width(file_name);
    // 按终端列宽预算选择字符边界，不能把字节偏移当成字符偏移。
    if file_width >= budget {
        let mut start = file_name.len();
        let mut width = 0;
        for (index, ch) in file_name.char_indices().rev() {
            width += char_display_width(ch);
            if width > budget {
                break;
            }
            start = index;
        }
        return format!("...{}", &file_name[start..]);
    }
    // 保留目录开头 + ... + 文件名
    let dir_budget = budget - file_width;
    let mut end = 0;
    let mut width = 0;
    for (index, ch) in dir_part.char_indices() {
        width += char_display_width(ch);
        if width > dir_budget {
            break;
        }
        end = index + ch.len_utf8();
    }
    format!("{}...{}", &dir_part[..end], file_name)
}

/// Parse `rg --json` output into search results.
///
/// 使用 JSON 而不是 `file:line:text`，避免 Windows 盘符和合法的冒号文件名
/// 被误当成字段分隔符。`rg` 的其它事件（begin/end/summary）会被忽略。
fn parse_rg_json_output(output: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    for line in output.lines() {
        if results.len() >= max_results {
            break;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if event.get("type").and_then(serde_json::Value::as_str) != Some("match") {
            continue;
        }
        let Some(data) = event.get("data") else {
            continue;
        };
        let Some(file) = data
            .get("path")
            .and_then(|path| path.get("text"))
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(line_number) = data
            .get("line_number")
            .and_then(serde_json::Value::as_u64)
            .map(|line| line as usize)
        else {
            continue;
        };
        let content = data
            .get("lines")
            .and_then(|lines| lines.get("text"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim_end_matches(['\r', '\n'])
            .to_string();

        results.push(SearchResult {
            file: file.to_string(),
            line_number,
            content,
            score: 0,
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(file: &str, line_number: usize, content: &str) -> SearchResult {
        SearchResult {
            file: file.to_string(),
            line_number,
            content: content.to_string(),
            score: 0,
        }
    }

    #[test]
    fn active_request_accepts_only_matching_results() {
        let mut state = GlobalSearchState::new();
        let token = state.begin_request(7);
        assert!(state.is_searching);
        assert!(!token.is_cancelled());

        assert!(!state.apply_results(6, vec![result("old.rs", 1, "old")], false));
        assert!(state.is_searching);
        assert!(state.results.is_empty());

        assert!(state.apply_results(7, vec![result("new.rs", 2, "new")], true));
        assert_eq!(state.results[0].file, "new.rs");
        assert!(state.truncated);
        assert!(!state.is_searching);
        assert_eq!(state.active_request_id, None);
    }

    #[test]
    fn beginning_or_cancelling_request_cancels_predecessor() {
        let mut state = GlobalSearchState::new();
        let first = state.begin_request(1);
        let second = state.begin_request(2);
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        state.cancel_active_request();
        assert!(second.is_cancelled());
        assert!(!state.is_searching);
        assert_eq!(state.active_request_id, None);
    }

    #[test]
    fn merge_results_deduplicates_the_incoming_batch() {
        let mut results = vec![result("lib.rs", 1, "existing")];
        merge_results(
            &mut results,
            vec![
                result("lib.rs", 1, "duplicate existing"),
                result("main.rs", 2, "first"),
                result("main.rs", 2, "duplicate incoming"),
            ],
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[1].content, "first");
    }

    #[test]
    fn json_parser_preserves_colons_in_paths() {
        let output = serde_json::json!({
            "type": "match",
            "data": {
                "path": {"text": "src/a:b.rs"},
                "lines": {"text": "fn needle() {}\n"},
                "line_number": 7
            }
        })
        .to_string();

        let results = parse_rg_json_output(&output, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file, "src/a:b.rs");
        assert_eq!(results[0].line_number, 7);
        assert_eq!(results[0].content, "fn needle() {}");
    }

    #[test]
    fn path_truncation_uses_character_boundaries_and_display_width() {
        use crate::ui::utils::render::display_width;

        assert_eq!(
            truncate_path_middle("/🙂🙂🙂/file.rs", 14),
            "/🙂.../file.rs"
        );
        assert_eq!(truncate_path_middle("🙂🙂🙂.rs", 8), "...🙂.rs");
        assert_eq!(truncate_path_middle("src/🙂.rs", 10), "src/🙂.rs");
        assert_eq!(
            truncate_path_middle("/alpha/beta/file.rs", 18),
            "/alpha/.../file.rs"
        );

        for path in ["/🙂🙂🙂/文件.rs", "🙂🙂🙂.rs", "src/short.rs", ""] {
            for width in 0..=24 {
                let truncated = truncate_path_middle(path, width);
                assert!(display_width(&truncated) <= width);
            }
        }
    }

    #[test]
    fn query_highlights_preserve_unicode_text_and_original_offsets() {
        for (text, query, expected_matches) in [
            ("K🙂Needle", "needle", vec!["Needle"]),
            ("🙂KK🙂", "k", vec!["K", "K"]),
            ("🙂K🙂", "K", vec!["K"]),
            ("İ🙂needle", "needle", vec!["needle"]),
            ("🙂 a.b axb A.B 🙂", "a.b", vec!["a.b", "A.B"]),
            ("🙂🙂🙂", "🙂🙂", vec!["🙂🙂"]),
            ("🙂 text", "", vec![]),
            ("🙂 text", "absent", vec![]),
            ("", "needle", vec![]),
        ] {
            let spans = highlight_query_matches(text, query);
            let restored: String = spans.iter().map(|span| span.content.as_ref()).collect();
            assert_eq!(restored, text);
            let highlighted: Vec<&str> = spans
                .iter()
                .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
                .map(|span| span.content.as_ref())
                .collect();
            assert_eq!(highlighted, expected_matches);
        }
    }

    #[tokio::test]
    async fn pre_cancelled_search_returns_no_result() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(execute_search("needle", Path::new("."), &token)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn non_git_directory_uses_the_embedded_search_backend() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(
            directory.path().join("note.txt"),
            "native fallback needle\n",
        )
        .expect("write search fixture");
        let token = CancellationToken::new();

        let (results, _) = execute_search("needle", directory.path(), &token)
            .await
            .expect("search should complete");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file, "note.txt");
        assert_eq!(results[0].line_number, 1);
    }
}

/// 高亮文本中的 query 匹配（对标 CCB highlightMatch — inverse video）。
///
/// 返回 Vec<Span>，匹配部分用 REVERSED 样式高亮。
pub fn highlight_query_matches(text: &str, query: &str) -> Vec<Span<'static>> {
    if query.is_empty() || text.is_empty() {
        return vec![Span::raw(text.to_string())];
    }

    // 与搜索后端一样做字面量、大小写不敏感匹配；偏移始终来自原文，
    // 避免 Unicode 大小写转换改变字节长度后切进字符内部。
    let Ok(pattern) = regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
    else {
        return vec![Span::raw(text.to_string())];
    };
    let mut spans = Vec::new();
    let mut last_end = 0;

    for matched in pattern.find_iter(text) {
        // 匹配前的普通文本
        if matched.start() > last_end {
            spans.push(Span::raw(text[last_end..matched.start()].to_string()));
        }
        // 匹配部分 — inverse video 高亮
        spans.push(Span::styled(
            matched.as_str().to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
        last_end = matched.end();
    }
    // 剩余普通文本
    if last_end < text.len() {
        spans.push(Span::raw(text[last_end..].to_string()));
    }

    if spans.is_empty() {
        vec![Span::raw(text.to_string())]
    } else {
        spans
    }
}

/// Render the global search dialog
///
/// 对标 CCB GlobalSearchDialog: 响应式布局 —
/// columns >= 140 时预览在右侧，否则在底部。
pub fn render_global_search(f: &mut Frame, state: &GlobalSearchState, area: Rect) {
    use super::fuzzy_picker;

    f.render_widget(Clear, area);

    // Pane 分割线（对标 CCB Pane Divider）
    fuzzy_picker::render_pane_divider(f, area, Color::Cyan);

    // 计算布局（对标 CCB FuzzyPicker 布局）
    let (areas, _preview_pos, _content_area) = fuzzy_picker::compute_layout(area, 140, 5);

    // Search input（对标 CCB SearchBox）
    fuzzy_picker::render_search_input(f, areas.search, "Search", &state.query, "Type to search…");

    // Results list（对标 CCB FuzzyPicker List + ListItem）
    let match_label = fuzzy_picker::format_match_label(
        state.results.len(),
        state.truncated,
        state.is_searching,
        "matches",
    );
    let results_block = Block::default()
        .title(format!(" Results ({}) ", match_label))
        .borders(Borders::ALL)
        .border_style(crate::ui::themes::theme::muted_style());

    if state.results.is_empty() {
        let empty_msg = if state.is_searching {
            "Searching…"
        } else if state.query.is_empty() {
            "Type to search…"
        } else {
            "No matches"
        };
        fuzzy_picker::render_empty_state(f, areas.list, results_block, empty_msg);
    } else {
        let query = state.query.clone();
        fuzzy_picker::render_scrolling_list(
            f,
            areas.list,
            results_block,
            &state.results,
            state.selected_index,
            |result, _is_focused| {
                let file_name = std::path::Path::new(&result.file)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| result.file.clone());
                let mut spans = vec![];
                let path_display = truncate_path_middle(&result.file, 40);
                spans.push(Span::styled(
                    format!("{}:", path_display),
                    Style::default().fg(Color::Yellow),
                ));
                spans.push(Span::styled(
                    format!("{} ", result.line_number),
                    Style::default().fg(Color::Green),
                ));
                spans.extend(highlight_query_matches(result.content.trim_start(), &query));
                Line::from(spans)
            },
        );
    }

    // Preview（对标 CCB FuzzyPicker renderPreview）
    let preview_lines = if let Some(result) = state.selected_result() {
        vec![
            Line::from(vec![Span::styled(
                format!("{}:{}", result.file, result.line_number),
                Style::default().fg(Color::Cyan),
            )]),
            Line::from(highlight_query_matches(
                result.content.trim_start(),
                &state.query,
            )),
        ]
    } else {
        vec![]
    };
    let preview = Paragraph::new(preview_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(crate::ui::themes::theme::muted_style()),
    );
    f.render_widget(preview, areas.preview);

    // Byline（对标 CCB FuzzyPicker byline）
    fuzzy_picker::render_byline(
        f,
        area,
        &[
            ("↑/↓", "navigate"),
            ("Enter", "open"),
            ("Tab", "mention"),
            ("Shift+Tab", "insert path"),
            ("Esc", "cancel"),
        ],
    );
}
