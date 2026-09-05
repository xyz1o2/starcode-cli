pub mod early_input;
pub mod logic;
pub mod runtime;

use crate::ui::state::ChatState;
use ratatui::prelude::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;

fn render_page(f: &mut ratatui::Frame<'_>, state: &mut ChatState, viewport: Rect) -> Rect {
    use ratatui::layout::{Constraint, Direction, Layout};

    let input_height = crate::ui::components::chat_input::calc_input_height(state);
    let status_lines =
        crate::ui::components::status_line::build_status_lines(state, viewport.width);
    let footer_h = input_height + status_lines.len() as u16;

    // Spinner between chat and footer — always reserve space to avoid layout jump
    // The spinner provides feedback that work is happening
    let show_spinner = state.is_processing;
    let spinner_h = 2u16; // Always reserve 2 lines to prevent layout jumps

    // Token warning: show when context window is getting full
    let token_warning_lines = crate::ui::components::status_line::token_warning_line(state);
    let token_warning_h = token_warning_lines.len() as u16;

    // Task panel auto-show/hide logic
    state.task_panel.auto_show_if_needed();
    state.task_panel.check_auto_hide();

    // Task panel above input (like openclaude's TaskListV2 above PromptInput)
    let task_panel_visible = state.task_panel.is_visible;
    let task_panel_h = if task_panel_visible {
        let task_count = state.task_panel.flatten_tasks().len();
        // Compact: title + tasks + borders, capped at 8 rows
        (task_count as u16 + 3).min(8).max(3)
    } else {
        0u16
    };

    // 后台代理选择器（对标 background agent selector）：仅在 ↓ 进入选择器后占位，
    // 与 task_panel 同一套「按需撑开一段高度」的模式
    let bg_panel_h = crate::ui::components::bg_agent_selector::selector_height(state);

    // 排队中的用户输入（对标 PromptInputQueuedCommands）：紧贴输入框上方，
    // 同样按需撑开
    let queued_panel_h =
        crate::ui::components::queued_input::queued_panel_height(state, viewport.width);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(token_warning_h),
            Constraint::Length(spinner_h),
            Constraint::Length(task_panel_h),
            Constraint::Length(bg_panel_h),
            Constraint::Length(queued_panel_h),
            Constraint::Length(footer_h),
        ])
        .split(viewport);
    let chat_area = chunks[0];
    let token_warning_area = chunks[1];
    let spinner_area = chunks[2];
    let task_area = chunks[3];
    let bg_panel_area = chunks[4];
    let queued_panel_area = chunks[5];
    let footer_area = chunks[6];

    let chat_lines = crate::ui::components::chat_history::render_chat_lines(state, chat_area.width);
    let total = chat_lines.len();
    let max_scroll = total.saturating_sub(chat_area.height as usize);
    if state.auto_follow {
        state.scroll = max_scroll;
    } else if state.scroll > max_scroll {
        state.scroll = max_scroll;
    }
    state.total_rendered_lines = total;
    // Update chat area for mouse event coordinate mapping
    state.last_chat_area = Some(chat_area);
    state.last_chat_height = chat_area.height;

    // Empty state placeholder when no chat history
    let chat_lines = if total == 0 && !state.is_processing {
        let theme = state.theme_manager.current();
        let model = if state.current_model.is_empty() {
            "..."
        } else {
            state.current_model.as_str()
        };
        vec![
            ratatui::text::Line::from(""),
            ratatui::text::Line::from(vec![ratatui::text::Span::styled(
                format!("  ⊙ {}", model),
                ratatui::style::Style::default().fg(theme.success),
            )]),
            ratatui::text::Line::from(vec![ratatui::text::Span::styled(
                crate::core::i18n::t(
                    "ui.empty.hint",
                    "  输入问题开始对话，或 Ctrl+P 打开命令面板",
                    "  Type a question to start, or Ctrl+P for commands",
                ),
                ratatui::style::Style::default().fg(theme.secondary),
            )]),
        ]
    } else {
        chat_lines
    };

    // Write chat lines directly to buffer — stable cell positions enable ratatui diff
    crate::ui::utils::render::write_lines_to_buffer(
        f.buffer_mut(),
        chat_area,
        &chat_lines,
        state.scroll,
    );

    // Render token warning (context window usage)
    if !token_warning_lines.is_empty() {
        f.render_widget(
            ratatui::widgets::Paragraph::new(token_warning_lines),
            token_warning_area,
        );
    }

    // Render spinner between chat and footer (always render to prevent layout jumps)
    let spinner_line = crate::ui::components::status_line::processing_spinner_line(state);
    f.render_widget(ratatui::widgets::Paragraph::new(spinner_line), spinner_area);

    // Render task panel above input (like openclaude's TaskListV2 above PromptInput)
    if task_panel_visible {
        let theme = state.theme_manager.current();
        crate::ui::components::task_panel::render_task_panel_mut(
            f,
            task_area,
            &mut state.task_panel,
            theme,
        );
    }

    if bg_panel_h > 0 {
        let selector_lines =
            crate::ui::components::bg_agent_selector::render_selector(state, bg_panel_area.width);
        f.render_widget(
            ratatui::widgets::Paragraph::new(selector_lines),
            bg_panel_area,
        );
    }

    if queued_panel_h > 0 {
        let queued_lines = crate::ui::components::queued_input::render_queued_panel(
            state,
            queued_panel_area.width,
        );
        f.render_widget(
            ratatui::widgets::Paragraph::new(queued_lines),
            queued_panel_area,
        );
    }

    let footer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(input_height),
            Constraint::Length(status_lines.len() as u16),
        ])
        .split(footer_area);
    let input_area = footer_chunks[0];
    crate::ui::components::chat_input::render_input(f, state, input_area);
    f.render_widget(
        ratatui::widgets::Paragraph::new(status_lines),
        footer_chunks[1],
    );

    // 保存 input 区域位置信息，用于鼠标事件处理
    state.last_input_area = Some(input_area);

    // 滚动到底部浮动指示器 — 当用户向上滚动且有新内容时显示
    if state.show_scroll_to_bottom && !state.auto_follow {
        let indicator_text = ratatui::text::Line::from(ratatui::text::Span::styled(
            " ↓ New messages ",
            ratatui::style::Style::default()
                .fg(ratatui::style::Color::Black)
                .bg(ratatui::style::Color::Cyan),
        ));
        let indicator_width = 18u16;
        let indicator_x = chat_area.x + chat_area.width.saturating_sub(indicator_width + 2);
        let indicator_y = chat_area.y + chat_area.height.saturating_sub(1);
        let indicator_area = ratatui::layout::Rect {
            x: indicator_x,
            y: indicator_y,
            width: indicator_width,
            height: 1,
        };
        if indicator_y >= chat_area.y && indicator_x >= chat_area.x {
            f.render_widget(
                ratatui::widgets::Paragraph::new(indicator_text),
                indicator_area,
            );
        }
    }

    // /clear 确认对话框 — 内联显示在输入框上方
    if state.show_clear_confirmation {
        let confirm_text = ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(
                " ⚠ ",
                ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
            ),
            ratatui::text::Span::styled(
                crate::core::i18n::t(
                    "ui.clear.confirm",
                    "确认清除所有对话? (y/n)",
                    "Clear all conversation? (y/n)",
                ),
                ratatui::style::Style::default().fg(ratatui::style::Color::White),
            ),
        ]);
        let confirm_area = ratatui::layout::Rect {
            x: input_area.x,
            y: input_area.y.saturating_sub(1),
            width: input_area.width,
            height: 1,
        };
        if confirm_area.y >= chat_area.y {
            f.render_widget(ratatui::widgets::Paragraph::new(confirm_text), confirm_area);
        }
    }

    // 大段粘贴确认 — 内联显示在输入框上方
    if state.show_paste_confirmation {
        let line_count = state
            .pending_paste
            .as_ref()
            .map(|t| t.lines().count())
            .unwrap_or(0);
        let paste_confirm_text = ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(
                " ⚠ ",
                ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
            ),
            ratatui::text::Span::styled(
                crate::core::i18n::t(
                    "ui.paste.confirm",
                    &format!("粘贴 {} 行文本？(y/n)", line_count),
                    &format!("Paste {} lines? (y/n)", line_count),
                ),
                ratatui::style::Style::default().fg(ratatui::style::Color::White),
            ),
        ]);
        let paste_confirm_area = ratatui::layout::Rect {
            x: input_area.x,
            y: input_area.y.saturating_sub(1),
            width: input_area.width,
            height: 1,
        };
        if paste_confirm_area.y >= chat_area.y {
            f.render_widget(
                ratatui::widgets::Paragraph::new(paste_confirm_text),
                paste_confirm_area,
            );
        }
    }

    input_area
}

