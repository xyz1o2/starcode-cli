/// Quick open dialog — fuzzy file finder with preview.
///
/// 对标 Claude Code QuickOpenDialog: 三区域布局（搜索框 / 文件列表 / 预览）。
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// File entry
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub size: String,
    pub modified: String,
}

/// Quick open state
#[derive(Debug)]
pub struct QuickOpenState {
    pub query: String,
    pub files: Vec<FileEntry>,
    pub selected_index: usize,
    pub preview_content: Option<String>,
    /// 代次计数器 — 每次 query 变化递增，用于丢弃过期搜索结果
    pub search_generation: u64,
}

impl QuickOpenState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            files: Vec::new(),
            selected_index: 0,
            preview_content: None,
            search_generation: 0,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected_index < self.files.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    pub fn selected_file(&self) -> Option<&FileEntry> {
        self.files.get(self.selected_index)
    }
}

/// Search files with fuzzy matching
pub async fn search_files(query: &str, cwd: &str, max_results: usize) -> Vec<FileEntry> {
    if query.is_empty() {
        return Vec::new();
    }

    let output = tokio::process::Command::new("fd")
        .args(&[
            "--type",
            "f",
            "--max-results",
            &max_results.to_string(),
            query,
            cwd,
        ])
        .output()
        .await;

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .take(max_results)
                .map(|path| {
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.to_string());
                    FileEntry {
                        path: path.to_string(),
                        name,
                        size: String::new(),
                        modified: String::new(),
                    }
                })
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

/// Render quick open dialog
///
/// 对标 CCB QuickOpenDialog: 响应式布局 —
/// columns >= 120 时预览在右侧，否则在底部。
pub fn render_quick_open(f: &mut Frame, state: &QuickOpenState, area: Rect) {
    use super::fuzzy_picker;

    f.render_widget(Clear, area);

    // Pane 分割线（对标 CCB Pane Divider）
    fuzzy_picker::render_pane_divider(f, area, Color::Cyan);

    // 计算布局（对标 CCB FuzzyPicker 布局）
    let (areas, _preview_pos, _content_area) = fuzzy_picker::compute_layout(area, 120, 5);

    // Search input（对标 CCB SearchBox）
    fuzzy_picker::render_search_input(
        f,
        areas.search,
        "Quick Open",
        &state.query,
        "Type to search files…",
    );

    // File list（对标 CCB FuzzyPicker List + ListItem）
    let match_label = fuzzy_picker::format_match_label(state.files.len(), false, false, "files");
    let files_block = Block::default()
        .title(format!(" Files ({}) ", match_label))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    if state.files.is_empty() {
        let empty_msg = if state.query.is_empty() {
            "Start typing to search…"
        } else {
            "No matching files"
        };
        fuzzy_picker::render_empty_state(f, areas.list, files_block, empty_msg);
    } else {
        fuzzy_picker::render_scrolling_list(
            f,
            areas.list,
            files_block,
            &state.files,
            state.selected_index,
            |file, _is_focused| {
                let path_display =
                    crate::ui::components::highlight::search::truncate_path_middle(&file.path, 50);
                Line::from(vec![Span::styled(
                    path_display,
                    Style::default().fg(Color::White),
                )])
            },
        );
    }

    // Preview（对标 CCB FuzzyPicker renderPreview）
    let preview_lines = if let Some(file) = state.selected_file() {
        vec![
            Line::from(Span::styled(
                file.path.clone(),
                Style::default().fg(Color::Cyan),
            )),
            Line::from(Span::styled(
                state
                    .preview_content
                    .as_deref()
                    .unwrap_or("Press Enter to open"),
                Style::default().fg(Color::DarkGray),
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
