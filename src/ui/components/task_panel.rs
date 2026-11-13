use crate::core::tasks::manager::TaskManager;
use crate::core::tasks::models::{TaskNode, TaskPriority, TaskStatus};
use chrono::Utc;
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
    /// 上次装载数据时文件的 mtime。渲染每帧比对一次，发现文件更新就自动 reload。
    pub last_loaded_mtime: Option<std::time::SystemTime>,
    /// 项目级 active 目标（`.star/goals.json`）。面板首段显示 goal 进度条，
    /// `/goal progress` 落盘后由 1Hz tick 的 mtime 自愈刷新。
    pub active_goals: Vec<crate::core::goal_tracking::Goal>,
    pub last_goals_mtime: Option<std::time::SystemTime>,
}

use std::path::PathBuf;

impl TaskPanel {
    pub fn new() -> Self {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = TaskManager::task_file_for_workspace(&workspace);

        let mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        let task_manager =
            TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new());

        let (active_goals, last_goals_mtime) =
            Self::load_active_goals_from(&Self::goals_file_for_workspace(&workspace));

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
            last_loaded_mtime: mtime,
            active_goals,
            last_goals_mtime,
        }
    }

    /// 项目级 goal 文件。与 `commands::parity::goal_manager` 的存储同源
    /// （`.star/goals.json`，`Vec<Goal>`）——命令写一笔，面板读一份。
    pub fn goals_file_for_workspace(workspace: &std::path::Path) -> PathBuf {
        workspace.join(".star").join("goals.json")
    }

    /// 只留 InProgress 的目标：`/goal done` 或 progress≥100 之后 goal 自动从
    /// 面板退场。新建的排前面（与 `/goal list` 一致），文件不存在或解析失败
    /// 返回空列表。
    fn load_active_goals_from(
        path: &std::path::Path,
    ) -> (
        Vec<crate::core::goal_tracking::Goal>,
        Option<std::time::SystemTime>,
    ) {
        use crate::core::goal_tracking::GoalStatus;
        let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        let goals: Vec<crate::core::goal_tracking::Goal> = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut goals: Vec<_> = goals
            .into_iter()
            .filter(|g| g.status == GoalStatus::InProgress)
            .collect();
        goals.sort_by_key(|g| -g.created_at);
        (goals, mtime)
    }

    fn save(&mut self) {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = TaskManager::task_file_for_workspace(&workspace);
        if self.task_manager.save_to_file(&path).is_ok() {
            // 自己写完要记下 mtime，否则下一帧的自愈检查会把刚写的文件
            // 当成"别人更新过"再无谓地 reload 一遍。
            self.last_loaded_mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
        }
    }

    pub fn reload(&mut self) {
        let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = TaskManager::task_file_for_workspace(&workspace);
        if let Ok(manager) = TaskManager::load_from_file(&path) {
            self.task_manager = manager;
            self.last_loaded_mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
        }
    }

    /// 渲染每帧调用：磁盘上的清单比内存里新就 reload。
    ///
    /// `reload()` 只在 `StreamMessage::ToolResult`、`/tasks`、面板按键等少数
    /// 路径上触发；用户在 TodoWrite 执行中途按 Esc 打断、或别的进程改了文件，
    /// 面板没人通知，就会一直显示旧清单（"更新不及时"）。这里按 mtime 自愈，
    /// 一次 `stat` 的开销可以忽略。编辑中不 reload，否则正在改的行会被冲掉。
    pub fn reload_if_stale(&mut self) {
        // goal 段与任务编辑互不相干，编辑锁只挡任务清单，goal 进度照常刷新
        self.reload_goals();
        if self.editing_task_id.is_some() {
            return;
        }
        let path = TaskManager::task_file_for_workspace(
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        let Some(mtime) = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
        else {
            return;
        };
        if self.last_loaded_mtime != Some(mtime) {
            self.reload();
        }
    }

    /// goals.json 的 mtime 自愈：`/goal progress` 直接落盘、不经过任何 UI
    /// 通知路径，靠 1Hz tick 走这里拿到最新进度（与任务清单同一套机制）。
    fn reload_goals(&mut self) {
        let path = Self::goals_file_for_workspace(
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        let Some(mtime) = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
        else {
            // 文件被删掉了：清空面板段落，别让旧进度挂着
            if self.last_goals_mtime.is_some() || !self.active_goals.is_empty() {
                self.active_goals.clear();
                self.last_goals_mtime = None;
            }
            return;
        };
        if self.last_goals_mtime != Some(mtime) {
            let (goals, goal_mtime) = Self::load_active_goals_from(&path);
            self.active_goals = goals;
            self.last_goals_mtime = goal_mtime;
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
                toggle_task_status(task);
            }
            self.save();
        }
    }

    pub fn skip_task(&mut self) {
        if let Some(id) = self.get_selected_task_id() {
            if let Some(task) = self.task_manager.graph.nodes.get_mut(&id) {
                task.status = TaskStatus::Skipped;
                task.updated_at = Utc::now();
            }
            self.save();
        }
    }
}

/// 手动勾选完成/重开：`status` 和 `completed_at` 必须一起动。
///
/// 抽成自由函数是因为 `toggle_status` 末尾的 `save()` 会写 cwd 下的真实
/// `.star/tasks.json`，单元测试走过去就把开发者的清单覆盖了；纯逻辑放这里，
/// 测试不用落盘也能验。
fn toggle_task_status(task: &mut TaskNode) {
    // completed_at 不盖章，面板的 30s TTL（`is_completed_expired`）永远判
    // false，✔ 项赖在清单最前下不去——就是"Todo 有残留"；反向把已完成项
    // 重新打开时不清掉旧章，它又会被 TTL 当已完成项直接从清单里删掉。
    match task.status {
        TaskStatus::Completed => {
            task.status = TaskStatus::Pending;
            task.completed_at = None;
        }
        _ => {
            task.status = TaskStatus::Completed;
            task.completed_at = Some(Utc::now());
        }
    }
    task.updated_at = Utc::now();
}