pub fn draw_ui(f: &mut ratatui::Frame<'_>, state: &mut ChatState) {
    state.animation_tick = state.animation_tick.wrapping_add(1);

    let input_area = render_page(f, state, f.area());
    if state.show_command_hints
        || state.show_mention_hints
        || state.show_provider_menu
        || state.show_session_menu
    {
        crate::ui::components::command_suggestions::render_overlays(f, state, input_area);
    }

    // Draw help popup if needed
    if state.show_help {
        crate::ui::components::help_popup::render_help_popup(f, f.area());
    }

    // Draw palette if needed (via unified modal stack)
    if state.is_palette_open() {
        crate::ui::components::palette::render_palette(f, f.area(), state);
    }

    // Draw MCP manager modal if on top of the modal stack
    if matches!(state.top_modal(), Some(crate::ui::state::Modal::Mcp { .. })) {
        crate::ui::components::modal::render_mcp_modal(f, f.area(), state);
    }

    // Draw extension marketplace modal if on top of the modal stack
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::Modal::Market { .. })
    ) {
        crate::ui::components::modal::render_market_modal(f, f.area(), state);
    }

    // Draw plugin manager modal (Claude Code style /plugin) if on top
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::Modal::Plugins { .. })
    ) {
        crate::ui::components::modal::render_plugins_modal(f, f.area(), state);
    }

    // Draw input modal if needed
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::InputModal)
    ) {
        crate::ui::components::input_modal::render_input_modal(f, f.area(), state);
    }

    // Draw status modal if needed
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::StatusModal)
    ) {
        crate::ui::components::status_modal::render_status_modal(f, f.area(), state);
    }

    // Draw global search dialog if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::GlobalSearch)
    ) {
        crate::ui::components::highlight::search::render_global_search(
            f,
            &state.global_search_state,
            f.area(),
        );
    }

    // Draw quick open dialog if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::QuickOpen)
    ) {
        crate::ui::components::highlight::quick_open::render_quick_open(
            f,
            &state.quick_open_state,
            f.area(),
        );
    }

    // Draw history search dialog if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::HistorySearch)
    ) {
        crate::ui::components::highlight::history::render_history_search(
            f,
            &state.history_search_state,
            f.area(),
        );
    }

    // Draw theme picker if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::ThemePicker { .. })
    ) {
        let themes = crate::ui::components::highlight::theme_picker::available_themes();
        crate::ui::components::highlight::theme_picker::render_theme_picker(
            f,
            &themes,
            state.selected_theme_index,
            f.area(),
        );
    }

    // Draw usage stats if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::UsageStats)
    ) {
        let theme = state.theme_manager.current();
        crate::ui::components::highlight::stats::render_usage_stats(
            f,
            &state.usage_stats,
            f.area(),
            theme,
        );
    }

    // Draw export dialog if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::Export)
    ) {
        crate::ui::components::highlight::export::render_export_dialog(
            f,
            &state.export_state,
            f.area(),
        );
    }

    // Draw compression status if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::CompressionStatus)
    ) {
        crate::ui::components::highlight::compression::render_compression_status(
            f,
            &state.compression_state,
            f.area(),
        );
    }

    // Draw context visualization if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::ContextViz)
    ) {
        let theme = state.theme_manager.current();
        crate::ui::components::highlight::context_viz::render_context_visualization(
            f,
            &state.context_breakdown,
            f.area(),
        );
    }

    // Draw error overlay if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::ErrorOverlay)
    ) {
        let theme = state.theme_manager.current();
        crate::ui::components::error_overlay::render_error_overlay(
            f,
            &state.error_overlay_state,
            f.area(),
            theme,
        );
    }

    // Draw log selector if active
    if matches!(
        state.top_modal(),
        Some(crate::ui::state::modal::Modal::LogSelector)
    ) {
        let theme = state.theme_manager.current();
        crate::ui::components::log_selector::render_log_selector(
            f,
            &state.log_selector_state,
            f.area(),
            theme,
        );
    }

    // Draw toast notifications (top-right, stacked)
    render_toasts(f, state);
}

