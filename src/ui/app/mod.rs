pub mod early_input;
pub mod logic;
pub mod runtime;

use crate::ui::state::ChatState;
use ratatui::prelude::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;

/// 渲染算出来、需要写回 [`ChatState`] 的布局结果。
///
/// 渲染函数对状态的写入集中在这里，由 [`draw_ui`] 在渲染结束后统一回写。此前这些
/// 赋值（滚动夹取、总行数、各区域 rect）散在 `render_page` 内部，副作用藏在绘制
/// 过程中：想回答「这一帧改了什么」只能通读整个函数。
///
/// **渲染还不是纯函数**：`render_chat_lines` 与 `render_task_panel_mut` 需要
/// `&mut ChatState`（前者写入 `rendered_cache` 记忆化，后者要动 ratatui 的
/// `ListState`）。彻底纯化要先把这些缓存挪出 state，不在本次范围内。
#[derive(Debug, Clone, Copy)]
pub struct RenderOutput {
    /// 夹取后的滚动位置。
    pub scroll: usize,
    /// 对话区渲染出来的总行数（滚动条与 PageUp/PageDown 靠它）。
    pub total_rendered_lines: usize,
    /// 对话区矩形：滚轮回溯与鼠标命中测试都按它换算。
    pub chat_area: Rect,
    /// 输入框矩形：点击聚焦输入框靠它。
    pub input_area: Rect,
}