impl TaskPanel {
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
    ///
    /// `agent_active` = agent 正在跑（`ChatState::is_processing`）。进行中的任务
    /// 只有在 agent 真在跑时才算"活跃"——模型收尾经常不把最后一项标 completed，
    /// 纯按数据状态判断的话这项会让 5 秒计时器永远启动不了，面板一直挂在输入框
    /// 上方不收起。判定标准和面板 spinner 一致（`render_task_panel_mut` 的
    /// `agent_active`），两处都由 turn 生命周期驱动而不是数据状态驱动
    /// （对标 CCB issue #58297 的 v2.1.141 修复）。
    pub fn check_auto_hide(&mut self, agent_active: bool) {
        // active goal 常驻面板：目标循环一跑几小时是常态，5 秒收起计时
        // 不该把 goal 进度条一起收走。手动隐藏不受影响（is_visible=false）。
        let has_active = !self.active_goals.is_empty()
            || self
                .task_manager
                .graph
                .nodes
                .values()
                .any(|n| match n.status {
                    TaskStatus::Pending | TaskStatus::Blocked => true,
                    TaskStatus::InProgress => agent_active,
                    TaskStatus::Completed | TaskStatus::Skipped => false,
                });

        if has_active {
            self.auto_hide_at = None;
        } else if self.is_visible {
            // All tasks done (or the list was cleared entirely — TodoWrite with
            // all-completed wipes the graph): start auto-hide timer.
            // 之前 `!nodes.is_empty()` 的前置条件让"清空后的空清单"永远
            // 触发不了收起，框就一直挂在输入框上方。
            if self.auto_hide_at.is_none() {
                self.auto_hide_at = Some(std::time::Instant::now());
            }
        }

        // Auto-hide after 5 seconds; an empty list collapses immediately
        if let Some(hide_at) = self.auto_hide_at {
            let is_empty = self.task_manager.graph.nodes.is_empty();
            let elapsed_long_enough = hide_at.elapsed() >= std::time::Duration::from_secs(5);
            if is_empty || elapsed_long_enough {
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
        // Flatten logic needed to navigate tree。高亮索引在 goal 行上多转一圈：
        // 可选区间是 [goal 行, goal 行 + flat.len())，前面的偏移属于 goal 段。
        let flat = self.flatten_tasks();
        let offset = self.active_goals.len();
        let selectable = offset + flat.len();
        if selectable == 0 {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= selectable - 1 {
                    offset
                } else {
                    i + 1
                }
            }
            None => offset,
        };
        self.list_state.select(Some(i));
    }

    pub fn previous(&mut self) {
        let flat = self.flatten_tasks();
        let offset = self.active_goals.len();
        let selectable = offset + flat.len();
        if selectable == 0 {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i <= offset {
                    selectable - 1
                } else {
                    i - 1
                }
            }
            None => selectable - 1,
        };
        self.list_state.select(Some(i));
    }

    pub fn get_selected_task_id(&self) -> Option<String> {
        let flat = self.flatten_tasks();
        let offset = self.active_goals.len();
        self.list_state
            .selected()
            .and_then(|i| (i >= offset).then(|| i - offset))
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
        // 根节点按 CCB 优先级重排；子树跟着父节点走，深度序不变
        let root_ids = self.sorted_root_ids();
        for (i, root_id) in root_ids.iter().enumerate() {
            let is_last = i == root_ids.len() - 1;
            // For roots, we start with empty prefix?
            // Or we treat them as children of an invisible root?
            // If we treat them as children, we get "├ " or "└ " at start.
            self.collect_nodes(root_id, "", is_last, &mut result);
        }
        result
    }

    /// 根节点优先级排序，对标 CCB `TaskListV2` 的 prioritized 列表：
    /// 最近完成(≤30s) → 进行中 → 未阻塞 pending → 已阻塞 → 更早完成。
    ///
    /// 只在 `root_ids` 层面重排，子节点跟着父节点走（`collect_nodes` 的深度
    /// 序不变），不然树的缩进结构会被打散。组内保持 root_ids 原序——CCB 用
    /// `byIdAsc`，id 是创建序，和我们的 root_ids 顺序等价，不必再解析 id。
    /// `sort_by_key` 是稳定排序，同优先级的条目不会来回跳。
    fn sorted_root_ids(&self) -> Vec<String> {
        let mut ids = self.task_manager.graph.root_ids.clone();
        ids.sort_by_key(|id| {
            self.task_manager
                .graph
                .nodes
                .get(id)
                .map(|n| self.priority_rank(n))
                .unwrap_or(u8::MAX)
        });
        ids
    }

    fn priority_rank(&self, node: &TaskNode) -> u8 {
        match node.status {
            // 最近完成排最前，让用户看到刚做完的；超过 30s 的落回末尾
            TaskStatus::Completed if !Self::is_completed_expired(node) => 0,
            TaskStatus::InProgress => 1,
            TaskStatus::Pending if !self.has_unresolved_dependency(node) => 2,
            TaskStatus::Pending | TaskStatus::Blocked => 3,
            TaskStatus::Completed | TaskStatus::Skipped => 4,
        }
    }

