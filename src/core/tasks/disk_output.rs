/// 磁盘输出管理
///
/// 对标claude-code-main的src/utils/task/diskOutput.ts
/// 管理任务输出的磁盘存储
use std::path::{Path, PathBuf};

/// 磁盘输出管理器
pub struct DiskOutputManager {
    /// 输出目录
    output_dir: PathBuf,
}

impl DiskOutputManager {
    /// 创建新的磁盘输出管理器
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }

    /// 初始化任务输出
    pub fn init_task_output(&self, task_id: &str) -> Result<PathBuf, std::io::Error> {
        let task_dir = self.output_dir.join(task_id);
        std::fs::create_dir_all(&task_dir)?;
        Ok(task_dir)
    }

    /// 获取任务输出路径
    pub fn get_task_output_path(&self, task_id: &str) -> PathBuf {
        self.output_dir.join(task_id).join("output.txt")
    }

    /// 写入任务输出
    pub fn write_task_output(&self, task_id: &str, content: &str) -> Result<(), std::io::Error> {
        let output_path = self.get_task_output_path(task_id);
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&output_path, content)
    }

    /// 追加任务输出
    pub fn append_task_output(&self, task_id: &str, content: &str) -> Result<(), std::io::Error> {
        let output_path = self.get_task_output_path(task_id);
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&output_path)?;
        writeln!(file, "{}", content)
    }

    /// 读取任务输出
    pub fn read_task_output(&self, task_id: &str) -> Result<String, std::io::Error> {
        let output_path = self.get_task_output_path(task_id);
        std::fs::read_to_string(&output_path)
    }

    /// 获取任务输出增量
    pub fn get_task_output_delta(
        &self,
        task_id: &str,
        offset: usize,
    ) -> Result<(String, usize), std::io::Error> {
        let content = self.read_task_output(task_id)?;
        let new_offset = content.len();
        let delta = if offset < content.len() {
            content[offset..].to_string()
        } else {
            String::new()
        };
        Ok((delta, new_offset))
    }

    /// 删除任务输出
    pub fn delete_task_output(&self, task_id: &str) -> Result<(), std::io::Error> {
        let task_dir = self.output_dir.join(task_id);
        if task_dir.exists() {
            std::fs::remove_dir_all(&task_dir)?;
        }
        Ok(())
    }

    /// 清理旧输出
    pub fn cleanup_old_outputs(&self, max_age_days: u64) -> Result<usize, std::io::Error> {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            - (max_age_days * 86400);

        let mut cleaned = 0;
        if self.output_dir.exists() {
            for entry in std::fs::read_dir(&self.output_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if let Ok(modified) = metadata.modified() {
                            let modified_secs = modified
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            if modified_secs < cutoff {
                                std::fs::remove_dir_all(&path)?;
                                cleaned += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(cleaned)
    }
}
