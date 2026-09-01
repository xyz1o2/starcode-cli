/// 作业调度器

/// 作业调度器
pub struct JobScheduler {
    /// 调度队列
    queue: Vec<String>,
}

impl JobScheduler {
    /// 创建新的作业调度器
    pub fn new() -> Self {
        Self {
            queue: Vec::new(),
        }
    }

    /// 调度作业
    pub fn schedule(&mut self, job_id: &str) {
        self.queue.push(job_id.to_string());
    }

    /// 获取下一个作业
    pub fn next(&mut self) -> Option<String> {
        if self.queue.is_empty() {
            None
        } else {
            Some(self.queue.remove(0))
        }
    }

    /// 获取队列长度
    pub fn queue_length(&self) -> usize {
        self.queue.len()
    }

    /// 检查队列是否为空
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
