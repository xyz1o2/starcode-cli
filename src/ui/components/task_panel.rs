use crate::core::tasks::manager::TaskManager;
use crate::core::tasks::models::{TaskNode, TaskPriority, TaskStatus};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState},
    Frame,
};
use tui_textarea::TextArea;

use crate::ui::themes::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditMode {
    Title,
    Description,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskViewMode {
    All,
    Active,
}

impl std::fmt::Display for TaskViewMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskViewMode::All => write!(f, "All"),
            TaskViewMode::Active => write!(f, "Active"),
        }
    }
}

pub struct TaskPanel {
    pub is_visible: bool,
    pub manually_hidden: bool, // Track if user manually hid the panel
    pub list_state: ListState,
    pub task_manager: TaskManager,
    pub editing_task_id: Option<String>,
    pub edit_mode: EditMode,
    pub edit_input: TextArea<'static>,
    pub view_mode: TaskViewMode,
    pub auto_hide_at: Option<std::time::Instant>, // When to auto-hide after all tasks complete
    pub tasks_modified_since_load: bool,          // Track if tasks were modified since startup
}

use std::path::PathBuf;

impl TaskPanel {
    pub fn new() -> Self {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = TaskManager::task_file_for_workspace(&workspace);

        let task_manager =
            TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new());

