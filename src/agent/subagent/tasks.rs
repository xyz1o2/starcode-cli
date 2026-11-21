//! 后台子代理的运行记录。RuntimeServices 持有共享实例，runner 登记和收尾，
//! TaskOutput 查询同一份记录。终态保留输出路径，通知消费不会删除查询记录。
//! 所有访问由 Tokio Mutex 串行化；文件读取在释放注册表锁后进行。

use crate::agent::subagent::notification::NotificationStatus;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 只发布终态，磁盘记录不携带可控制的路径或取消句柄。
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTask {
    version: u32,
    task_id: String,
    description: String,
    status: NotificationStatus,
    output_error: Option<String>,
}

impl SubagentTask {
    pub async fn persist_final(&self) -> Result<(), String> {
        let status = self.status.clone().ok_or("Cannot persist a running task")?;
        let record = StoredTask {
            version: 1,
            task_id: self.task_id.clone(),
            description: self.description.clone(),
            status,
            output_error: self.output_error.clone(),
        };
        let bytes = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
        let destination = self.output_file.with_extension("json");
        let temporary = self
            .output_file
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        // 临时文件与目标同目录，读者只会看到完整的终态记录。
        let result = async {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .await?;
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&temporary, &destination).await
        }
        .await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(format!(
                "Failed to store task record {}: {error}",
                self.task_id
            ));
        }
        Ok(())
    }

    pub async fn load_final(
        directory: &std::path::Path,
        task_id: &str,
    ) -> Result<Option<Self>, String> {
        let path = directory.join(format!("{task_id}.json"));
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Failed to read task record {task_id}: {error}")),
        };
        let record: StoredTask = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid task record {task_id}: {error}"))?;
        if record.version != 1 || record.task_id != task_id {
            return Err(format!(
                "Invalid task record {task_id}: unsupported version or mismatched ID"
            ));
        }
        Ok(Some(Self {
            task_id: record.task_id,
            description: record.description,
            output_file: directory.join(format!("{task_id}.txt")),
            status: Some(record.status),
            output_error: record.output_error,
            cancel: None,
        }))
    }
}

pub(crate) type SharedSubagentTasks = Arc<Mutex<SubagentTasks>>;

#[derive(Clone, Debug)]
pub(crate) struct SubagentTask {
    pub task_id: String,
    pub description: String,
    pub output_file: PathBuf,
    /// None 表示仍在执行；终态与通知使用同一状态类型。
    pub status: Option<NotificationStatus>,
    pub output_error: Option<std::string::String>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

#[derive(Default)]
pub(crate) struct SubagentTasks {
    entries: HashMap<String, SubagentTask>,
}

impl SubagentTasks {
    pub fn register(&mut self, task_id: &str, description: String, output_file: PathBuf) {
        self.entries.insert(
            task_id.to_owned(),
            SubagentTask {
                task_id: task_id.to_owned(),
                description,
                output_file,
                status: None,
                output_error: None,
                cancel: None,
            },
        );
    }

    pub fn finish(
        &mut self,
        task_id: &str,
        status: NotificationStatus,
        output_error: Option<String>,
    ) {
        if let Some(task) = self.entries.get_mut(task_id) {
            // 收尾只能完成一次，晚到的旧通知不能改写已经发布的终态。
            if task.status.is_none() {
                task.status = Some(status);
                task.output_error = output_error;
            }
        }
    }

    pub fn close_control(&mut self, task_id: &str) -> bool {
        self.entries
            .get_mut(task_id)
            .and_then(|task| task.cancel.take())
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
    }

    pub fn attach_cancel(&mut self, task_id: &str, cancel: Arc<std::sync::atomic::AtomicBool>) {
        if let Some(task) = self.entries.get_mut(task_id) {
            task.cancel = Some(cancel);
        }
    }

    /// 只发停止请求；实际循环退出并收尾后才发布终态。
    pub fn request_stop(&self, task_id: &str) -> Result<bool, String> {
        let task = self
            .entries
            .get(task_id)
            .ok_or_else(|| format!("Task '{task_id}' not found"))?;
        if task.status.is_some() {
            return Ok(false);
        }
        let Some(cancel) = task.cancel.as_ref() else {
            return Ok(false);
        };
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(true)
    }

    pub fn get(&self, task_id: &str) -> Option<SubagentTask> {
        self.entries.get(task_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn request_stop_distinguishes_missing_uncontrollable_active_and_finished_tasks() {
        let mut tasks = SubagentTasks::default();
        assert!(tasks.request_stop("missing").is_err());

        tasks.register("unattached", "work".into(), PathBuf::from("output.txt"));
        assert!(!tasks.request_stop("unattached").unwrap());

        tasks.register("active", "work".into(), PathBuf::from("output.txt"));
        let cancel = Arc::new(AtomicBool::new(false));
        tasks.attach_cancel("active", cancel.clone());
        assert!(tasks.request_stop("active").unwrap());
        assert!(cancel.load(Ordering::SeqCst));

        tasks.finish("active", NotificationStatus::Completed, None);
        assert!(!tasks.request_stop("active").unwrap());
    }
}
