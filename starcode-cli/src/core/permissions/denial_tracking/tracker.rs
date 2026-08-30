/// 拒绝追踪器

use super::DenialRecord;

/// 拒绝追踪器
pub struct DenialTracker {
    /// 拒绝记录
    records: Vec<DenialRecord>,
    /// 最大记录数
    max_records: usize,
}

impl DenialTracker {
    /// 创建新的拒绝追踪器
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            max_records: 1000,
        }
    }

    /// 记录拒绝
    pub fn record(&mut self, record: DenialRecord) {
        self.records.push(record);

        // 限制记录数量
        if self.records.len() > self.max_records {
            self.records.remove(0);
        }
    }

    /// 获取所有记录
    pub fn get_all_records(&self) -> &[DenialRecord] {
        &self.records
    }

    /// 获取最近的记录
    pub fn get_recent_records(&self, count: usize) -> Vec<&DenialRecord> {
        self.records.iter().rev().take(count).collect()
    }

    /// 清空记录
    pub fn clear(&mut self) {
        self.records.clear();
    }
}
