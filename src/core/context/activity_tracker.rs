use std::collections::HashMap;

/// Tracks code activity based on git history
pub struct ActivityTracker {
    /// File activity scores: path -> score (0.0 = inactive, 1.0 = very active)
    pub file_activity: HashMap<String, f64>,
    /// Function activity: function_name -> score
    pub function_activity: HashMap<String, f64>,
    /// Last git commit timestamps per file
    pub last_commits: HashMap<String, i64>,
    /// Commit counts per file (last 30 days)
    pub commit_counts: HashMap<String, usize>,
}

impl ActivityTracker {
    pub fn new() -> Self {
        Self {
            file_activity: HashMap::new(),
            function_activity: HashMap::new(),
            last_commits: HashMap::new(),
            commit_counts: HashMap::new(),
        }
    }

    /// Update activity from git log
    pub async fn update_from_git(&mut self, repo_path: &str) -> Result<(), String> {
        let output = tokio::process::Command::new("git")
            .args(["log", "--name-only", "--pretty=format:%H %ct", "-100"])
            .current_dir(repo_path)
            .output()
            .await
            .map_err(|e| format!("Failed to run git: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let now = chrono::Utc::now().timestamp();
        let thirty_days_ago = now - 30 * 24 * 3600;

        let mut current_commit_time: i64 = 0;

        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }

            if let Some((_, ts_str)) = line.split_once(' ') {
                if let Ok(ts) = ts_str.parse::<i64>() {
                    current_commit_time = ts;
                }
                continue;
            }

            let file = line.trim();
            if file.is_empty() {
                continue;
            }

            let last = self.last_commits.entry(file.to_string()).or_insert(0);
            if current_commit_time > *last {
                *last = current_commit_time;
            }

            if current_commit_time > thirty_days_ago {
                *self.commit_counts.entry(file.to_string()).or_insert(0) += 1;
            }
        }

        self.calculate_scores(now);

        Ok(())
    }

    fn calculate_scores(&mut self, now: i64) {
        let max_commits = self.commit_counts.values().max().copied().unwrap_or(1);
        let thirty_days = 30 * 24 * 3600;

        for (file, last_commit) in &self.last_commits {
            let recency = ((now - last_commit) as f64 / thirty_days as f64).min(1.0);
            let frequency =
                self.commit_counts.get(file).copied().unwrap_or(0) as f64 / max_commits as f64;

            let score = (1.0 - recency) * 0.7 + frequency * 0.3;
            self.file_activity.insert(file.clone(), score);
        }
    }

    /// Check if a file is active (modified in last 7 days)
    pub fn is_file_active(&self, path: &str) -> bool {
        self.file_activity.get(path).copied().unwrap_or(0.0) > 0.3
    }

    /// Get activity score for a file
    pub fn get_file_score(&self, path: &str) -> f64 {
        self.file_activity.get(path).copied().unwrap_or(0.0)
    }

    /// Get top N most active files
    pub fn get_most_active(&self, n: usize) -> Vec<(&str, f64)> {
        let mut files: Vec<(&str, f64)> = self
            .file_activity
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        files.into_iter().take(n).collect()
    }
}
