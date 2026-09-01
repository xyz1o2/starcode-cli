pub mod extract;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use extract::{ExtractedMemory, MemoryExtractor, MemoryExtractorConfig, MemoryType};

/// Project memory - knowledge about the current project
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectMemory {
    /// File metadata (path -> description)
    pub files: HashMap<String, FileMeta>,
    /// Coding conventions
    pub conventions: Vec<Convention>,
    /// Dependencies between modules
    pub dependencies: HashMap<String, Vec<String>>,
    /// Last updated timestamp
    pub last_updated: i64,
}

/// Metadata about a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    /// Brief description of what this file does
    pub description: String,
    /// Key symbols/functions in this file
    pub symbols: Vec<String>,
    /// Last read timestamp
    pub last_read: i64,
    /// Last modified timestamp
    pub last_modified: i64,
    /// Number of times this file was accessed
    pub access_count: u32,
}

/// A coding convention observed in the project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Convention {
    /// Type of convention (indentation, naming, etc.)
    pub convention_type: String,
    /// The observed pattern
    pub pattern: String,
    /// Confidence level (0.0 - 1.0)
    pub confidence: f64,
    /// Examples where this was observed
    pub examples: Vec<String>,
}

/// User memory - preferences and history
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserMemory {
    /// User preferences
    pub preferences: HashMap<String, String>,
    /// Feedback history
    pub feedback: Vec<Feedback>,
    /// Coding style observations
    pub coding_style: CodingStyle,
    /// Last updated timestamp
    pub last_updated: i64,
}

/// User feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feedback {
    /// What the user said
    pub message: String,
    /// Whether it was positive or negative
    pub sentiment: Sentiment,
    /// Timestamp
    pub timestamp: i64,
}

/// Sentiment of feedback
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Sentiment {
    Positive,
    Negative,
    Neutral,
}

/// Observed coding style
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodingStyle {
    /// Indentation preference (tabs vs spaces)
    pub indentation: Option<String>,
    /// Quote style (single vs double)
    pub quote_style: Option<String>,
    /// Semicolon usage (for JS/TS)
    pub semicolons: Option<bool>,
    /// Naming convention (camelCase, snake_case, etc.)
    pub naming_convention: Option<String>,
    /// Line ending preference
    pub line_endings: Option<String>,
}

/// Error pattern library
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ErrorMemory {
    /// Patterns of errors encountered
    pub patterns: Vec<ErrorPattern>,
    /// Solutions that worked
    pub solutions: HashMap<String, Vec<String>>,
    /// Last updated timestamp
    pub last_updated: i64,
}

/// An error pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPattern {
    /// Error message pattern (regex or substring)
    pub pattern: String,
    /// Context where this error occurred
    pub context: String,
    /// Solution that worked
    pub solution: String,
    /// How many times this pattern was seen
    pub occurrence_count: u32,
    /// Last seen timestamp
    pub last_seen: i64,
}

/// Memory manager - handles loading and saving memory
pub struct MemoryManager {
    project_root: PathBuf,
    memory_dir: PathBuf,
}

impl MemoryManager {
    /// Create a new memory manager
    pub fn new(project_root: &Path) -> Self {
        let memory_dir = project_root.join(".star").join("memory");
        Self {
            project_root: project_root.to_path_buf(),
            memory_dir,
        }
    }

    /// Ensure memory directory exists
    async fn ensure_dir(&self) -> Result<(), String> {
        if !self.memory_dir.exists() {
            tokio::fs::create_dir_all(&self.memory_dir)
                .await
                .map_err(|e| format!("Failed to create memory directory: {}", e))?;
        }
        Ok(())
    }

    /// Load project memory
    pub async fn load_project_memory(&self) -> Result<ProjectMemory, String> {
        let path = self.memory_dir.join("project.json");
        if !path.exists() {
            return Ok(ProjectMemory::default());
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read project memory: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse project memory: {}", e))
    }

    /// Save project memory
    pub async fn save_project_memory(&self, memory: &ProjectMemory) -> Result<(), String> {
        self.ensure_dir().await?;
        let path = self.memory_dir.join("project.json");
        let content = serde_json::to_string_pretty(memory)
            .map_err(|e| format!("Failed to serialize project memory: {}", e))?;
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| format!("Failed to write project memory: {}", e))?;
        Ok(())
    }

    /// Load user memory
    pub async fn load_user_memory(&self) -> Result<UserMemory, String> {
        let path = self.memory_dir.join("user.json");
        if !path.exists() {
            return Ok(UserMemory::default());
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read user memory: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse user memory: {}", e))
    }

    /// Save user memory
    pub async fn save_user_memory(&self, memory: &UserMemory) -> Result<(), String> {
        self.ensure_dir().await?;
        let path = self.memory_dir.join("user.json");
        let content = serde_json::to_string_pretty(memory)
            .map_err(|e| format!("Failed to serialize user memory: {}", e))?;
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| format!("Failed to write user memory: {}", e))?;
        Ok(())
    }

    /// Load error memory
    pub async fn load_error_memory(&self) -> Result<ErrorMemory, String> {
        let path = self.memory_dir.join("errors.json");
        if !path.exists() {
            return Ok(ErrorMemory::default());
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read error memory: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse error memory: {}", e))
    }

    /// Save error memory
    pub async fn save_error_memory(&self, memory: &ErrorMemory) -> Result<(), String> {
        self.ensure_dir().await?;
        let path = self.memory_dir.join("errors.json");
        let content = serde_json::to_string_pretty(memory)
            .map_err(|e| format!("Failed to serialize error memory: {}", e))?;
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| format!("Failed to write error memory: {}", e))?;
        Ok(())
    }

