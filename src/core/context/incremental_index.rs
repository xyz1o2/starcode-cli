use super::indexer::Indexer;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Incremental index - only updates changed files
pub struct IncrementalIndex {
    /// The main indexer
    pub indexer: Indexer,
    /// File hashes for change detection
    pub file_hashes: HashMap<String, String>,
    /// Files that need reindexing
    pub dirty_files: HashSet<String>,
    /// Index version
    pub version: u64,
    /// Last indexed timestamp
    pub last_indexed: Option<i64>,
}

impl IncrementalIndex {
    pub fn new(project_root: &Path) -> Self {
        Self {
            indexer: Indexer::new(project_root),
            file_hashes: HashMap::new(),
            dirty_files: HashSet::new(),
            version: 0,
            last_indexed: None,
        }
    }

    /// Check if a file needs reindexing
    pub fn needs_reindex(&self, path: &str, content: &str) -> bool {
        let new_hash = format!("{:x}", md5::compute(content));

        match self.file_hashes.get(path) {
            Some(old_hash) => *old_hash != new_hash,
            None => true,
        }
    }

    /// Index only changed files
    pub async fn index_changed(&mut self, repo_path: &str) -> Result<usize, String> {
        let changed_files = self.get_changed_files(repo_path).await?;
        let mut indexed = 0;

        for file_path in changed_files {
            let full_path = format!("{}/{}", repo_path, file_path);

            let content = match tokio::fs::read_to_string(&full_path).await {
                Ok(c) => c,
                Err(_) => continue,
            };

            if !self.needs_reindex(&file_path, &content) {
                continue;
            }

            let new_hash = format!("{:x}", md5::compute(&content));
            self.file_hashes.insert(file_path.clone(), new_hash);
            self.dirty_files.remove(&file_path);

            indexed += 1;
        }

        if indexed > 0 {
            self.version += 1;
            self.last_indexed = Some(chrono::Utc::now().timestamp());
        }

        Ok(indexed)
    }

    /// Full index of all files
    pub fn full_index(&mut self) -> Result<usize, String> {
        let result = self.indexer.index_project().map_err(|e| e.to_string())?;
        self.version += 1;
        self.last_indexed = Some(chrono::Utc::now().timestamp());
        Ok(result.new_blobs.len())
    }

    /// Clone the inner Indexer so callers can run `index_project()`
    /// on a `spawn_blocking` thread without holding the async RwLock.
    pub fn clone_indexer(&self) -> Indexer {
        self.indexer.clone()
    }

    async fn get_changed_files(&self, repo_path: &str) -> Result<Vec<String>, String> {
        let output = tokio::process::Command::new("git")
            .args(["diff", "--name-only", "HEAD~10"])
            .current_dir(repo_path)
            .output()
            .await
            .map_err(|e| format!("Git error: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<String> = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();

        Ok(files)
    }

    /// Get index status summary
    pub fn get_status(&self) -> IndexStatus {
        IndexStatus {
            version: self.version,
            total_files: self.file_hashes.len(),
            dirty_files: self.dirty_files.len(),
            last_indexed: self.last_indexed,
        }
    }
}

pub struct IndexStatus {
    pub version: u64,
    pub total_files: usize,
    pub dirty_files: usize,
    pub last_indexed: Option<i64>,
}

impl std::fmt::Display for IndexStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Index v{}: {} files indexed, {} dirty, last updated: {}",
            self.version,
            self.total_files,
            self.dirty_files,
            self.last_indexed
                .map(|ts| chrono::DateTime::from_timestamp(ts, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "unknown".to_string()))
                .unwrap_or_else(|| "never".to_string())
        )
    }
}
