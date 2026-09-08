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
    /// 当前 ripgrep 子进程的取消令牌，归 UI 状态所有。
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

    /// 使当前请求失效，并让正在运行的 ripgrep 子进程及时退出。
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
/// 对标 Claude Code GlobalSearchDialog:
/// - `-n --no-heading -i -m {MAX_MATCHES_PER_FILE} -F -e query`
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
            "--line-number",
            "--no-heading",
            "-i",
            "-m",
            &max_per_file,
            "-F",
            "-e",
            query,
        ])
        .arg(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return Some((Vec::new(), false)),
    };
    let stdout = child.stdout.take();
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut stdout) = stdout {
            let _ = stdout.read_to_end(&mut bytes).await;
        }
        bytes
    });

    tokio::select! {
        result = child.wait() => {
            if result.is_err() {
                let _ = stdout_task.await;
                return Some((Vec::new(), false));
            }
        }
        _ = cancellation.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            return None;
        }
    }

    let stdout = stdout_task.await.unwrap_or_default();
    if cancellation.is_cancelled() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&stdout);
    let results = parse_rg_output(stdout.trim(), MAX_TOTAL_MATCHES);
    let truncated = results.len() >= MAX_TOTAL_MATCHES;
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
    if path.len() <= max_width || max_width < 5 {
        return path.to_string();
    }
    // 找到最后一个 / 分隔目录和文件名
    let last_sep = path.rfind('/').unwrap_or(0);
    let file_name = &path[last_sep..];
    let dir_part = &path[..last_sep];
    // 如果文件名本身就够长，截断文件名
    if file_name.len() >= max_width.saturating_sub(3) {
        let start = file_name.len().saturating_sub(max_width.saturating_sub(3));
        return format!("...{}", &file_name[start..]);
    }
    // 保留目录开头 + ... + 文件名
    let dir_budget = max_width.saturating_sub(file_name.len()).saturating_sub(3);
    format!("{}...{}", &dir_part[..dir_budget], file_name)
}

/// Parse ripgrep output into SearchResult entries
fn parse_rg_output(output: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    for line in output.lines().take(max_results) {
        // Format: file:line:content
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() >= 3 {
            let file = parts[0].to_string();
            let line_number = parts[1].parse().unwrap_or(0);
            let content = parts[2].to_string();

            results.push(SearchResult {
                file,
                line_number,
                content,
                score: 0,
            });
        }
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

    #[tokio::test]
    async fn pre_cancelled_search_returns_no_result() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(execute_search("needle", Path::new("."), &token)
            .await
            .is_none());
    }
}

/// 高亮文本中的 query 匹配（对标 CCB highlightMatch — inverse video）。
///
/// 返回 Vec<Span>，匹配部分用 REVERSED 样式高亮。
pub fn highlight_query_matches(text: &str, query: &str) -> Vec<Span<'static>> {
    if query.is_empty() || text.is_empty() {
        return vec![Span::raw(text.to_string())];
    }

    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    let mut spans = Vec::new();
    let mut last_end = 0;

    // 查找所有匹配位置
    let mut search_start = 0;
    while let Some(pos) = text_lower[search_start..].find(&query_lower) {
        let abs_pos = search_start + pos;
        // 匹配前的普通文本
        if abs_pos > last_end {
            spans.push(Span::raw(text[last_end..abs_pos].to_string()));
        }
        // 匹配部分 — inverse video 高亮
        let match_end = abs_pos + query.len();
        spans.push(Span::styled(
            text[abs_pos..match_end].to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
        last_end = match_end;
        search_start = match_end;
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
        .border_style(Style::default().fg(Color::DarkGray));

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
            .border_style(Style::default().fg(Color::DarkGray)),
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
