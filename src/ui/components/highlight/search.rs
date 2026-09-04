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

/// Search result entry
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file: String,
    pub line_number: usize,
    pub content: String,
    pub score: i32,
}

/// Global search state
#[derive(Debug)]
pub struct GlobalSearchState {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub selected_index: usize,
    pub is_searching: bool,
    pub search_error: Option<String>,
    /// 代次计数器 — 每次 query 变化递增，用于丢弃过期搜索结果
    pub search_generation: u64,
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
            search_generation: 0,
            truncated: false,
        }
    }

    pub fn reset(&mut self) {
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
        self.is_searching = false;
        self.search_error = None;
        self.search_generation = 0;
        self.truncated = false;
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

/// Execute a ripgrep search，返回 (results, truncated)。
///
/// 对标 Claude Code GlobalSearchDialog:
/// - `-n --no-heading -i -m {MAX_MATCHES_PER_FILE} -F -e query`
/// - 结果去重（key = "file:line"）
/// - 达到 MAX_TOTAL_MATCHES 时截断
pub async fn execute_search(query: &str, cwd: &str) -> (Vec<SearchResult>, bool) {
    if query.is_empty() {
        return (Vec::new(), false);
    }

    let max_per_file = MAX_MATCHES_PER_FILE.to_string();
    let output = tokio::process::Command::new("rg")
        .args(&[
            "--line-number",
            "--no-heading",
            "-i",
            "-m",
            &max_per_file,
            "-F",
            "-e",
            query,
            cwd,
        ])
        .output()
        .await;

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let results = parse_rg_output(stdout.trim(), MAX_TOTAL_MATCHES);
            let truncated = results.len() >= MAX_TOTAL_MATCHES;
            (results, truncated)
        }
        Err(_) => (Vec::new(), false),
    }
}

/// 将新结果合并到已有结果中（追加 + 去重，对标 Claude Code 的 append+dedup 策略）。
pub fn merge_results(existing: &mut Vec<SearchResult>, new_results: Vec<SearchResult>) {
    use std::collections::HashSet;
    let seen: HashSet<String> = existing
        .iter()
        .map(|r| format!("{}:{}", r.file, r.line_number))
        .collect();
    for r in new_results {
        let key = format!("{}:{}", r.file, r.line_number);
        if !seen.contains(&key) {
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
        // Format: file:line:col:content
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
