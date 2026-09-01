use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenialRecord {
    pub timestamp: i64,
    pub tool: String,
    pub args: Value,
    pub reason: String,
    pub rule_id: Option<String>,
}

pub struct DenyLog {
    records: RwLock<Vec<DenialRecord>>,
    persist_path: Option<PathBuf>,
    max_records: usize,
}

/// Session-scoped denial tracker for detecting consecutive same-tool denials
/// and injecting nudge messages to break potential infinite loops.
///
/// Mirrors Claude Code's `denialTracking.ts` logic: if the same tool is
/// denied ≥3 times consecutively within a session, auto-inject a system
/// message telling the model to stop trying that tool and use an alternative.
pub struct DenialTracker {
    /// (tool_name, consecutive_count) — reset when a different tool is used.
    current_tool: Option<String>,
    consecutive_count: u32,
    /// Total denials in this session (for logging/telemetry).
    total_denials: u32,
    /// Threshold for auto-injection (default 3, configurable).
    threshold: u32,
}

impl DenialTracker {
    pub fn new() -> Self {
        let threshold = std::env::var("STAR_DENIAL_TRACKING_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3)
            .clamp(2, 10);

        Self {
            current_tool: None,
            consecutive_count: 0,
            total_denials: 0,
            threshold,
        }
    }

    /// Record a denial for the given tool. Returns `Some(message)` if the
    /// threshold has been reached and a system nudge should be injected.
    pub fn record_denial(&mut self, tool: &str, reason: &str) -> Option<String> {
        self.total_denials += 1;

        match &self.current_tool {
            Some(current) if current == tool => {
                self.consecutive_count += 1;
            }
            _ => {
                // Different tool or first denial — reset the counter
                self.current_tool = Some(tool.to_string());
                self.consecutive_count = 1;
            }
        }

        if self.consecutive_count >= self.threshold {
            // Reset to avoid spamming the same nudge repeatedly
            self.consecutive_count = 0;
            self.current_tool = None;

            let nudge = format!(
                "[DENIAL_TRACKING] The tool `{tool}` has been denied {count} times \
                 consecutively (reason: {reason}). This tool cannot be used. \
                 DO NOT call `{tool}` again. Instead, use an alternative approach: \
                 try a different tool, restructure your workflow, or explain to \
                 the user why this operation cannot be performed. \
                 Proceed with the task using other available tools.",
                tool = tool,
                count = self.threshold,
                reason = reason,
            );
            return Some(nudge);
        }

        None
    }

    /// Reset the tracker for a new session/task.
    pub fn reset(&mut self) {
        self.current_tool = None;
        self.consecutive_count = 0;
        self.total_denials = 0;
    }

    /// Current consecutive count for the tracked tool.
    pub fn consecutive_count(&self) -> u32 {
        self.consecutive_count
    }

    /// Total denials recorded in this session.
    pub fn total_denials(&self) -> u32 {
        self.total_denials
    }

    /// The configured threshold for auto-injection.
    pub fn threshold(&self) -> u32 {
        self.threshold
    }
}

impl Default for DenialTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DenyLog {
    pub fn new() -> Self {
        Self {
            records: RwLock::new(Vec::new()),
            persist_path: None,
            max_records: 1000,
        }
    }

    pub fn with_persistence(path: PathBuf) -> Self {
        let log = Self {
            records: RwLock::new(Vec::new()),
            persist_path: Some(path),
            max_records: 1000,
        };
        log.load();
        log
    }

    pub fn with_max_records(mut self, max: usize) -> Self {
        self.max_records = max;
        self
    }

    pub fn record(&self, tool: &str, args: &Value, reason: &str, rule_id: Option<String>) {
        let record = DenialRecord {
            timestamp: chrono::Utc::now().timestamp(),
            tool: tool.to_string(),
            args: args.clone(),
            reason: reason.to_string(),
            rule_id,
        };

        let mut records = self.records.write().unwrap();
        records.push(record);

        if records.len() > self.max_records {
            let drain_count = records.len() - self.max_records;
            records.drain(0..drain_count);
        }
        drop(records);

        self.save();
    }

    pub fn get_records(&self) -> Vec<DenialRecord> {
        self.records.read().unwrap().clone()
    }

    pub fn get_records_for_tool(&self, tool: &str) -> Vec<DenialRecord> {
        self.records
            .read()
            .unwrap()
            .iter()
            .filter(|r| r.tool == tool)
            .cloned()
            .collect()
    }

    /// Check consecutive denials for a specific tool in recent history.
    /// Returns the count of consecutive denials for the given tool.
    pub fn consecutive_denials_for_tool(&self, tool: &str) -> u32 {
        let records = self.records.read().unwrap();
        let mut count = 0u32;
        // Iterate from most recent backwards
        for record in records.iter().rev() {
            if record.tool == tool {
                count += 1;
            } else {
                break; // Stop at first non-matching record (non-consecutive)
            }
        }
        count
    }

    pub fn count(&self) -> usize {
        self.records.read().unwrap().len()
    }

    pub fn clear(&self) {
        self.records.write().unwrap().clear();
        self.save();
    }

    fn load(&self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(parsed) = serde_json::from_str::<Vec<DenialRecord>>(&content) else {
            return;
        };
        let mut records = self.records.write().unwrap();
        *records = parsed;
    }

    fn save(&self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let records = self.records.read().unwrap();
        if let Ok(json) = serde_json::to_string_pretty(&*records) {
            let _ = std::fs::write(path, json);
        }
    }
}

impl Default for DenyLog {
    fn default() -> Self {
        Self::new()
    }
}
