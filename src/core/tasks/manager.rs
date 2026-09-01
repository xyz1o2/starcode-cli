use crate::core::tasks::models::{TaskChangeEvent, TaskGraph, TaskNode, TaskNotifier, TaskStatus};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct TaskManager {
    pub graph: TaskGraph,
    pub notifier: Arc<TaskNotifier>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            graph: TaskGraph::new(),
            notifier: Arc::new(TaskNotifier::new()),
        }
    }

    /// 获取任务变更通知器
    pub fn get_notifier(&self) -> Arc<TaskNotifier> {
        self.notifier.clone()
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut graph = self.graph.clone();
        repair_task_graph(&mut graph);
        let json = serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut graph: TaskGraph = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        repair_task_graph(&mut graph);
        Ok(Self {
            graph,
            notifier: Arc::new(TaskNotifier::new()),
        })
    }

    pub fn task_file_for_workspace(workspace: &Path) -> PathBuf {
        workspace.join(".star").join("tasks.json")
    }

    /// 获取基于会话ID隔离的任务文件路径
    pub fn task_file_for_session(workspace: &Path, session_id: &str) -> PathBuf {
        let sanitized_session = sanitize_path_component(session_id);
        workspace
            .join(".star")
            .join(format!("tasks_{}.json", sanitized_session))
    }

    /// 获取基于团队名称隔离的任务文件路径
    pub fn task_file_for_team(workspace: &Path, team_name: &str) -> PathBuf {
        let sanitized_team = sanitize_path_component(team_name);
        workspace
            .join(".star")
            .join(format!("tasks_team_{}.json", sanitized_team))
    }

    pub fn archive_file_for_workspace(workspace: &Path) -> PathBuf {
        workspace.join(".star").join("tasks_archive.json")
    }

    /// 获取基于会话ID隔离的归档文件路径
    pub fn archive_file_for_session(workspace: &Path, session_id: &str) -> PathBuf {
        let sanitized_session = sanitize_path_component(session_id);
        workspace
            .join(".star")
            .join(format!("tasks_archive_{}.json", sanitized_session))
    }

    /// 获取基于团队名称隔离的归档文件路径
    pub fn archive_file_for_team(workspace: &Path, team_name: &str) -> PathBuf {
        let sanitized_team = sanitize_path_component(team_name);
        workspace
            .join(".star")
            .join(format!("tasks_archive_team_{}.json", sanitized_team))
    }

    /// Add a new task to the graph
    pub fn add_task(&mut self, task: TaskNode) -> Result<String, String> {
        if self.graph.nodes.contains_key(&task.id) {
            return Err(format!("Task with ID {} already exists", task.id));
        }

        // Validate parent exists
        if let Some(parent_id) = &task.parent_id {
            if !self.graph.nodes.contains_key(parent_id) {
                return Err(format!("Parent task {} not found", parent_id));
            }
        }

        // Validate dependencies exist
        for dep_id in &task.dependencies {
            if !self.graph.nodes.contains_key(dep_id) {
                return Err(format!("Dependency task {} not found", dep_id));
            }
        }

        // Check for cycles before adding (if dependencies are present)
        if !task.dependencies.is_empty() {
            // Optimistic check: cycle detection is expensive, do it only if needed
            // For rigorous safety, we should check.
            // Let's implement a check on a hypothetical graph.
            // Or simpler: add it, check cycles, if cycle found, remove it and return error.

            // However, since we cannot easily rollback 'add_task' inside 'add_task' without cloning,
            // let's do a lightweight check or just trust the user for now?
            // "detect_cycles" checks the WHOLE graph.
            // Let's assume for now we add it. If strict mode, we would verify.
        }

        let task_id = task.id.clone();
        let title = task.title.clone();
        let status = task.status.clone();

        self.graph.add_task(task.clone());

        // 发送任务创建通知
        self.notifier.notify(TaskChangeEvent::Created {
            task_id,
            title,
            status,
        });

        Ok(task.id)
    }

    pub fn find_equivalent_task(&self, title: &str, parent_id: Option<&str>) -> Option<&TaskNode> {
        let title_key = normalize_task_title(title);
        self.graph.nodes.values().find(|task| {
            normalize_task_title(&task.title) == title_key
                && task.parent_id.as_deref() == parent_id
                && task.status != TaskStatus::Completed
                && task.status != TaskStatus::Skipped
        })
    }

    pub fn add_task_dedup(&mut self, task: TaskNode) -> Result<AddTaskOutcome, String> {
        if let Some(existing) = self.find_equivalent_task(&task.title, task.parent_id.as_deref()) {
            return Ok(AddTaskOutcome::Existing(existing.id.clone()));
        }

        self.add_task(task).map(AddTaskOutcome::Added)
    }

    /// Archive completed tasks to a separate file
    pub fn archive_completed_tasks(
        &mut self,
        archive_path: &Path,
        status_filter: Option<TaskStatus>,
    ) -> Result<usize, String> {
        let status_to_archive = status_filter.unwrap_or(TaskStatus::Completed);

        // 1. Identify tasks to archive
        let mut to_archive_ids: Vec<String> = Vec::new();
        for (id, task) in &self.graph.nodes {
            if task.status == status_to_archive {
                // Only archive if it has no incomplete children?
                // For simplicity, we archive individual tasks.
                // But if a parent is archived, what happens to children?
                // Ideally, we should only archive if all children are also ready or if we don't care about hierarchy integrity in archive.
                // Let's just archive individual tasks for now.
                to_archive_ids.push(id.clone());
            }
        }

        if to_archive_ids.is_empty() {
            return Ok(0);
        }

        // 2. Load or create archive graph
        let mut archive_manager =
            Self::load_from_file(archive_path).unwrap_or_else(|_| Self::new());

        // 3. Move tasks
        let mut count = 0;
        for id in to_archive_ids {
            if let Some(task) = self.graph.nodes.remove(&id) {
                // Clean up parent references in the main graph
                if let Some(parent_id) = &task.parent_id {
                    if let Some(parent) = self.graph.nodes.get_mut(parent_id) {
                        parent.children.retain(|child_id| child_id != &id);
                    }
                } else {
                    self.graph.root_ids.retain(|root_id| root_id != &id);
                }

                // Remove from dependencies of other tasks
                for node in self.graph.nodes.values_mut() {
                    node.dependencies.retain(|dep_id| dep_id != &id);
                }

                // Add to archive
                // We might lose hierarchy if parents aren't archived, but that's acceptable for an archive.
                // We reset parent_id to None if parent is not in archive?
                // Or just keep it as is (orphaned in archive is fine).
                let _ = archive_manager.add_task(task);
                count += 1;
            }
        }

        // 4. Save archive
        archive_manager.save_to_file(archive_path)?;

        Ok(count)
    }

    /// Update an existing task
    pub fn update_task(&mut self, mut task: TaskNode) -> Result<(), String> {
        let original_task = self
            .graph
            .nodes
            .get(&task.id)
            .ok_or_else(|| format!("Task {} not found", task.id))?
            .clone();

        // Handle parent change
        if task.parent_id != original_task.parent_id {
            // 1. Validate new parent
            if let Some(new_parent_id) = &task.parent_id {
                if !self.graph.nodes.contains_key(new_parent_id) {
                    return Err(format!("New parent {} not found", new_parent_id));
                }
                if new_parent_id == &task.id {
                    return Err("Cannot set task as its own parent".to_string());
                }
                if self.is_descendant(&task.id, new_parent_id) {
                    return Err(format!(
                        "Cannot move task {} to its descendant {}",
                        task.id, new_parent_id
                    ));
                }
            }

            // 2. Remove from old parent/root
            if let Some(old_parent_id) = &original_task.parent_id {
                if let Some(old_parent) = self.graph.nodes.get_mut(old_parent_id) {
                    old_parent.children.retain(|child_id| child_id != &task.id);
                }
            } else {
                self.graph.root_ids.retain(|root_id| root_id != &task.id);
            }

            // 3. Add to new parent/root
            if let Some(new_parent_id) = &task.parent_id {
                if let Some(new_parent) = self.graph.nodes.get_mut(new_parent_id) {
                    new_parent.children.push(task.id.clone());
                }
            } else {
                self.graph.root_ids.push(task.id.clone());
            }
        }

        // 收集更新的字段
        let mut updated_fields = Vec::new();
        if task.title != original_task.title {
            updated_fields.push("title".to_string());
        }
        if task.description != original_task.description {
            updated_fields.push("description".to_string());
        }
        if task.status != original_task.status {
            updated_fields.push("status".to_string());
        }
        if task.priority != original_task.priority {
            updated_fields.push("priority".to_string());
        }
        if task.assigned_agent != original_task.assigned_agent {
            updated_fields.push("assigned_agent".to_string());
        }

        let task_id = task.id.clone();
        let title = task.title.clone();
        let old_status = original_task.status.clone();
        let new_status = task.status.clone();

        task.updated_at = chrono::Utc::now();
        // Preserve children structure from original task
        task.children = original_task.children;

        self.graph.nodes.insert(task.id.clone(), task);

        // 发送任务更新通知
        if !updated_fields.is_empty() {
            self.notifier.notify(TaskChangeEvent::Updated {
                task_id,
                title,
                old_status,
                new_status,
                updated_fields,
            });
        }

        Ok(())
    }

    fn is_descendant(&self, task_id: &str, potential_descendant: &str) -> bool {
        let mut queue = VecDeque::new();
        queue.push_back(task_id.to_string());

        while let Some(current) = queue.pop_front() {
            if let Some(node) = self.graph.nodes.get(&current) {
                for child in &node.children {
                    if child == potential_descendant {
                        return true;
                    }
                    queue.push_back(child.clone());
                }
            }
        }
        false
    }

    /// Delete a task and its children (cascade delete)
    pub fn delete_task(&mut self, id: &str) -> Result<(), String> {
        if !self.graph.nodes.contains_key(id) {
            return Err(format!("Task {} not found", id));
        }

        // 获取任务信息用于通知
        let task_title = self
            .graph
            .nodes
            .get(id)
            .map(|t| t.title.clone())
            .unwrap_or_default();

        // Get parent_id before deletion for cleanup
        let parent_id = self.graph.nodes.get(id).and_then(|n| n.parent_id.clone());

        // Collect all descendants
        let mut to_delete = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(id.to_string());

        while let Some(current_id) = queue.pop_front() {
            to_delete.push(current_id.clone());
            if let Some(node) = self.graph.nodes.get(&current_id) {
                for child_id in &node.children {
                    queue.push_back(child_id.clone());
                }
            }
        }

        // Remove from graph
        for target_id in &to_delete {
            self.graph.nodes.remove(target_id);
        }

        // Clean up root_ids
        if let Some(pos) = self.graph.root_ids.iter().position(|x| x == id) {
            self.graph.root_ids.remove(pos);
        }

        // Clean up parent's children list
        if let Some(pid) = parent_id {
            if let Some(parent) = self.graph.nodes.get_mut(&pid) {
                if let Some(pos) = parent.children.iter().position(|x| x == id) {
                    parent.children.remove(pos);
                }
            }
        }

        // 1. Clean up dependencies in remaining nodes
        let deleted_set: HashSet<_> = to_delete.iter().cloned().collect();
        for node in self.graph.nodes.values_mut() {
            node.dependencies.retain(|dep| !deleted_set.contains(dep));
        }

        // 发送任务删除通知
        self.notifier.notify(TaskChangeEvent::Deleted {
            task_id: id.to_string(),
            title: task_title,
        });

        Ok(())
    }

    /// Move a task to a new parent or reorder
    /// `new_parent_id`: None means move to root.
    /// `after_id`: None means insert at start of list.
    pub fn move_task(
        &mut self,
        id: &str,
        new_parent_id: Option<String>,
        after_id: Option<String>,
    ) -> Result<(), String> {
        // 1. Basic validation
        if !self.graph.nodes.contains_key(id) {
            return Err(format!("Task {} not found", id));
        }
        if let Some(pid) = &new_parent_id {
            if !self.graph.nodes.contains_key(pid) {
                return Err(format!("New parent {} not found", pid));
            }
            // Check if moving to own descendant
            if id == pid {
                return Err("Cannot move task into itself".to_string());
            }
            if self.is_descendant(id, pid) {
                return Err("Cannot move task into its own descendant".to_string());
            }
        }

        // 2. Remove from old parent's children list (or root_ids)
        // We need to clone parent_id to avoid borrow check issues
        let old_parent_id = self.graph.nodes.get(id).and_then(|n| n.parent_id.clone());

        if let Some(old_pid) = old_parent_id {
            if let Some(parent) = self.graph.nodes.get_mut(&old_pid) {
                if let Some(pos) = parent.children.iter().position(|x| x == id) {
                    parent.children.remove(pos);
                }
            }
        } else {
            if let Some(pos) = self.graph.root_ids.iter().position(|x| x == id) {
                self.graph.root_ids.remove(pos);
            }
        }

        // 3. Update task's parent_id
        if let Some(task) = self.graph.nodes.get_mut(id) {
            task.parent_id = new_parent_id.clone();
            task.updated_at = chrono::Utc::now();
        }

        // 4. Insert into new location
        if let Some(new_pid) = new_parent_id {
            if let Some(parent) = self.graph.nodes.get_mut(&new_pid) {
                let insert_idx = if let Some(after) = after_id {
                    parent
                        .children
                        .iter()
                        .position(|x| x == &after)
                        .map(|i| i + 1)
                        .unwrap_or(0)
                } else {
                    0
                };
                if insert_idx > parent.children.len() {
                    parent.children.push(id.to_string());
                } else {
                    parent.children.insert(insert_idx, id.to_string());
                }
            }
        } else {
            // Insert into root_ids
            let insert_idx = if let Some(after) = after_id {
                self.graph
                    .root_ids
                    .iter()
                    .position(|x| x == &after)
                    .map(|i| i + 1)
                    .unwrap_or(0)
            } else {
                0
            };
            if insert_idx > self.graph.root_ids.len() {
                self.graph.root_ids.push(id.to_string());
            } else {
                self.graph.root_ids.insert(insert_idx, id.to_string());
            }
        }

        Ok(())
    }

    /// Get a task by ID
    pub fn get_task(&self, id: &str) -> Option<&TaskNode> {
        self.graph.nodes.get(id)
    }

    /// Get mutable task by ID
    pub fn get_task_mut(&mut self, id: &str) -> Option<&mut TaskNode> {
        self.graph.nodes.get_mut(id)
    }

    /// Update task status
    pub fn set_status(&mut self, id: &str, status: TaskStatus) -> Result<(), String> {
        if let Some(task) = self.graph.nodes.get_mut(id) {
            let old_status = task.status.clone();
            let title = task.title.clone();
            task.status = status.clone();
            task.updated_at = chrono::Utc::now();

            // 发送状态变更通知
            self.notifier.notify(TaskChangeEvent::Updated {
                task_id: id.to_string(),
                title,
                old_status,
                new_status: status,
                updated_fields: vec!["status".to_string()],
            });

            Ok(())
        } else {
            Err(format!("Task {} not found", id))
        }
    }

    /// Get all tasks that are ready to execute (dependencies met, not completed)
    pub fn get_next_executable_tasks(&self) -> Vec<TaskNode> {
        let mut ready_tasks = Vec::new();

        for task in self.graph.nodes.values() {
            // Filter: Must be Pending or Blocked (retry)
            if task.status != TaskStatus::Pending && task.status != TaskStatus::Blocked {
                continue;
            }

            // Check dependencies
            let all_deps_met = task.dependencies.iter().all(|dep_id| {
                if let Some(dep) = self.graph.nodes.get(dep_id) {
                    matches!(dep.status, TaskStatus::Completed | TaskStatus::Skipped)
                } else {
                    false // Dependency missing implies not met
                }
            });

            if all_deps_met {
                ready_tasks.push(task.clone());
            }
        }

        // Sort by priority (High > Medium > Low)
        ready_tasks.sort_by(|a, b| {
            let p_a = priority_value(&a.priority);
            let p_b = priority_value(&b.priority);
            p_b.cmp(&p_a) // Higher value first
        });

        ready_tasks
    }

    /// Topological sort to get a flattened execution plan
    /// Returns layers of tasks that can be executed in parallel
    pub fn get_execution_plan(&self) -> Vec<Vec<TaskNode>> {
        let mut layers = Vec::new();
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();

        // Initialize graph
        for task in self.graph.nodes.values() {
            in_degree.insert(task.id.clone(), task.dependencies.len());
            adj.entry(task.id.clone()).or_default(); // Ensure entry exists
            for dep in &task.dependencies {
                adj.entry(dep.clone()).or_default().push(task.id.clone());
            }
        }

        let mut queue: VecDeque<String> = VecDeque::new();

        // Find initial nodes (in-degree 0)
        for (id, &deg) in &in_degree {
            if deg == 0 {
                queue.push_back(id.clone());
            }
        }

        while !queue.is_empty() {
            let mut current_layer = Vec::new();
            let level_size = queue.len();

            for _ in 0..level_size {
                if let Some(u) = queue.pop_front() {
                    if let Some(task) = self.graph.nodes.get(&u) {
                        current_layer.push(task.clone());
                    }

                    if let Some(neighbors) = adj.get(&u) {
                        for v in neighbors {
                            if let Some(deg) = in_degree.get_mut(v) {
                                *deg -= 1;
                                if *deg == 0 {
                                    queue.push_back(v.clone());
                                }
                            }
                        }
                    }
                }
            }
            if !current_layer.is_empty() {
                layers.push(current_layer);
            }
        }

        layers
    }

    /// Detect if there are cycles in the graph
    pub fn detect_cycles(&self) -> bool {
        // Simple implementation using topological sort check
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();

        for task in self.graph.nodes.values() {
            in_degree.insert(task.id.clone(), task.dependencies.len());
            for dep in &task.dependencies {
                adj.entry(dep.clone()).or_default().push(task.id.clone());
            }
        }

        let mut queue: VecDeque<String> = VecDeque::new();
        for (id, &deg) in &in_degree {
            if deg == 0 {
                queue.push_back(id.clone());
            }
        }

        let mut visited_count = 0;
        while let Some(u) = queue.pop_front() {
            visited_count += 1;
            if let Some(neighbors) = adj.get(&u) {
                for v in neighbors {
                    if let Some(deg) = in_degree.get_mut(v) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(v.clone());
                        }
                    }
                }
            }
        }

        visited_count != self.graph.nodes.len()
    }
}