fn render_toasts(f: &mut ratatui::Frame<'_>, state: &mut ChatState) {
    use crate::ui::state::store::ToastKind;

    // Remove expired toasts
    let now = std::time::Instant::now();
    state
        .toast_queue
        .retain(|t| now.duration_since(t.created_at).as_secs() < t.duration_secs);

    if state.toast_queue.is_empty() {
        return;
    }

    let area = f.area();
    let max_width = 40u16;
    let mut y = area.y + 1; // Start 1 row from top

    for toast in state.toast_queue.iter().take(5) {
        let (icon, color) = match toast.kind {
            ToastKind::Info => ("ℹ", ratatui::style::Color::Cyan),
            ToastKind::Success => ("✓", ratatui::style::Color::Green),
            ToastKind::Warning => ("⚠", ratatui::style::Color::Yellow),
            ToastKind::Error => ("✗", ratatui::style::Color::Red),
        };
        let text = format!(" {} {} ", icon, toast.message);
        let width = text.len().min(max_width as usize) as u16;
        let x = area.x + area.width.saturating_sub(width + 2);
        let toast_area = ratatui::layout::Rect {
            x,
            y,
            width,
            height: 1,
        };
        if y < area.y + area.height {
            f.render_widget(
                ratatui::widgets::Paragraph::new(ratatui::text::Line::from(
                    ratatui::text::Span::styled(text, ratatui::style::Style::default().fg(color)),
                )),
                toast_area,
            );
        }
        y += 1;
    }
}

