use chrono::Utc;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone)]
struct LearnedRulesCacheEntry {
    modified_at_unix_ms: Option<u128>,
    file_len: u64,
    content: String,
}

/// Reflector 组件
/// 职责：在任务结束后进行复盘，分析成功/失败原因，生成启发式规则。
pub struct Reflector {
    rules_path: PathBuf,
    cache: Mutex<Option<LearnedRulesCacheEntry>>,
}

impl Reflector {
    pub fn new(project_root: &Path) -> Self {
        // 将学习到的规则存储在 .star/context/learned_rules.md
        let rules_path = project_root
            .join(".star")
            .join("context")
            .join("learned_rules.md");
        Self {
            rules_path,
            cache: Mutex::new(None),
        }
    }

    /// 保存一条新规则
    pub fn save_rule(&self, rule: &str) -> Result<(), std::io::Error> {
        if let Some(parent) = self.rules_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.rules_path)?;

        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        // 写入 Markdown 列表格式
        writeln!(file, "- **[{}]** {}", timestamp, rule)?;
        self.invalidate_cache();
        Ok(())
    }

    /// 获取所有已学习的规则
    pub fn get_learned_rules(&self) -> Result<String, std::io::Error> {
        let metadata = match fs::metadata(&self.rules_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.invalidate_cache();
                return Ok(String::new());
            }
            Err(err) => return Err(err),
        };

        let modified_at_unix_ms = metadata
            .modified()
            .ok()
            .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis());
        let file_len = metadata.len();

        if let Ok(cache) = self.cache.lock() {
            if let Some(entry) = cache.as_ref() {
                if entry.modified_at_unix_ms == modified_at_unix_ms && entry.file_len == file_len {
                    return Ok(entry.content.clone());
                }
            }
        }

        let content = fs::read_to_string(&self.rules_path)?;
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some(LearnedRulesCacheEntry {
                modified_at_unix_ms,
                file_len,
                content: content.clone(),
            });
        }
        Ok(content)
    }

    /// 清除所有规则 (用于测试或重置)
    pub fn clear_rules(&self) -> Result<(), std::io::Error> {
        if self.rules_path.exists() {
            fs::remove_file(&self.rules_path)?;
        }
        self.invalidate_cache();
        Ok(())
    }

    fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = None;
        }
    }
}