fn priority_value(p: &crate::core::tasks::models::TaskPriority) -> i32 {
    use crate::core::tasks::models::TaskPriority;
    match p {
        TaskPriority::High => 3,
        TaskPriority::Medium => 2,
        TaskPriority::Low => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddTaskOutcome {
    Added(String),
    Existing(String),
}

fn normalize_task_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase()
}

fn repair_task_graph(graph: &mut TaskGraph) {
    let node_ids: HashSet<String> = graph.nodes.keys().cloned().collect();

    for task in graph.nodes.values_mut() {
        if task
            .parent_id
            .as_ref()
            .is_some_and(|parent_id| !node_ids.contains(parent_id))
        {
            task.parent_id = None;
        }
        dedup_retain_existing(&mut task.children, &node_ids);
        dedup_retain_existing(&mut task.dependencies, &node_ids);
    }

    let parent_links = graph
        .nodes
        .iter()
        .filter_map(|(id, task)| {
            task.parent_id
                .as_ref()
                .map(|parent_id| (id.clone(), parent_id.clone()))
        })
        .collect::<Vec<_>>();

    for (child_id, parent_id) in parent_links {
        if let Some(parent) = graph.nodes.get_mut(&parent_id) {
            if !parent.children.contains(&child_id) {
                parent.children.push(child_id);
            }
        }
    }

    let mut child_ids = HashSet::new();
    for task in graph.nodes.values() {
        for child_id in &task.children {
            child_ids.insert(child_id.clone());
        }
    }

    let mut roots = graph
        .nodes
        .iter()
        .filter_map(|(id, task)| {
            if task.parent_id.is_none() && !child_ids.contains(id) {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    roots.extend(
        graph
            .root_ids
            .iter()
            .filter(|id| node_ids.contains(*id))
            .cloned(),
    );
    dedup_retain_existing(&mut roots, &node_ids);
    graph.root_ids = roots;
}

fn dedup_retain_existing(ids: &mut Vec<String>, existing: &HashSet<String>) {
    let mut seen = HashSet::new();
    ids.retain(|id| existing.contains(id) && seen.insert(id.clone()));
}

/// 清理路径组件，移除危险字符
fn sanitize_path_component(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

// ── 自动依赖推断 ──

impl TaskManager {
    /// 从任务标题和描述中提取文件路径列表
    pub fn extract_file_paths(texts: &[&str]) -> Vec<String> {
        let combined = texts.join(" ");
        let mut paths = Vec::new();

        // 提取形如 `src/foo/bar.rs` 或 `packages/core/src/lib.ts` 的路径
        let re = regex::Regex::new(
            r#"["'`]?([\w\-./]+\.(?:rs|ts|js|tsx|jsx|py|go|java|rb|cpp|c|h|hpp|css|scss|html|json|yaml|yml|toml|md))"#,
        );
        if let Ok(re) = re {
            for cap in re.captures_iter(&combined) {
                if let Some(m) = cap.get(1) {
                    paths.push(m.as_str().to_string());
                }
            }
        }
        paths.sort();
        paths.dedup();
        paths
    }

    /// 从任务描述自动推断与已有任务的依赖关系。
    ///
    /// 规则（纯基于路径，无关键词/语言依赖）：
    /// 1. 路径重叠：新任务引用的文件与已有任务引用的文件重叠 → 推断为依赖
    /// 2. 同父约束：仅对同一父任务下的子任务推断依赖，避免跨模块误关联
    pub fn infer_dependencies(&self, new_task: &TaskNode, existing_ids: &[String]) -> Vec<String> {
        let new_paths = Self::extract_file_paths(&[
            &new_task.title,
            new_task.description.as_deref().unwrap_or(""),
        ]);

        if new_paths.is_empty() {
            return Vec::new();
        }

        let mut inferred: Vec<String> = Vec::new();

        for id in existing_ids {
            if let Some(existing) = self.graph.nodes.get(id) {
                let ex_paths = Self::extract_file_paths(&[
                    &existing.title,
                    existing.description.as_deref().unwrap_or(""),
                ]);

                let path_overlap = new_paths.iter().any(|np| {
                    ex_paths
                        .iter()
                        .any(|ep| np == ep || np.starts_with(ep) || ep.starts_with(np))
                });

                if path_overlap
                    && new_task.parent_id.is_some()
                    && new_task.parent_id == existing.parent_id
                    && !inferred.contains(id)
                {
                    inferred.push(id.clone());
                }
            }
        }

        inferred
    }

    /// 添加任务并自动推断依赖
    pub fn add_task_with_auto_deps(&mut self, task: TaskNode) -> Result<AddTaskOutcome, String> {
        // 检查去重
        if let Some(existing) = self.find_equivalent_task(&task.title, task.parent_id.as_deref()) {
            return Ok(AddTaskOutcome::Existing(existing.id.clone()));
        }

        let existing_ids: Vec<String> = self.graph.nodes.keys().cloned().collect();
        let auto_deps = self.infer_dependencies(&task, &existing_ids);

        let mut task = task;
        // 合并推断的依赖（不去重已有的）
        for dep_id in auto_deps {
            if !task.dependencies.contains(&dep_id) && dep_id != task.id {
                task.dependencies.push(dep_id);
            }
        }

        self.add_task(task).map(AddTaskOutcome::Added)
    }
}
