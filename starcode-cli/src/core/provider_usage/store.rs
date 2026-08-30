/// 使用量存储

use super::types::{UsageRecord, UsageSummary, ProviderUsage, ModelUsage};
use std::collections::HashMap;

/// 使用量存储
pub struct UsageStore {
    /// 使用量记录
    records: Vec<UsageRecord>,
    /// 存储路径
    storage_path: Option<String>,
}

impl UsageStore {
    /// 创建新的使用量存储
    pub fn new(storage_path: Option<String>) -> Self {
        let mut store = Self {
            records: Vec::new(),
            storage_path,
        };

        // 尝试从文件加载
        if let Some(path) = &store.storage_path {
            store.load_from_file(path);
        }

        store
    }

    /// 添加记录
    pub fn add_record(&mut self, record: UsageRecord) {
        self.records.push(record);

        // 限制记录数量
        if self.records.len() > 10000 {
            self.records.remove(0);
        }

        // 保存到文件
        if let Some(path) = &self.storage_path.clone() {
            self.save_to_file(path);
        }
    }

    /// 获取Provider使用量摘要
    pub fn get_summary(&self, provider: &str) -> UsageSummary {
        let provider_records: Vec<&UsageRecord> = self.records.iter()
            .filter(|r| r.provider == provider)
            .collect();

        let total_requests = provider_records.len() as u64;
        let total_prompt_tokens: u64 = provider_records.iter()
            .map(|r| r.prompt_tokens as u64)
            .sum();
        let total_completion_tokens: u64 = provider_records.iter()
            .map(|r| r.completion_tokens as u64)
            .sum();
        let total_tokens = total_prompt_tokens + total_completion_tokens;
        let total_cost: f64 = provider_records.iter()
            .filter_map(|r| r.cost)
            .sum();

        let start_time = provider_records.first()
            .map(|r| r.timestamp)
            .unwrap_or(0);
        let end_time = provider_records.last()
            .map(|r| r.timestamp)
            .unwrap_or(0);

        UsageSummary {
            provider: provider.to_string(),
            total_requests,
            total_prompt_tokens,
            total_completion_tokens,
            total_tokens,
            total_cost,
            average_latency_ms: 0.0,
            success_rate: 1.0,
            start_time,
            end_time,
        }
    }

    /// 获取所有Provider使用量
    pub fn get_all_usage(&self) -> Vec<ProviderUsage> {
        let mut provider_map: HashMap<String, Vec<&UsageRecord>> = HashMap::new();

        for record in &self.records {
            provider_map
                .entry(record.provider.clone())
                .or_insert_with(Vec::new)
                .push(record);
        }

        let mut result = Vec::new();

        for (provider, records) in provider_map {
            let mut model_map: HashMap<String, Vec<&UsageRecord>> = HashMap::new();

            for record in &records {
                model_map
                    .entry(record.model.clone())
                    .or_insert_with(Vec::new)
                    .push(record);
            }

            let models: Vec<ModelUsage> = model_map.iter()
                .map(|(model, records)| ModelUsage {
                    model: model.clone(),
                    requests: records.len() as u64,
                    total_tokens: records.iter().map(|r| r.total_tokens as u64).sum(),
                    cost: records.iter().filter_map(|r| r.cost).sum(),
                })
                .collect();

            let total = self.get_summary(&provider);

            result.push(ProviderUsage {
                provider,
                models,
                total,
            });
        }

        result
    }

    /// 从文件加载
    fn load_from_file(&mut self, path: &str) {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(records) = serde_json::from_str::<Vec<UsageRecord>>(&content) {
                self.records = records;
            }
        }
    }

    /// 保存到文件
    fn save_to_file(&self, path: &str) {
        if let Ok(content) = serde_json::to_string_pretty(&self.records) {
            let _ = std::fs::write(path, content);
        }
    }
}
