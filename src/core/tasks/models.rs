use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Skipped,
}

/// 任务变更事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskChangeEvent {
    /// 任务创建
    Created {
        task_id: String,
        title: String,
        status: TaskStatus,
    },
    /// 任务更新
    Updated {
        task_id: String,
        title: String,
        old_status: TaskStatus,
        new_status: TaskStatus,
        updated_fields: Vec<String>,
    },
    /// 任务删除
    Deleted { task_id: String, title: String },
    /// 任务列表重置
    Reset,
}

/// 任务变更通知器
#[derive(Clone)]
pub struct TaskNotifier {
    sender: broadcast::Sender<TaskChangeEvent>,
}

impl TaskNotifier {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(100);
        Self { sender }
    }

    /// 发送任务变更通知
    pub fn notify(&self, event: TaskChangeEvent) {
        // 忽略发送错误（如果没有接收者）
        let _ = self.sender.send(event);
    }

    /// 订阅任务变更
    pub fn subscribe(&self) -> broadcast::Receiver<TaskChangeEvent> {
        self.sender.subscribe()
    }
}

impl Default for TaskNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskPriority {
    High,
    Medium,
    Low,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Medium
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub dependencies: Vec<String>,
    /// 本任务阻塞的其他任务 ID（反向依赖，对标 Claude Code 的 `blocks`）
    #[serde(default)]
    pub blocks: Vec<String>,
    pub children: Vec<String>,
    pub assigned_agent: Option<String>,
    /// 进行时描述（对标 Claude Code 的 `activeForm`）：任务 in_progress 时面板/转圈
    /// 显示 "Running tests" 而不是祈使句 "Run tests"。旧文件没有该字段 → None。
    #[serde(default)]
    pub active_form: Option<String>,
    /// 完成时间（对标 Claude Code 的30s TTL自动清除）。
    /// 旧文件没有该字段 → None，不会被自动清除。
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskNode {
    pub fn new(title: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            title,
            description: None,
            status: TaskStatus::Pending,
            priority: TaskPriority::Medium,
            dependencies: Vec::new(),
            blocks: Vec::new(),
            children: Vec::new(),
            assigned_agent: None,
            active_form: None,
            completed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskGraph {
    pub nodes: HashMap<String, TaskNode>,
    pub root_ids: Vec<String>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            root_ids: Vec::new(),
        }
    }

    /// 按清单顺序（root_ids → children DFS）返回所有任务 id。
    ///
    /// `nodes` 是 HashMap，直接 `.values()` 遍历顺序随机；agent 端任何
    /// "列出任务"的路径都必须走这里，否则模型读到的清单与面板显示的
    /// 顺序不一致，会不按清单从上往下执行。
    /// 兜底：因图损坏游离在 root_ids/children 之外的节点按 id 追加在末尾。
    pub fn ordered_ids(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(self.nodes.len());

        fn walk(
            id: &str,
            nodes: &HashMap<String, TaskNode>,
            seen: &mut std::collections::HashSet<String>,
            out: &mut Vec<String>,
        ) {
            if !seen.insert(id.to_string()) {
                return;
            }
            out.push(id.to_string());
            if let Some(node) = nodes.get(id) {
                for child in &node.children {
                    walk(child, nodes, seen, out);
                }
            }
        }

        for root in &self.root_ids {
            walk(root, &self.nodes, &mut seen, &mut out);
        }

        let mut orphans: Vec<&String> =
            self.nodes.keys().filter(|id| !seen.contains(*id)).collect();
        orphans.sort();
        for id in orphans {
            out.push(id.clone());
        }
        out
    }

    pub fn add_task(&mut self, mut task: TaskNode) {
        if task.parent_id.is_none() {
            if !self.root_ids.contains(&task.id) {
                self.root_ids.push(task.id.clone());
            }
        } else {
            // Ensure parent exists and add to parent's children
            if let Some(parent_id) = &task.parent_id {
                if let Some(parent) = self.nodes.get_mut(parent_id) {
                    if !parent.children.contains(&task.id) {
                        parent.children.push(task.id.clone());
                    }
                } else {
                    // Parent not found, treat as root? Or error?
                    // For now, force it to be root if parent missing
                    task.parent_id = None;
                    if !self.root_ids.contains(&task.id) {
                        self.root_ids.push(task.id.clone());
                    }
                }
            }
        }
        self.nodes.insert(task.id.clone(), task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 清单必须按写入顺序返回：agent 依赖这个顺序从上往下执行。
    /// 回归背景：id 是随机 uuid，任何按 id / HashMap 序的遍历都会打乱清单。
    #[test]
    fn ordered_ids_preserves_insertion_order() {
        let mut g = TaskGraph::new();
        let mut a = TaskNode::new("first".into());
        let mut b = TaskNode::new("second".into());
        let mut c = TaskNode::new("child of b".into());
        c.parent_id = Some(b.id.clone());
        b.children.push(c.id.clone());

        // nodes 是 HashMap，故意乱序插入，验证遍历仍走 root_ids/children
        g.nodes.insert(b.id.clone(), b.clone());
        g.nodes.insert(c.id.clone(), c.clone());
        g.nodes.insert(a.id.clone(), a.clone());
        g.root_ids = vec![a.id.clone(), b.id.clone()];

        let ids = g.ordered_ids();
        let titles: Vec<&str> = ids
            .iter()
            .map(|id| g.nodes.get(id).unwrap().title.as_str())
            .collect();
        assert_eq!(titles, vec!["first", "second", "child of b"]);
    }

    /// 图损坏产生游离节点时，兜底追加且不 panic、不重复。
    #[test]
    fn ordered_ids_appends_orphans_without_duplicating() {
        let mut g = TaskGraph::new();
        let mut a = TaskNode::new("listed".into());
        g.add_task(a.clone());
        // 模拟损坏：直接插入一个不在 root_ids/children 里的节点
        let orphan = TaskNode::new("orphan".into());
        g.nodes.insert(orphan.id.clone(), orphan);

        let ids = g.ordered_ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(g.nodes.get(&ids[0]).unwrap().title, "listed");
        assert_eq!(g.nodes.get(&ids[1]).unwrap().title, "orphan");
    }
}