        Self {
            is_visible: false,
            manually_hidden: false,
            list_state: ListState::default(),
            task_manager,
            editing_task_id: None,
            edit_mode: EditMode::Title,
            edit_input: TextArea::default(),
            view_mode: TaskViewMode::All,
            auto_hide_at: None,
            tasks_modified_since_load: false,
        }
    }

    fn save(&self) {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = TaskManager::task_file_for_workspace(&workspace);
        let _ = self.task_manager.save_to_file(&path);
    }

    pub fn reload(&mut self) {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = TaskManager::task_file_for_workspace(&workspace);
        if let Ok(manager) = TaskManager::load_from_file(&path) {
            self.task_manager = manager;
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editing_task_id.is_some()
    }

    pub fn enter_edit_mode(&mut self, mode: EditMode) {
        if let Some(id) = self.get_selected_task_id() {
            if let Some(task) = self.task_manager.graph.nodes.get(&id) {
                self.editing_task_id = Some(id.clone());
                self.edit_mode = mode.clone();

                let content = match mode {
                    EditMode::Title => vec![task.title.clone()],
                    EditMode::Description => {
                        if let Some(desc) = &task.description {
                            desc.lines().map(|s| s.to_string()).collect()
                        } else {
                            vec![String::new()]
                        }
                    }
                };

                self.edit_input = TextArea::from(content);

                let title = match mode {
                    EditMode::Title => " Edit Task Title ",
                    EditMode::Description => " Edit Task Description (Ctrl+Enter to save) ",
                };

                self.edit_input.set_block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(title)
                        .border_style(Style::default().fg(Color::Yellow)),
                );
                self.edit_input.set_cursor_line_style(Style::default());
            }
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editing_task_id = None;
        self.edit_input = TextArea::default();
    }

    pub fn submit_edit(&mut self) {
        if let Some(id) = &self.editing_task_id {
            match self.edit_mode {
                EditMode::Title => {
                    let new_title = self.edit_input.lines().first().cloned().unwrap_or_default();
                    if !new_title.trim().is_empty() {
                        if let Some(task) = self.task_manager.graph.nodes.get_mut(id) {
                            task.title = new_title;
                        }
                        self.save();
                    }
                }
                EditMode::Description => {
                    let lines = self.edit_input.lines();
                    let new_desc = lines.join("\n");
                    if let Some(task) = self.task_manager.graph.nodes.get_mut(id) {
                        task.description = Some(new_desc);
                    }
                    self.save();
                }
            }
        }
        self.cancel_edit();
    }

    pub fn handle_edit_input(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => {
                // If description, Enter adds new line unless Ctrl is pressed
                match self.edit_mode {
                    EditMode::Title => {
                        self.submit_edit();
                        true
                    }
                    EditMode::Description => {
                        if key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            self.submit_edit();
                            true
                        } else {
                            self.edit_input.input(key);
                            true
                        }
                    }
                }
            }
            KeyCode::Esc => {
                self.cancel_edit();
                true
            }
            _ => {
                self.edit_input.input(key);
                true
            }
        }
    }

    pub fn toggle_priority(&mut self) {
        if let Some(id) = self.get_selected_task_id() {
            if let Some(task) = self.task_manager.graph.nodes.get_mut(&id) {
                task.priority = match task.priority {
                    TaskPriority::Low => TaskPriority::Medium,
                    TaskPriority::Medium => TaskPriority::High,
                    TaskPriority::High => TaskPriority::Low,
                };
            }
            self.save();
        }
    }

    pub fn toggle_status(&mut self) {
        if let Some(id) = self.get_selected_task_id() {
            if let Some(task) = self.task_manager.graph.nodes.get_mut(&id) {
                task.status = match task.status {
                    TaskStatus::Completed => TaskStatus::Pending,
                    _ => TaskStatus::Completed,
                };
            }
            self.save();
        }
    }

    pub fn skip_task(&mut self) {
        if let Some(id) = self.get_selected_task_id() {
            if let Some(task) = self.task_manager.graph.nodes.get_mut(&id) {
                task.status = TaskStatus::Skipped;
            }
            self.save();
        }
    }

    pub fn toggle_visibility(&mut self) {
        self.is_visible = !self.is_visible;
        self.manually_hidden = !self.is_visible;
        // Also cancel edit if hiding
        if !self.is_visible {
            self.cancel_edit();
            // Reset selection when hiding to avoid stale state
            self.list_state.select(None);
        } else {
            // Reset selection when showing to avoid stale index
            self.list_state.select(None);
        }
    }

    /// Auto-show panel when tasks are added (unless user manually hid it)
    /// Matches Claude Code behavior: todo list appears automatically when TodoWrite is called
    /// Only shows when tasks have been modified since startup (not on initial load)
    pub fn auto_show_if_needed(&mut self) {
        // Only auto-show if tasks were modified since startup (e.g., via TodoWrite)
        if !self.is_visible && !self.manually_hidden && self.tasks_modified_since_load {
            let has_active = self
                .task_manager
                .graph
                .nodes
                .values()
                .any(|n| matches!(n.status, TaskStatus::Pending | TaskStatus::InProgress));
            if has_active {
                self.is_visible = true;
            }
        }
        // Reset manually_hidden when all tasks are complete
        if self.manually_hidden {
            let has_active = self
                .task_manager
                .graph
                .nodes
                .values()
                .any(|n| matches!(n.status, TaskStatus::Pending | TaskStatus::InProgress));
            if !has_active {
                self.manually_hidden = false;
            }
        }
    }

    /// Mark tasks as modified (called when TodoWrite tool is executed)
    pub fn mark_modified(&mut self) {
        self.tasks_modified_since_load = true;
    }

    /// Check if we should auto-hide (all tasks completed, 5s delay)
    pub fn check_auto_hide(&mut self) {
        let has_active = self.task_manager.graph.nodes.values().any(|n| {
            matches!(
                n.status,
                TaskStatus::Pending | TaskStatus::InProgress | TaskStatus::Blocked
            )
        });

        if has_active {
            self.auto_hide_at = None;
        } else if !self.task_manager.graph.nodes.is_empty() && self.is_visible {
            // All tasks done - start auto-hide timer
            if self.auto_hide_at.is_none() {
                self.auto_hide_at = Some(std::time::Instant::now());
            }
        }

        // Auto-hide after 5 seconds
        if let Some(hide_at) = self.auto_hide_at {
            if hide_at.elapsed() >= std::time::Duration::from_secs(5) {
                self.is_visible = false;
                self.auto_hide_at = None;
            }
        }
    }

    /// Get compact task summary for status line
    pub fn get_summary(&self) -> Option<String> {
        let total = self.task_manager.graph.nodes.len();
        if total == 0 {
            return None;
        }

        let completed = self
            .task_manager
            .graph
            .nodes
            .values()
            .filter(|n| n.status == TaskStatus::Completed)
            .count();
        let in_progress = self
            .task_manager
            .graph
            .nodes
            .values()
            .filter(|n| n.status == TaskStatus::InProgress)
            .count();
        let pending = self
            .task_manager
            .graph
            .nodes
            .values()
            .filter(|n| n.status == TaskStatus::Pending)
            .count();

        if in_progress > 0 {
            Some(format!(
                "{} tasks ({} active, {} done)",
                total, in_progress, completed
            ))
        } else if pending > 0 {
            Some(format!(
                "{} tasks ({} pending, {} done)",
                total, pending, completed
            ))
        } else {
            Some(format!("{} tasks (all done)", total))
        }
    }

    pub fn cycle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            TaskViewMode::All => TaskViewMode::Active,
            TaskViewMode::Active => TaskViewMode::All,
        };
        // Reset selection to avoid out of bounds
        self.list_state.select(Some(0));
    }

    pub fn next(&mut self) {
        // Flatten logic needed to navigate tree
        let flat = self.flatten_tasks();
        if flat.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= flat.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn previous(&mut self) {
        let flat = self.flatten_tasks();
        if flat.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    flat.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn get_selected_task_id(&self) -> Option<String> {
        let flat = self.flatten_tasks();
        self.list_state
            .selected()
            .and_then(|i| flat.get(i).map(|(node, _)| node.id.clone()))
    }

    pub fn move_up(&mut self) {
        let id = match self.get_selected_task_id() {
            Some(i) => i,
            None => return,
        };
        let (parent_id, after_id) = {
            let graph = &self.task_manager.graph;
            let node = match graph.nodes.get(&id) {
                Some(n) => n,
                None => return,
            };
            let siblings = if let Some(pid) = &node.parent_id {
                match graph.nodes.get(pid) {
                    Some(p) => &p.children,
                    None => return,
                }
            } else {
                &graph.root_ids
            };

            let idx = match siblings.iter().position(|x| x == &id) {
                Some(i) => i,
                None => return,
            };
            if idx == 0 {
                return;
            } // Can't move up

            let target_after = if idx == 1 {
                None
            } else {
                Some(siblings[idx - 2].clone())
            };
            (node.parent_id.clone(), target_after)
        };

        let _ = self.task_manager.move_task(&id, parent_id, after_id);
        self.save();
    }

    pub fn move_down(&mut self) {
        let id = match self.get_selected_task_id() {
            Some(i) => i,
            None => return,
        };
        let (parent_id, after_id) = {
            let graph = &self.task_manager.graph;
            let node = match graph.nodes.get(&id) {
                Some(n) => n,
                None => return,
            };
            let siblings = if let Some(pid) = &node.parent_id {
                match graph.nodes.get(pid) {
                    Some(p) => &p.children,
                    None => return,
                }
            } else {
                &graph.root_ids
            };

            let idx = match siblings.iter().position(|x| x == &id) {
                Some(i) => i,
                None => return,
            };
            if idx >= siblings.len() - 1 {
                return;
            } // Can't move down

            let target_after = Some(siblings[idx + 1].clone());
            (node.parent_id.clone(), target_after)
        };

        let _ = self.task_manager.move_task(&id, parent_id, after_id);
        self.save();
    }

    pub fn indent(&mut self) {
        let id = match self.get_selected_task_id() {
            Some(i) => i,
            None => return,
        };
        let (new_parent_id, after_id) = {
            let graph = &self.task_manager.graph;
            let node = match graph.nodes.get(&id) {
                Some(n) => n,
                None => return,
            };
            let siblings = if let Some(pid) = &node.parent_id {
                match graph.nodes.get(pid) {
                    Some(p) => &p.children,
                    None => return,
                }
            } else {
                &graph.root_ids
            };

            let idx = match siblings.iter().position(|x| x == &id) {
                Some(i) => i,
                None => return,
            };
            if idx == 0 {
                return;
            } // No sibling above to become parent

            let new_parent_id = siblings[idx - 1].clone();

            // We append to the new parent's children
            let new_parent = match graph.nodes.get(&new_parent_id) {
                Some(n) => n,
                None => return,
            };
            let target_after = new_parent.children.last().cloned();

            (Some(new_parent_id), target_after)
        };

        let _ = self.task_manager.move_task(&id, new_parent_id, after_id);
        self.save();
    }

    pub fn outdent(&mut self) {
        let id = match self.get_selected_task_id() {
            Some(i) => i,
            None => return,
        };
        let (new_parent_id, after_id) = {
            let graph = &self.task_manager.graph;
            let node = match graph.nodes.get(&id) {
                Some(n) => n,
                None => return,
            };

            // If no parent, can't outdent (already root)
            let current_parent_id = match &node.parent_id {
                Some(pid) => pid,
                None => return,
            };

            let current_parent = match graph.nodes.get(current_parent_id) {
                Some(n) => n,
                None => return,
            };

            // New parent is grandparent
            let grandparent_id = current_parent.parent_id.clone();

            // We want to be a sibling AFTER our current parent
            let target_after = Some(current_parent_id.clone());

            (grandparent_id, target_after)
        };

        let _ = self.task_manager.move_task(&id, new_parent_id, after_id);
        self.save();
    }

    pub fn delete_selected(&mut self) {
        if let Some(id) = self.get_selected_task_id() {
            let _ = self.task_manager.delete_task(&id);
            // Adjust selection to previous item
            self.previous();
            self.save();
        }
    }

    pub fn add_new_task(&mut self) {
        let (parent_id, after_id) = if let Some(id) = self.get_selected_task_id() {
            let node = self.task_manager.graph.nodes.get(&id);
            (node.and_then(|n| n.parent_id.clone()), Some(id))
        } else {
            (None, None) // Add to root start if nothing selected? Or end?
        };

        let mut new_task = TaskNode::new("New Task".to_string());
        new_task.parent_id = parent_id.clone();

        let new_id = new_task.id.clone();

        if let Ok(_) = self.task_manager.add_task(new_task) {
            // If we wanted it at a specific position
            if let Some(after) = after_id {
                let _ = self.task_manager.move_task(&new_id, parent_id, Some(after));
            } else if parent_id.is_none() {
                // If adding to root and nothing selected, maybe add to end (default)?
                // Or if we want it at start, use move_task(id, None, None).
            }

            // Select the new task?
            // Need to find where it ended up.
            // For now user can navigate to it.
            self.save();
        }
    }

    // Helper to flatten tree for display (DFS)
    pub fn flatten_tasks(&self) -> Vec<(&TaskNode, String)> {
        let mut result = Vec::new();
        let root_ids = &self.task_manager.graph.root_ids;
        for (i, root_id) in root_ids.iter().enumerate() {
            let is_last = i == root_ids.len() - 1;
            // For roots, we start with empty prefix?
            // Or we treat them as children of an invisible root?
            // If we treat them as children, we get "├ " or "└ " at start.
            self.collect_nodes(root_id, "", is_last, &mut result);
        }
        result
    }

    fn has_visible_descendant(&self, id: &str) -> bool {
        if let Some(node) = self.task_manager.graph.nodes.get(id) {
            for child_id in &node.children {
                if let Some(child) = self.task_manager.graph.nodes.get(child_id) {
                    // Check child itself
                    let is_child_visible = match self.view_mode {
                        TaskViewMode::All => true,
                        TaskViewMode::Active => matches!(
                            child.status,
                            TaskStatus::Pending | TaskStatus::InProgress | TaskStatus::Blocked
                        ),
                    };
                    if is_child_visible {
                        return true;
                    }
                    // Recursively check
                    if self.has_visible_descendant(child_id) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn collect_nodes<'a>(
        &'a self,
        id: &str,
        prefix: &str,
        is_last: bool,
        result: &mut Vec<(&'a TaskNode, String)>,
    ) {
        if let Some(node) = self.task_manager.graph.nodes.get(id) {
            // Visibility Check
            let is_self_visible = match self.view_mode {
                TaskViewMode::All => true,
                TaskViewMode::Active => matches!(
                    node.status,
                    TaskStatus::Pending | TaskStatus::InProgress | TaskStatus::Blocked
                ),
            };

            let show_node = is_self_visible || self.has_visible_descendant(id);

            if !show_node {
                return;
            }

            let marker = if is_last { "└ " } else { "├ " };
            let current_prefix = format!("{}{}", prefix, marker);

            result.push((node, current_prefix));

            let child_prefix = format!("{}{}", prefix, if is_last { "  " } else { "│ " });

            // We need to know which children are actually going to be shown to determine is_last for them
            // This is getting complicated for tree lines.
            // Simplified approach: Iterate all children, collect those that SHOULD show, then render them.

            let visible_children: Vec<&String> = node
                .children
                .iter()
                .filter(|cid| {
                    if let Some(c) = self.task_manager.graph.nodes.get(*cid) {
                        let c_visible = match self.view_mode {
                            TaskViewMode::All => true,
                            TaskViewMode::Active => matches!(
                                c.status,
                                TaskStatus::Pending | TaskStatus::InProgress | TaskStatus::Blocked
                            ),
                        };
                        c_visible || self.has_visible_descendant(cid)
                    } else {
                        false
                    }
                })
                .collect();

            for (i, child_id) in visible_children.iter().enumerate() {
                let is_last_child = i == visible_children.len() - 1;
                self.collect_nodes(child_id, &child_prefix, is_last_child, result);
            }
        }
    }
}

// Revised signature for rendering with mutable state
pub fn render_task_panel_mut(f: &mut Frame, area: Rect, panel: &mut TaskPanel, theme: &Theme) {
    if !panel.is_visible {
        return;
    }

    // Safety: skip rendering if area is too small
    if area.height < 3 || area.width < 4 {
        return;
    }

    // Clear the area to prevent ghosting from underlying content
    f.render_widget(Clear, area);

    let flat_tasks = panel.flatten_tasks();

    let mut total = 0usize;
    let mut completed = 0usize;
    let mut pending = 0usize;
    let mut in_progress = 0usize;
    let mut blocked = 0usize;

    for node in panel.task_manager.graph.nodes.values() {
        total += 1;
        match node.status {
            TaskStatus::Pending => pending += 1,
            TaskStatus::InProgress => in_progress += 1,
            TaskStatus::Completed => completed += 1,
            TaskStatus::Blocked => blocked += 1,
            TaskStatus::Skipped => {}
        }
    }

    let active = pending + in_progress + blocked;

    let items: Vec<ListItem> = if flat_tasks.is_empty() {
        let label = if panel.view_mode == TaskViewMode::Active {
            "No active tasks. Use Ctrl+N to add or /task add."
        } else {
            "No tasks. Use Ctrl+N to add or /task add."
        };
        vec![ListItem::new(Line::from(Span::styled(
            label.to_string(),
            Style::default().fg(theme.subtle),
        )))]
    } else {
        flat_tasks
            .iter()
            .map(|(node, prefix)| {
                let status_icon = match node.status {
                    TaskStatus::Pending => "☐",
                    TaskStatus::InProgress => "▶",
                    TaskStatus::Completed => "✓",
                    TaskStatus::Blocked => "!",
                    TaskStatus::Skipped => "-",
                };

                let status_style = match node.status {
                    TaskStatus::Pending => Style::default().fg(theme.secondary),
                    TaskStatus::InProgress => Style::default()
                        .fg(theme.warning)
                        .add_modifier(Modifier::BOLD),
                    TaskStatus::Completed => Style::default().fg(theme.success),
                    TaskStatus::Blocked => Style::default().fg(theme.error),
                    TaskStatus::Skipped => Style::default().fg(theme.inactive),
                };

                // If high priority, show marker
                let title_style = if node.priority == TaskPriority::High {
                    status_style.add_modifier(Modifier::BOLD)
                } else {
                    status_style
                };

                let content = format!("{}{} {}", prefix, status_icon, node.title);

                ListItem::new(Line::from(Span::styled(content, title_style)))
            })
            .collect()
    };

    let title = if total == 0 {
        " Tasks ".to_string()
    } else {
        format!(" Tasks {}/{} ", completed, total)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .title_style(Style::default().fg(theme.primary))
        .border_style(Style::default().fg(theme.border));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, area, &mut panel.list_state);

    // Render Edit Input Overlay
    if panel.is_editing() {
        let popup_area = match panel.edit_mode {
            EditMode::Title => {
                if area.height > 6 {
                    Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(0), Constraint::Length(3)])
                        .split(area)[1]
                } else {
                    area
                }
            }
            EditMode::Description => {
                let v_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(20),
                        Constraint::Percentage(60),
                        Constraint::Percentage(20),
                    ])
                    .split(area);
                v_layout[1]
            }
        };

        f.render_widget(Clear, popup_area);
        f.render_widget(&panel.edit_input, popup_area);
    }
}