    /// Record a file access
    pub async fn record_file_access(
        &self,
        file_path: &str,
        description: Option<&str>,
        symbols: Option<Vec<String>>,
    ) -> Result<(), String> {
        let mut memory = self.load_project_memory().await?;
        let now = chrono::Utc::now().timestamp();

        // Clone values before using them in closure
        let symbols_clone = symbols.clone();
        let desc_clone = description.map(|s| s.to_string());

        let entry = memory
            .files
            .entry(file_path.to_string())
            .or_insert_with(|| FileMeta {
                description: desc_clone.unwrap_or_default(),
                symbols: symbols_clone.unwrap_or_default(),
                last_read: now,
                last_modified: now,
                access_count: 0,
            });

        entry.last_read = now;
        entry.access_count += 1;
        if let Some(desc) = description {
            entry.description = desc.to_string();
        }
        if let Some(syms) = symbols {
            entry.symbols = syms;
        }

        memory.last_updated = now;
        self.save_project_memory(&memory).await
    }

    /// Record a user feedback
    pub async fn record_feedback(&self, message: &str, sentiment: Sentiment) -> Result<(), String> {
        let mut memory = self.load_user_memory().await?;
        let now = chrono::Utc::now().timestamp();

        memory.feedback.push(Feedback {
            message: message.to_string(),
            sentiment,
            timestamp: now,
        });

        // Keep only last 100 feedback entries
        if memory.feedback.len() > 100 {
            memory.feedback = memory.feedback.split_off(memory.feedback.len() - 100);
        }

        memory.last_updated = now;
        self.save_user_memory(&memory).await
    }

    /// Record an error pattern
    pub async fn record_error(
        &self,
        pattern: &str,
        context: &str,
        solution: &str,
    ) -> Result<(), String> {
        let mut memory = self.load_error_memory().await?;
        let now = chrono::Utc::now().timestamp();

        // Check if this pattern already exists
        if let Some(existing) = memory.patterns.iter_mut().find(|p| p.pattern == pattern) {
            existing.occurrence_count += 1;
            existing.last_seen = now;
            existing.solution = solution.to_string();
        } else {
            memory.patterns.push(ErrorPattern {
                pattern: pattern.to_string(),
                context: context.to_string(),
                solution: solution.to_string(),
                occurrence_count: 1,
                last_seen: now,
            });
        }

        // Keep only last 200 patterns
        if memory.patterns.len() > 200 {
            memory.patterns = memory.patterns.split_off(memory.patterns.len() - 200);
        }

        memory.last_updated = now;
        self.save_error_memory(&memory).await
    }

    /// Update user coding style observation
    pub async fn update_coding_style(&self, style: &CodingStyle) -> Result<(), String> {
        let mut memory = self.load_user_memory().await?;
        let now = chrono::Utc::now().timestamp();

        // Merge style observations
        if let Some(indent) = &style.indentation {
            memory.coding_style.indentation = Some(indent.clone());
        }
        if let Some(quote) = &style.quote_style {
            memory.coding_style.quote_style = Some(quote.clone());
        }
        if let Some(semi) = style.semicolons {
            memory.coding_style.semicolons = Some(semi);
        }
        if let Some(naming) = &style.naming_convention {
            memory.coding_style.naming_convention = Some(naming.clone());
        }
        if let Some(line_end) = &style.line_endings {
            memory.coding_style.line_endings = Some(line_end.clone());
        }

        memory.last_updated = now;
        self.save_user_memory(&memory).await
    }

    /// Get a summary of the memory state
    pub async fn get_summary(&self) -> Result<MemorySummary, String> {
        let project = self.load_project_memory().await?;
        let user = self.load_user_memory().await?;
        let errors = self.load_error_memory().await?;

        Ok(MemorySummary {
            files_tracked: project.files.len(),
            conventions_count: project.conventions.len(),
            feedback_count: user.feedback.len(),
            error_patterns_count: errors.patterns.len(),
            last_updated: project
                .last_updated
                .max(user.last_updated)
                .max(errors.last_updated),
        })
    }
}

/// Summary of memory state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySummary {
    pub files_tracked: usize,
    pub conventions_count: usize,
    pub feedback_count: usize,
    pub error_patterns_count: usize,
    pub last_updated: i64,
}

/// Format memory summary for display
pub fn format_memory_summary(summary: &MemorySummary) -> String {
    format!(
        "Memory: {} files tracked, {} conventions, {} feedback entries, {} error patterns (last updated: {})",
        summary.files_tracked,
        summary.conventions_count,
        summary.feedback_count,
        summary.error_patterns_count,
        chrono::DateTime::from_timestamp(summary.last_updated, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "never".to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_memory_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let manager = MemoryManager::new(dir.path());

        // Record file access
        manager
            .record_file_access(
                "src/main.rs",
                Some("Main entry point"),
                Some(vec!["main".to_string(), "init".to_string()]),
            )
            .await
            .unwrap();

        // Load and verify
        let memory = manager.load_project_memory().await.unwrap();
        assert_eq!(memory.files.len(), 1);
        assert_eq!(memory.files["src/main.rs"].description, "Main entry point");
        assert_eq!(memory.files["src/main.rs"].access_count, 1);
    }

    #[tokio::test]
    async fn test_error_recording() {
        let dir = tempfile::tempdir().unwrap();
        let manager = MemoryManager::new(dir.path());

        // Record error
        manager
            .record_error(
                "string not found",
                "replace operation",
                "re-read file first",
            )
            .await
            .unwrap();

        // Load and verify
        let memory = manager.load_error_memory().await.unwrap();
        assert_eq!(memory.patterns.len(), 1);
        assert_eq!(memory.patterns[0].pattern, "string not found");
        assert_eq!(memory.patterns[0].occurrence_count, 1);
    }
}
