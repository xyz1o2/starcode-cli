/// 摘要存储

use super::types::ToolCallRecord;
use std::collections::HashMap;

/// 摘要存储
pub struct SummaryStorage {
    /// 记录列表
    records: Vec<ToolCallRecord>,
    /// 最大记录数
    max_records: usize,
}

impl SummaryStorage {
    /// 创建新的摘要存储
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            max_records: 1000,
        }
    }

    /// 添加记录
    pub fn add_record(&mut self, record: ToolCallRecord) {
        self.records.push(record);

        // 限制记录数量
        if self.records.len() > self.max_records {
            self.records.remove(0);
        }
    }

    /// 获取所有记录
    pub fn get_all_records(&self) -> &[ToolCallRecord] {
        &self.records
    }

    /// 按工具名称获取记录
    pub fn get_records_by_tool(&self, tool_name: &str) -> Vec<&ToolCallRecord> {
        self.records.iter()
            .filter(|r| r.tool_name == tool_name)
            .collect()
    }

    /// 获取记录数量
    pub fn count(&self) -> usize {
        self.records.len()
    }

    /// 清空记录
    pub fn clear(&mut self) {
        self.records.clear();
    }
}