use crate::runtime::messages::AgentRequest;
use crate::runtime::messages::StreamMessage;

/// Alternative event loop using crossterm::event::poll/read directly.
///
/// **Deprecated**: Use `runtime::run_app` instead, which uses a dedicated keyboard
/// reader thread with `use-dev-tty` feature for better WSL2/SSH compatibility.
///
/// This function is kept for reference and potential fallback scenarios.
#[deprecated(note = "Use runtime::run_app instead")]
pub async fn run_app(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    mut state: ChatState,
    mut agent_rx: tokio::sync::mpsc::Receiver<StreamMessage>,
    agent_tx: tokio::sync::mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crossterm::event::{self, Event};
    use std::time::Duration;

    // Initial load of configured providers
    let _ = agent_tx.send(AgentRequest::LoadConfiguredProviders).await;

    let mut last_processed_key_time: Option<std::time::Instant> = None;

    let mut last_draw = std::time::Instant::now();
    let mut needs_redraw = true;
    let target_framerate = Duration::from_millis(33); // ~30 FPS

    // Track layout changes for terminal clear (only on task panel toggle)
    let mut last_task_panel_visible = state.task_panel.is_visible;

    loop {
        // Only clear terminal on layout changes (task panel toggle), not on scroll.
        // Scroll changes are handled by the buffer reset in render_page which clears
        // every cell before writing, avoiding the need for a full terminal clear.
        // This eliminates the main source of screen flickering.
        let task_panel_changed = state.task_panel.is_visible != last_task_panel_visible;
        if task_panel_changed {
            terminal.clear()?;
        }
        last_task_panel_visible = state.task_panel.is_visible;

        // Only draw if needed or enough time passed (to prevent freezing if needs_redraw logic is flawed)
        if needs_redraw || last_draw.elapsed() > target_framerate {
            terminal.draw(|f| draw_ui(f, &mut state))?;
            needs_redraw = false;
            last_draw = std::time::Instant::now();
        }

        // Calculate how long to wait for input
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(last_draw);
        let timeout = target_framerate
            .checked_sub(elapsed)
            .unwrap_or(Duration::from_millis(0));

        if crossterm::event::poll(timeout)? {
            // Handle the first event
            match event::read()? {
                Event::Key(key) => {
                    if let Err(_e) = crate::ui::events::input::handle_key_event(
                        &mut state,
                        key,
                        &agent_tx,
                        last_processed_key_time,
                    )
                    .await
                    {
                        state.chat_history.push(
                            crate::types::ChatEntry::assistant(format!(
                                "Error handling key event: {}",
                                _e
                            ))
                            .with_streaming(false),
                        );
                    }
                    last_processed_key_time = Some(std::time::Instant::now());
                    needs_redraw = true;
                }
                Event::Mouse(mouse) => {
                    // 鼠标捕获开启时 Moved（悬停）事件高频产生，
                    // 不处理也不触发重绘，避免重绘风暴
                    if !matches!(mouse.kind, crossterm::event::MouseEventKind::Moved) {
                        crate::ui::events::mouse::handle_mouse_event(&mut state, mouse);
                        needs_redraw = true;
                    }
                }
                Event::Resize(width, height) => {
                    // Force cache clear on resize.
                    // Full frame clearing happens via the ratatui render cycle below;
                    // the key fix for CJK artifacts is the Unicode width tracking in render_page.
                    state.last_chat_height = height.saturating_sub(4);
                    state.last_terminal_width = width;
                    state.clear_cache();
                    needs_redraw = true;
                }
                _ => {}
            }

            // Try to consume pending events (e.g. paste or fast scroll)
            // Increased limit to 50ms to handle fast scroll wheels better
            let start = std::time::Instant::now();
            let mut count = 0;
            while count < 50
                && start.elapsed() < Duration::from_millis(15)
                && crossterm::event::poll(Duration::from_millis(0))?
            {
                match event::read()? {
                    Event::Key(key) => {
                        if let Err(_e) = crate::ui::events::input::handle_key_event(
                            &mut state,
                            key,
                            &agent_tx,
                            last_processed_key_time,
                        )
                        .await
                        {
                            // Log error
                        }
                        last_processed_key_time = Some(std::time::Instant::now());
                        needs_redraw = true;
                    }
                    Event::Mouse(mouse) => {
                        if !matches!(mouse.kind, crossterm::event::MouseEventKind::Moved) {
                            crate::ui::events::mouse::handle_mouse_event(&mut state, mouse);
                            needs_redraw = true;
                        }
                    }
                    Event::Resize(width, height) => {
                        state.last_chat_height = height.saturating_sub(4);
                        state.last_terminal_width = width;
                        state.clear_cache();
                        needs_redraw = true;
                    }
                    _ => {}
                }
                count += 1;
            }
        }

        // Process agent messages
        let mut msg_processed = false;
        // Limit message processing per frame to avoid blocking input for too long
        let mut msg_count = 0;
        while let Ok(msg) = agent_rx.try_recv() {
            crate::ui::services::stream::handle_stream_update(&mut state, msg, &agent_tx).await?;
            msg_processed = true;
            msg_count += 1;
            if msg_count > 50 {
                break;
            }
        }

        if msg_processed {
            needs_redraw = true;
        }
    }
}