    /// 是否还有未完成的依赖（对标 CCB `blockedBy.some(id => unresolved.has(id))`）。
    /// 依赖条目不存在（`None`）按未解决算，和行渲染里的 `> blocked by` 判定一致。
    fn has_unresolved_dependency(&self, node: &TaskNode) -> bool {
        node.dependencies.iter().any(|dep_id| {
            self.task_manager
                .graph
                .nodes
                .get(dep_id)
                .map(|n| n.status != TaskStatus::Completed)
                .unwrap_or(true)
        })
    }

    fn has_visible_descendant(&self, id: &str) -> bool {
        if let Some(node) = self.task_manager.graph.nodes.get(id) {
            for child_id in &node.children {
                if let Some(child) = self.task_manager.graph.nodes.get(child_id) {
                    // Skip expired completed tasks
                    if Self::is_completed_expired(child) {
                        continue;
                    }
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

    /// 对标 Claude Code: completed 任务保留 30s 后自动从 UI 清除
    const COMPLETED_TTL_SECS: i64 = 30;

    fn is_completed_expired(node: &TaskNode) -> bool {
        if let Some(completed_at) = node.completed_at {
            let elapsed = chrono::Utc::now().signed_duration_since(completed_at);
            elapsed.num_seconds() > Self::COMPLETED_TTL_SECS
        } else {
            false
        }
    }

    fn collect_nodes<'a>(
        &'a self,
        id: &str,
        prefix: &str,
        is_last: bool,
        result: &mut Vec<(&'a TaskNode, String)>,
    ) {
        if let Some(node) = self.task_manager.graph.nodes.get(id) {
            // 30s TTL: completed/skipped tasks older than 30s are hidden
            if Self::is_completed_expired(node) {
                return;
            }

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

            // 对标 Claude Code TaskListV2：不用 ├/└ 树线，子任务按层级缩进 2 空格
            let _ = is_last;
            let current_prefix = prefix.to_string();

            result.push((node, current_prefix));

            let child_prefix = format!("{}  ", prefix);

            // We need to know which children are actually going to be shown to determine is_last for them
            // This is getting complicated for tree lines.
            // Simplified approach: Iterate all children, collect those that SHOULD show, then render them.

            let visible_children: Vec<&String> = node
                .children
                .iter()
                .filter(|cid| {
                    if let Some(c) = self.task_manager.graph.nodes.get(*cid) {
                        // Skip expired completed tasks
                        if Self::is_completed_expired(c) {
                            return false;
                        }
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
pub fn render_task_panel_mut(
    f: &mut Frame,
    area: Rect,
    panel: &mut TaskPanel,
    theme: &Theme,
    animation_tick: u64,
    agent_active: bool,
) {
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

    // 对标 CCB TaskListV2 standalone 头："<N> tasks (<K> done, <M> in progress, <P> open)"
    // in_progress 计数只在 >0 时出现，和参考实现一致
    let mut total = 0usize;
    let mut completed = 0usize;
    let mut pending = 0usize;
    let mut in_progress = 0usize;
    for node in panel.task_manager.graph.nodes.values() {
        total += 1;
        match node.status {
            TaskStatus::Pending => pending += 1,
            TaskStatus::InProgress => in_progress += 1,
            TaskStatus::Completed => completed += 1,
            // Blocked/Skipped 既未完成也非进行中，清单里也用 pending 的 ▫ 图标，
            // 必须计入 open。否则头部 N > done + in_progress + open，数字对不上
            // （3 项 1 阻塞时显示 "3 tasks (0 done, 0 in progress, 2 open)"）。
            TaskStatus::Blocked | TaskStatus::Skipped => pending += 1,
        }
    }

    // 首段：active goal 的进度条。`/goal progress` 落盘 → 1Hz tick mtime reload
    // → 下一帧条子跟着动；完成后（100% 或 /goal done）自动退场。
    let mut items: Vec<ListItem> = Vec::new();
    for goal in &panel.active_goals {
        let short_goal_id: String = goal.id.chars().take(8).collect();
        // 右半段（进度条 + 百分比 + id）是 goal 段唯一的信息载体，必须先占位：
        // 以前标题不限宽，窄终端下标题把整行吃满，被裁掉的恰好是进度条，
        // goal 行看起来就只剩一段文字。按显示宽度（CJK 占 2 列）截标题。
        let suffix = format!(
            "  [{}] {}% · {}",
            progress_bar(goal.progress, 10),
            goal.progress,
            short_goal_id
        );
        // 边框 2 列 + "🎯 " 3 列 + 选中行高亮符 "> " 2 列
        const FIXED_WIDTH: usize = 2 + 3 + 2;
        let suffix_width = crate::ui::utils::render::display_width(&suffix);
        let title = crate::ui::utils::text::truncate_display(
            &goal.title,
            (area.width as usize).saturating_sub(FIXED_WIDTH + suffix_width),
        );
        items.push(ListItem::new(Line::from(vec![
            Span::styled("🎯 ", Style::default().fg(theme.primary)),
            Span::styled(title, Style::default().fg(theme.primary)),
            Span::styled(suffix, Style::default().fg(theme.subtle)),
        ])));
    }

    let task_items: Vec<ListItem> = if flat_tasks.is_empty() && panel.active_goals.is_empty() {
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
                // 对标 Claude Code TaskListV2::getTaskIcon —— figures.tick / squareSmallFilled / squareSmall
                // Spinner 帧：对标 Claude Code spinner 动画 (dots variant)。只在 agent
                // 真的在跑时转——agent 已经停了还留着 in_progress 项转圈，看起来像
                // 卡住了（模型经常不在收尾时把最后一项标完成）。停着的时候用静止的
                // ●，和状态行的 spinner 同一套字符。
                const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                let spinner_frame = if agent_active {
                    SPINNER_FRAMES[(animation_tick as usize / 6) % SPINNER_FRAMES.len()]
                } else {
                    "●"
                };

                let (status_icon, icon_color) = match node.status {
                    TaskStatus::Pending => ("▫", None),
                    TaskStatus::InProgress => (spinner_frame, Some(theme.primary)),
                    TaskStatus::Completed => ("✔", Some(theme.success)),
                    TaskStatus::Blocked => ("▫", Some(theme.error)),
                    TaskStatus::Skipped => ("▫", Some(theme.inactive)),
                };

                // 对标 TaskItem 文本样式：完成 = 删除线 + 暗色；进行中 = 加粗高亮；阻塞/跳过 = 暗色
                let mut title_style = match node.status {
                    TaskStatus::Pending => Style::default(),
                    TaskStatus::InProgress => Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD),
                    TaskStatus::Completed => Style::default()
                        .fg(theme.inactive)
                        .add_modifier(Modifier::CROSSED_OUT),
                    TaskStatus::Blocked | TaskStatus::Skipped => {
                        Style::default().fg(theme.inactive)
                    }
                };

                // If high priority, show marker
                if node.priority == TaskPriority::High {
                    title_style = title_style.add_modifier(Modifier::BOLD);
                }

                // 进行中的行显示 activeForm（"Running tests"），其余显示 content（祈使句）。
                // 没有 active_form 时回退到 title。
                let label = if node.status == TaskStatus::InProgress {
                    node.active_form.as_deref().unwrap_or(&node.title)
                } else {
                    &node.title
                };

                let icon_style = match icon_color {
                    Some(color) => Style::default().fg(color),
                    None => Style::default(),
                };

                // 构建任务行：前缀 + 图标 + 标题 + 阻塞信息
                let mut spans = vec![
                    Span::styled(prefix.clone(), Style::default()),
                    Span::styled(format!("{} ", status_icon), icon_style),
                    Span::styled(label.to_string(), title_style),
                ];

                // 显示 blocked-by 信息（对标 Claude Code `> blocked by #id1, #id2`）
                if !node.dependencies.is_empty() {
                    let unresolved: Vec<&str> = node
                        .dependencies
                        .iter()
                        .filter(|dep_id| {
                            panel
                                .task_manager
                                .graph
                                .nodes
                                .get(*dep_id)
                                .map(|n| n.status != TaskStatus::Completed)
                                .unwrap_or(true)
                        })
                        .map(|s| s.as_str())
                        .collect();
                    if !unresolved.is_empty() {
                        spans.push(Span::styled(
                            format!(" > blocked by {}", unresolved.join(", ")),
                            Style::default().fg(theme.error),
                        ));
                    }
                }

                ListItem::new(Line::from(spans))
            })
            .collect()
    };
    items.extend(task_items);

    // 对标 CCB TaskListV2 standalone 头：` 5 tasks (2 done, 1 in progress, 2 open) `。
    // 计数描述状态分布，agent 在跑时再跟上当前进行中项的 activeForm
    // （` · Running tests `）描述动作——两者互补，和状态行的 spinner 动词不重复。
    let mut title = if total == 0 {
        // 只有 goal、没有任务时标题仍写 "Tasks"，整个面板看起来像空的 ——
        // 这一屏到底在显示什么就没人知道了。换成 goal 计数。
        let goal_count = panel.active_goals.len();
        if goal_count > 0 {
            format!(
                " {} goal{} ",
                goal_count,
                if goal_count == 1 { "" } else { "s" }
            )
        } else {
            " Tasks ".to_string()
        }
    } else {
        let mut parts = vec![format!("{} done", completed)];
        if in_progress > 0 {
            parts.push(format!("{} in progress", in_progress));
        }
        parts.push(format!("{} open", pending));
        format!(
            " {} task{} ({}) ",
            total,
            if total == 1 { "" } else { "s" },
            parts.join(", ")
        )
    };
    if agent_active {
        if let Some(activity) = current_activity(panel) {
            // trim_end 去掉了标题末尾的空格，分隔符前要自己补一个，
            // 否则渲染成 `... 2 open)· Running tests`。
            title = format!("{} · {} ", title.trim_end(), activity);
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .title_style(Style::default().fg(theme.primary))
        .border_style(Style::default().fg(theme.border));

    // Find next task hint (对标 Claude Code "Next task" 提示)
    let next_task_hint = find_next_task_hint(panel);

    // 提示画在最后一行内边距上，只在那一行确实空着时才画。
    // 清单撑满面板时（任务数超过 8 行 cap 最常见）它会跟列表最后一行抢位置，
    // 把一条任务盖掉——用户看到的是「任务少了一条」。宁可这次不提示。
    // 注意不能靠收窄列表高度来让行：那样列表的下边框会上移一行，
    // 提示反而画到下边框上（`╰→ Next: ...────╯`）。
    let inner_rows = area.height.saturating_sub(2) as usize;
    let items_len = items.len();
    let hint = next_task_hint.filter(|_| area.height > 3 && items_len < inner_rows);

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, area, &mut panel.list_state);

    // Render next task hint at the bottom if available
    if let Some(hint) = hint {
        if area.height > 3 {
            let hint_area = Rect {
                x: area.x + 1,
                y: area.y + area.height - 2,
                width: area.width.saturating_sub(2),
                height: 1,
            };
            let hint_line = Line::from(vec![
                Span::styled("→ ", Style::default().fg(theme.primary)),
                Span::styled(hint, Style::default().fg(theme.subtle)),
            ]);
            f.render_widget(ratatui::widgets::Paragraph::new(hint_line), hint_area);
        }
    }

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

/// 10 格进度条：`███░░░░░░░`。自由函数便于单测 0/50/100/越界的形态。
fn progress_bar(pct: u8, width: usize) -> String {
    let filled = (pct.min(100) as usize) * width / 100;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// 取当前进行中任务的展示文案（activeForm 优先，回退 title），按 `ordered_ids()`
/// 的清单顺序取第一个进行中项——和清单里看到的顺序一致。无进行中项返回 None。
///
/// 标题用它显示「正在干嘛」，逻辑和 `status_line::in_progress_todo_verb` 一样，
/// 但这里不挑语言：清单行本身就原样显示 activeForm，标题跟着同一种语言不会更怪。
fn current_activity(panel: &TaskPanel) -> Option<String> {
    let graph = &panel.task_manager.graph;
    for id in graph.ordered_ids() {
        let node = graph.nodes.get(&id)?;
        if node.status == TaskStatus::InProgress {
            return Some(
                node.active_form
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| node.title.clone()),
            );
        }
    }
    None
}

/// 找到下一个建议执行的任务（对标 Claude Code "Next task" 提示）
///
/// 优先级：
/// 1. 最近完成任务解除阻塞的第一个 pending 任务
/// 2. 第一个 pending 任务
/// 3. 无则返回 None
fn find_next_task_hint(panel: &TaskPanel) -> Option<String> {
    let graph = &panel.task_manager.graph;

    // 找最近完成的任务（有 completed_at 的最新一个）。
    // 这里不能用 `?` 提前返回：没有已完成项时下面的「第一个 pending」回退
    // 就永远走不到，提示会在纯 pending 清单上直接消失。
    let recently_completed = graph
        .nodes
        .values()
        .filter(|n| n.status == TaskStatus::Completed)
        .max_by_key(|n| n.completed_at);

    // 找被这个任务阻塞的 pending 任务
    if let Some(recently_completed) = recently_completed {
        for id in graph.ordered_ids() {
            if let Some(node) = graph.nodes.get(&id) {
                if node.status == TaskStatus::Pending
                    && node.dependencies.contains(&recently_completed.id)
                {
                    return Some(format!("Next: {}", node.title));
                }
            }
        }
    }

    // 如果没有被阻塞的任务，按清单顺序找第一个 pending 任务
    // （`nodes.values()` 是 HashMap 随机序，不能用于挑选"下一个"）
    for id in graph.ordered_ids() {
        if let Some(node) = graph.nodes.get(&id) {
            if node.status == TaskStatus::Pending {
                return Some(format!("Next: {}", node.title));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tasks::models::{TaskNode, TaskStatus};
    use chrono::Utc;

    /// 空的内存面板。`TaskPanel::new()` 会读磁盘上的 `.star/tasks.json`，
    /// 测试不能依赖 cwd（CI 里那个文件可能有内容）。
    fn empty_panel() -> TaskPanel {
        TaskPanel {
            is_visible: true,
            manually_hidden: false,
            list_state: ListState::default(),
            task_manager: TaskManager::new(),
            editing_task_id: None,
            edit_mode: EditMode::Title,
            edit_input: TextArea::default(),
            view_mode: TaskViewMode::All,
            auto_hide_at: None,
            tasks_modified_since_load: false,
            last_loaded_mtime: None,
            active_goals: Vec::new(),
            last_goals_mtime: None,
        }
    }

    fn goal(
        id: &str,
        title: &str,
        status: crate::core::goal_tracking::GoalStatus,
        progress: u8,
    ) -> crate::core::goal_tracking::Goal {
        use crate::core::goal_tracking::{Goal, GoalPriority};
        Goal {
            id: id.to_string(),
            title: title.to_string(),
            description: None,
            status,
            priority: GoalPriority::Medium,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            sub_goals: Vec::new(),
            tags: Vec::new(),
            progress,
            parent_id: None,
            deadline: None,
            milestones: Vec::new(),
        }
    }

    /// 加一个根任务，返回 id
    fn add_root(panel: &mut TaskPanel, node: TaskNode) -> String {
        let id = node.id.clone();
        panel.task_manager.graph.root_ids.push(id.clone());
        panel.task_manager.graph.nodes.insert(id.clone(), node);
        id
    }

    fn root_titles_in_order(panel: &TaskPanel) -> Vec<&str> {
        panel
            .sorted_root_ids()
            .iter()
            .map(|id| {
                panel
                    .task_manager
                    .graph
                    .nodes
                    .get(id)
                    .unwrap()
                    .title
                    .as_str()
            })
            .collect()
    }

    // ── A. 优先级排序 ──────────────────────────────────────────────

    #[test]
    fn sorted_root_ids_groups_by_status() {
        // root_ids 故意乱排，验证输出按优先级分组
        let mut panel = empty_panel();
        let mut old = TaskNode::new("old done".into());
        old.status = TaskStatus::Completed;
        old.completed_at = Some(Utc::now() - chrono::Duration::seconds(31));
        let mut running = TaskNode::new("running".into());
        running.status = TaskStatus::InProgress;
        let pending = TaskNode::new("pending".into());
        let mut blocked = TaskNode::new("blocked".into());
        blocked.status = TaskStatus::Blocked;
        let mut fresh = TaskNode::new("fresh done".into());
        fresh.status = TaskStatus::Completed;
        fresh.completed_at = Some(Utc::now());

        add_root(&mut panel, old);
        add_root(&mut panel, running);
        add_root(&mut panel, pending);
        add_root(&mut panel, blocked);
        add_root(&mut panel, fresh);

        assert_eq!(
            root_titles_in_order(&panel),
            vec!["fresh done", "running", "pending", "blocked", "old done"],
        );
    }

    #[test]
    fn sort_is_stable_within_bucket() {
        // 三个同优先级的 pending，组内顺序不能被打散（≈ CCB 的 byIdAsc）
        let mut panel = empty_panel();
        add_root(&mut panel, TaskNode::new("first".into()));
        add_root(&mut panel, TaskNode::new("second".into()));
        add_root(&mut panel, TaskNode::new("third".into()));

        assert_eq!(
            root_titles_in_order(&panel),
            vec!["first", "second", "third"],
        );
    }

    #[test]
    fn sort_keeps_children_attached() {
        // 父节点排序后子节点仍紧跟父节点，缩进前缀不变
        let mut panel = empty_panel();
        let mut parent = TaskNode::new("parent".into());
        let mut child = TaskNode::new("child".into());
        child.parent_id = Some(parent.id.clone());
        parent.children.push(child.id.clone());
        let mut running = TaskNode::new("running".into());
        running.status = TaskStatus::InProgress;

        let child_id = child.id.clone();
        // running 排在 parent 前面（in_progress=1 < pending=2）
        add_root(&mut panel, parent.clone());
        add_root(&mut panel, running);
        panel.task_manager.graph.nodes.insert(child_id, child);

        assert_eq!(root_titles_in_order(&panel), vec!["running", "parent"]);

        let flat = panel.flatten_tasks();
        let titles: Vec<&str> = flat.iter().map(|(n, _)| n.title.as_str()).collect();
        assert_eq!(titles, vec!["running", "parent", "child"]);
        // 子节点缩进 2 空格，没被排序提到别处去
        assert_eq!(flat[2].1, "  ");
    }

    #[test]
    fn sort_respects_30s_recent_completed_window() {
        // 31s 前完成的落回末尾，1s 前完成的排最前
        let mut panel = empty_panel();
        let mut stale = TaskNode::new("stale done".into());
        stale.status = TaskStatus::Completed;
        stale.completed_at = Some(Utc::now() - chrono::Duration::seconds(31));
        let mut running = TaskNode::new("running".into());
        running.status = TaskStatus::InProgress;
        let mut fresh = TaskNode::new("fresh done".into());
        fresh.status = TaskStatus::Completed;
        fresh.completed_at = Some(Utc::now() - chrono::Duration::seconds(1));

        add_root(&mut panel, stale);
        add_root(&mut panel, running);
        add_root(&mut panel, fresh);

        assert_eq!(
            root_titles_in_order(&panel),
            vec!["fresh done", "running", "stale done"],
        );
    }

    #[test]
    fn blocked_pending_ranks_below_unblocked() {
        // 有未完成依赖的 pending 排在无依赖的 pending 后面
        let mut panel = empty_panel();
        let mut blocker = TaskNode::new("blocker".into());
        blocker.status = TaskStatus::InProgress;
        let mut blocked = TaskNode::new("blocked".into());
        blocked.dependencies.push(blocker.id.clone());
        let unblocked = TaskNode::new("unblocked".into());

        add_root(&mut panel, blocked);
        add_root(&mut panel, unblocked);
        add_root(&mut panel, blocker);

        assert_eq!(
            root_titles_in_order(&panel),
            vec!["blocker", "unblocked", "blocked"],
        );
    }

    // ── B. auto-hide 与 agent 活跃判定对齐 ──────────────────────────

    #[test]
    fn auto_hide_starts_when_agent_idle_with_stuck_in_progress() {
        // 模型收尾没把最后一项标完成：agent 停了，这项不该再让面板挂着
        let mut panel = empty_panel();
        let mut stuck = TaskNode::new("left spinning".into());
        stuck.status = TaskStatus::InProgress;
        add_root(&mut panel, stuck);

        panel.is_visible = true;
        panel.check_auto_hide(false);
        assert!(
            panel.auto_hide_at.is_some(),
            "agent 停了，残留 in_progress 该启动收起计时"
        );
        assert!(panel.is_visible, "5 秒还没到，不该立刻收起");
    }

    #[test]
    fn auto_hide_held_while_agent_running() {
        let mut panel = empty_panel();
        let mut stuck = TaskNode::new("left spinning".into());
        stuck.status = TaskStatus::InProgress;
        add_root(&mut panel, stuck);

        panel.is_visible = true;
        panel.check_auto_hide(true);
        assert!(
            panel.auto_hide_at.is_none(),
            "agent 还在跑，in_progress 是真活跃，不该启动收起计时"
        );
    }

    #[test]
    fn auto_hide_collapses_after_delay_with_stuck_in_progress() {
        let mut panel = empty_panel();
        let mut stuck = TaskNode::new("left spinning".into());
        stuck.status = TaskStatus::InProgress;
        add_root(&mut panel, stuck);

        panel.is_visible = true;
        // 计时器已在 6 秒前启动
        panel.auto_hide_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        panel.check_auto_hide(false);
        assert!(!panel.is_visible, "超时后面板该收起");
        assert!(panel.auto_hide_at.is_none());
    }

    #[test]
    fn pending_blocks_auto_hide_in_both_agent_states() {
        // 未完成的任务无论 agent 是否在跑都算活跃——收起只针对"agent 停了 +
        // 只剩残留 in_progress"这一种情况
        for agent_active in [false, true] {
            let mut panel = empty_panel();
            add_root(&mut panel, TaskNode::new("real work".into()));
            panel.is_visible = true;
            panel.check_auto_hide(agent_active);
            assert!(
                panel.auto_hide_at.is_none(),
                "pending 任务在 agent_active={agent_active} 下都不该收起"
            );
        }
    }

    // ── C. 手动勾选与 TTL 盖章 ──────────────────────────────────────
    // 走自由的 `toggle_task_status` 而不是 `TaskPanel::toggle_status`：
    // 后者末尾 save() 会写 cwd 下的真实 .star/tasks.json，测试不能落盘。

    /// 回归：手动勾完成必须同步盖 `completed_at`。`is_completed_expired` 只认
    /// 这个字段，不盖章的话 ✔ 项永远不过期、永远排在清单最前——就是"Todo
    /// 有残留"。
    #[test]
    fn toggling_to_completed_stamps_completed_at() {
        let mut task = TaskNode::new("run tests".into());
        toggle_task_status(&mut task);
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.completed_at.is_some(), "勾完成要盖章，TTL 才走得动");
    }

    /// 回归：把已完成项重新打开，旧章必须清掉，否则 30s TTL 会把它当
    /// 已完成项从清单里删掉（`collect_nodes` 直接 return）。
    #[test]
    fn toggling_back_to_pending_clears_completed_at() {
        let mut task = TaskNode::new("run tests".into());
        task.status = TaskStatus::Completed;
        task.completed_at = Some(Utc::now() - chrono::Duration::seconds(120));

        toggle_task_status(&mut task);

        assert_eq!(task.status, TaskStatus::Pending);
        assert!(
            task.completed_at.is_none(),
            "重开的项目必须清掉完成时间戳，否则会被 TTL 误删"
        );
    }

    /// 编辑中不 reload，否则正在改的行会被磁盘数据冲掉。
    #[test]
    fn reload_if_stale_is_noop_while_editing() {
        let mut panel = empty_panel();
        panel.editing_task_id = Some("some-id".to_string());
        // 哪怕把 mtime 改成"远比磁盘旧"（下一帧本应触发 reload），编辑中也锁住
        panel.last_loaded_mtime = Some(std::time::UNIX_EPOCH);
        panel.reload_if_stale();
        assert_eq!(
            panel.last_loaded_mtime,
            Some(std::time::UNIX_EPOCH),
            "编辑中不能 reload"
        );
    }

    // ── D. goal 进度段 ─────────────────────────────────────────────

    #[test]
    fn load_active_goals_keeps_inprogress_only_newest_first() {
        use crate::core::goal_tracking::GoalStatus;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join(".star");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("goals.json"),
            serde_json::to_string(&[
                goal("11111111-0000", "新的 50%", GoalStatus::InProgress, 50),
                goal("22222222-0000", "完成的不显示", GoalStatus::Completed, 100),
                goal("33333333-0000", "更早的", GoalStatus::InProgress, 10),
            ])
            .unwrap(),
        )
        .unwrap();

        let (goals, mtime) = TaskPanel::load_active_goals_from(&d.join("goals.json"));
        assert!(mtime.is_some(), "要拿到 mtime 才能做自愈比对");
        assert_eq!(goals.len(), 2, "完成/取消的 goal 不进面板");
        assert_eq!(
            goals[0].title, "新的 50%",
            "新的排前面（与 /goal list 一致）"
        );
        assert_eq!(goals[0].progress, 50);
    }

    #[test]
    fn load_active_goals_survives_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (goals, mtime) =
            TaskPanel::load_active_goals_from(&dir.path().join(".star").join("goals.json"));
        assert!(goals.is_empty());
        assert!(mtime.is_none());
    }

    #[test]
    fn active_goal_holds_the_panel_open() {
        use crate::core::goal_tracking::GoalStatus;
        let mut panel = empty_panel();
        panel.active_goals.push(goal(
            "11111111-0000",
            "测试 goal",
            GoalStatus::InProgress,
            50,
        ));
        panel.is_visible = true;

        panel.check_auto_hide(false);
        assert!(
            panel.auto_hide_at.is_none(),
            "active goal 不该启动收起计时（不然 5 秒后进度条被收走）"
        );
        assert!(panel.is_visible);
    }

    #[test]
    fn selection_offsets_skip_goal_rows() {
        use crate::core::goal_tracking::GoalStatus;
        let mut panel = empty_panel();
        panel.active_goals.push(goal(
            "11111111-0000",
            "测试 goal",
            GoalStatus::InProgress,
            50,
        ));
        // 两个任务：偏移 1 之后才是任务行（TaskNode 的 id 是 UUID，先接住）
        let t1 = TaskNode::new("task one".into());
        let t1_id = t1.id.clone();
        panel.task_manager.graph.root_ids.push(t1_id.clone());
        panel.task_manager.graph.nodes.insert(t1_id.clone(), t1);
        let t2 = TaskNode::new("task two".into());
        let t2_id = t2.id.clone();
        panel.task_manager.graph.root_ids.push(t2_id.clone());
        panel.task_manager.graph.nodes.insert(t2_id.clone(), t2);

        // 索引 0 是 goal 行：不属于任务，返回 None
        panel.list_state.select(Some(0));
        assert_eq!(panel.get_selected_task_id(), None);

        panel.list_state.select(Some(1));
        assert_eq!(
            panel.get_selected_task_id(),
            Some(t1_id.clone()),
            "跳过 goal 偏移索到第一个任务"
        );
        panel.list_state.select(Some(2));
        assert_eq!(
            panel.get_selected_task_id(),
            Some(t2_id),
            "索引 2 → 第二个任务"
        );

        // next 从 None 出发落在第一个任务（跳过 goal 偏移），末尾之后回绕
        panel.list_state.select(None);
        panel.next();
        assert_eq!(panel.list_state.selected(), Some(1), "next → 第一个任务");
        panel.next();
        assert_eq!(panel.list_state.selected(), Some(2));
        panel.next();
        assert_eq!(
            panel.list_state.selected(),
            Some(1),
            "末尾之后回到第一个任务"
        );

        // previous 从 None 出发落在最后一个任务，再往前回到第一个任务
        panel.list_state.select(None);
        panel.previous();
        assert_eq!(
            panel.list_state.selected(),
            Some(2),
            "previous → 最后一个任务"
        );
        panel.previous();
        assert_eq!(panel.list_state.selected(), Some(1), "往前回到第一个任务");
        panel.previous();
        assert_eq!(panel.list_state.selected(), Some(2), "不会越过 goal 行越界");
    }

    // ── E. 实际渲染 ────────────────────────────────────────────────

    /// 布局侧（`ui::app::render_page`）给面板分配的高度，公式照抄。
    fn layout_height(panel: &TaskPanel) -> u16 {
        let task_count = panel.flatten_tasks().len() as u16;
        let goal_count = panel.active_goals.len() as u16;
        (task_count + goal_count + 3).min(8 + goal_count).max(3)
    }

    /// 按布局高度把面板画进 TestBackend，返回逐行纯文本。
    fn render_rows(panel: &mut TaskPanel, width: u16, agent_active: bool) -> Vec<String> {
        use ratatui::{backend::TestBackend, Terminal};
        let height = layout_height(panel);
        let theme = Theme::default_dark();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| {
                render_task_panel_mut(
                    f,
                    Rect::new(0, 0, width, height),
                    panel,
                    &theme,
                    0,
                    agent_active,
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn with_status(title: &str, status: TaskStatus) -> TaskNode {
        let mut node = TaskNode::new(title.into());
        if status == TaskStatus::Completed {
            node.completed_at = Some(Utc::now());
        }
        node.status = status;
        node
    }

    #[test]
    fn render_probe_prints_typical_scenarios() {
        use crate::core::goal_tracking::GoalStatus;

        // 场景 1：3 项混合状态，agent 在跑
        let mut panel = empty_panel();
        add_root(
            &mut panel,
            with_status("Read config", TaskStatus::Completed),
        );
        let mut running = with_status("Run tests", TaskStatus::InProgress);
        running.active_form = Some("Running tests".into());
        add_root(&mut panel, running);
        add_root(&mut panel, with_status("Write report", TaskStatus::Pending));
        println!("--- scenario 1: mixed, agent active ---");
        for row in render_rows(&mut panel, 60, true) {
            println!("{row}");
        }

        // 场景 2：1 完成 + 7 pending，超过 8 行 cap
        let mut panel = empty_panel();
        add_root(
            &mut panel,
            with_status("step 0 done", TaskStatus::Completed),
        );
        for i in 1..=7 {
            add_root(
                &mut panel,
                with_status(&format!("step {i}"), TaskStatus::Pending),
            );
        }
        println!("--- scenario 2: over the 8-row cap ---");
        for row in render_rows(&mut panel, 60, false) {
            println!("{row}");
        }

        // 场景 3：goal + 任务
        let mut panel = empty_panel();
        panel.active_goals.push(goal(
            "11111111-0000",
            "测试下 task面板的显示",
            GoalStatus::InProgress,
            40,
        ));
        add_root(&mut panel, with_status("step a", TaskStatus::Completed));
        add_root(&mut panel, with_status("step b", TaskStatus::Pending));
        println!("--- scenario 3: goal + tasks ---");
        for row in render_rows(&mut panel, 60, false) {
            println!("{row}");
        }

        // 场景 4：只有 goal，没有任务
        let mut panel = empty_panel();
        panel.active_goals.push(goal(
            "11111111-0000",
            "only a goal",
            GoalStatus::InProgress,
            0,
        ));
        println!("--- scenario 4: goal only ---");
        for row in render_rows(&mut panel, 60, false) {
            println!("{row}");
        }

        // 场景 5：空清单
        let mut panel = empty_panel();
        println!("--- scenario 5: empty ---");
        for row in render_rows(&mut panel, 60, false) {
            println!("{row}");
        }

        // 场景 6：窄终端 + 长标题 goal
        let mut panel = empty_panel();
        panel.active_goals.push(goal(
            "11111111-0000",
            "a fairly long goal title that will not fit in a narrow terminal",
            GoalStatus::InProgress,
            40,
        ));
        add_root(&mut panel, with_status("step a", TaskStatus::Pending));
        println!("--- scenario 6: narrow, long goal title ---");
        for row in render_rows(&mut panel, 40, false) {
            println!("{row}");
        }
    }

    /// 头部计数必须自洽：`N` 等于 done + in_progress + open。
    /// Blocked/Skipped 既不完成也不进行中，归入 open——它们和 pending 一样
    /// 用 ▫ 图标，不计入会让头部数字与实际行数对不上。
    #[test]
    fn header_counts_blocked_and_skipped_as_open() {
        let mut panel = empty_panel();
        add_root(&mut panel, with_status("blocked one", TaskStatus::Blocked));
        add_root(&mut panel, with_status("skipped one", TaskStatus::Skipped));
        add_root(&mut panel, with_status("pending one", TaskStatus::Pending));

        let rows = render_rows(&mut panel, 80, false);
        let header = &rows[0];
        // in_progress 为 0 时整段省略（对标实现同样如此）
        assert!(
            header.contains("3 tasks (0 done, 3 open)"),
            "头部应把 blocked/skipped 计入 open，实际 {header:?}"
        );
    }

    #[test]
    fn progress_bar_boundaries() {
        assert_eq!(progress_bar(0, 10), "░░░░░░░░░░");
        assert_eq!(progress_bar(50, 10), "█████░░░░░");
        assert_eq!(progress_bar(100, 10), "██████████");
        assert_eq!(progress_bar(150, 10), "██████████");
    }
}