fn render_page(f: &mut ratatui::Frame<'_>, state: &mut ChatState, viewport: Rect) -> RenderOutput {
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

    // 任务面板的显隐/定时器**不在渲染里**：那是状态推进。此前 `reload_if_stale()`
    // 每帧一次 `current_dir()` + `fs::metadata`（流式时 30 次/秒），显隐判定又在
    // 绘制过程中改 `is_visible`，副作用会被 dirty 门控放大成「画没画过不一样」。
    // 现在由事件循环的 1Hz 节拍调用 `ChatState::tick_task_panel` 推进。

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
    let scroll = if state.auto_follow || state.scroll > max_scroll {
        max_scroll
    } else {
        state.scroll
    };

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
    //
    // 用本帧算出来的 `scroll`，不是 `state.scroll`：后者要到 `draw_ui` 渲染完之后
    // 才回写（`auto_follow` 时每帧都变），读它等于用上一帧的位置画这一帧。
    crate::ui::utils::render::write_lines_to_buffer(
        f.buffer_mut(),
        chat_area,
        &chat_lines,
        scroll,
    );

    // Render token warning (context window usage)
    if !token_warning_lines.is_empty() {
        f.render_widget(
            ratatui::widgets::Paragraph::new(token_warning_lines),
            token_warning_area,
        );
    }

    // Render spinner between chat and footer (always render to prevent layout jumps)
    let spinner_line =
        crate::ui::components::status_line::processing_spinner_line(state, spinner_area.width);
    f.render_widget(ratatui::widgets::Paragraph::new(spinner_line), spinner_area);

    // Render task panel above input (like openclaude's TaskListV2 above PromptInput)
    if task_panel_visible {
        let theme = state.theme_manager.current();
        crate::ui::components::task_panel::render_task_panel_mut(
            f,
            task_area,
            &mut state.task_panel,
            theme,
            state.animation_tick,
            // is_processing 在 agent 收到 Done 时回落 false（stream.rs），转圈跟着停
            state.is_processing,
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

    RenderOutput {
        scroll,
        total_rendered_lines: total,
        chat_area,
        input_area,
    }
}

pub fn draw_ui(f: &mut ratatui::Frame<'_>, state: &mut ChatState) {
    // 注意：**不要**在这里自增 `animation_tick`。
    //
    // 此前 draw_ui 每次调用都 `animation_tick += 1`，导致帧与帧之间 buffer 必然不同，
    // ratatui 的 diff 永远非空 —— 于是「内容没变就不重绘」在物理上不可能成立，
    // 空屏也要 30fps 全量刷屏。动画时钟改由事件循环按需推进，见
    // `super::runtime` 里的 `is_animating` / `advance_animation`。
    let out = render_page(f, state, f.area());

    // 渲染结果统一回写：滚动夹取与各区域 rect 属于事件层（滚动计算、鼠标命中测试
    // 都在渲染之外读它们），由这里一处落库，渲染函数内部不再直接改状态。
    state.scroll = out.scroll;
    state.total_rendered_lines = out.total_rendered_lines;
    state.last_chat_area = Some(out.chat_area);
    state.last_chat_height = out.chat_area.height;
    state.last_input_area = Some(out.input_area);

    let input_area = out.input_area;
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

    // Palette 是唯一可以在「不在栈顶」时也要画的弹窗：输入框会叠在它上面
    // （见 `exit_input_modal` 用 retain 而不是 pop）。先画它，后画的栈顶弹窗
    // 盖在上面 —— 层级与模态栈顺序一致。
    if state.is_palette_open() {
        crate::ui::components::palette::render_palette(f, f.area(), state);
    }

    // 其余弹窗按栈顶单点分发。**这张表必须穷尽 [`Modal`]**：此前是 14 个
    // `if matches!(state.top_modal(), …)`，漏写一个变体编译照过、按键照落主输入框。
    // 先 clone 再 match —— 渲染函数要 `&mut state`，`top_modal()` 只借不借出。
    if let Some(modal) = state.top_modal().cloned() {
        use crate::ui::state::modal::Modal;
        match modal {
            Modal::Palette => { /* 上面已画（栈里即可见） */ }
            Modal::Mcp { .. } => {
                crate::ui::components::modal::render_mcp_modal(f, f.area(), state);
            }
            Modal::Market { .. } => {
                crate::ui::components::modal::render_market_modal(f, f.area(), state);
            }
            Modal::Plugins { .. } => {
                crate::ui::components::modal::render_plugins_modal(f, f.area(), state);
            }
            // "新增 provider" 表单复用 InputModal 的开关，但画的是多字段面板
            Modal::InputModal => {
                if crate::ui::components::provider_form::is_provider_form_active(state) {
                    crate::ui::components::provider_form::render_provider_form(f, f.area(), state);
                } else {
                    crate::ui::components::input_modal::render_input_modal(f, f.area(), state);
                }
            }
            Modal::StatusModal => {
                crate::ui::components::status_modal::render_status_modal(f, f.area(), state);
            }
            Modal::GlobalSearch => crate::ui::components::highlight::search::render_global_search(
                f,
                &state.global_search_state,
                f.area(),
            ),
            Modal::QuickOpen => crate::ui::components::highlight::quick_open::render_quick_open(
                f,
                &state.quick_open_state,
                f.area(),
            ),
            Modal::HistorySearch => {
                crate::ui::components::highlight::history::render_history_search(
                    f,
                    &state.history_search_state,
                    f.area(),
                )
            }
            Modal::ThemePicker { .. } => {
                let themes = crate::ui::components::highlight::theme_picker::available_themes();
                crate::ui::components::highlight::theme_picker::render_theme_picker(
                    f,
                    &themes,
                    state.selected_theme_index,
                    f.area(),
                );
            }
            Modal::UsageStats => {
                let theme = state.theme_manager.current();
                crate::ui::components::highlight::stats::render_usage_stats(
                    f,
                    &state.usage_stats,
                    f.area(),
                    theme,
                );
            }
            Modal::Export => crate::ui::components::highlight::export::render_export_dialog(
                f,
                &state.export_state,
                f.area(),
            ),
            Modal::CompressionStatus => {
                crate::ui::components::highlight::compression::render_compression_status(
                    f,
                    &state.compression_state,
                    f.area(),
                )
            }
            Modal::ContextViz => {
                crate::ui::components::highlight::context_viz::render_context_visualization(
                    f,
                    &state.context_breakdown,
                    f.area(),
                )
            }
            Modal::ErrorOverlay => {
                let theme = state.theme_manager.current();
                crate::ui::components::error_overlay::render_error_overlay(
                    f,
                    &state.error_overlay_state,
                    f.area(),
                    theme,
                );
            }
            Modal::LogSelector => {
                let theme = state.theme_manager.current();
                crate::ui::components::log_selector::render_log_selector(
                    f,
                    &state.log_selector_state,
                    f.area(),
                    theme,
                );
            }
        }
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
                    // 不处理也不触发重绘，避免重绘风暴。
                    // 但拖选进行中的 Moved 必须放行：不支持 SGR 鼠标模式
                    // （?1006h）的终端把「按住左键移动」编码成 button 3 + motion，
                    // crossterm 解成 Moved 而非 Drag(Left)，一律丢弃会导致这类
                    // 终端上左键拖选完全失效。
                    let is_move = matches!(mouse.kind, crossterm::event::MouseEventKind::Moved);
                    if !is_move || state.text_selection.is_selecting {
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
                        let is_move = matches!(mouse.kind, crossterm::event::MouseEventKind::Moved);
                        if !is_move || state.text_selection.is_selecting {
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
