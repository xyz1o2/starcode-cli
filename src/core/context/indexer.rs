use crate::core::utils::file_utils::read_file_with_encoding;
use ignore::WalkBuilder; // Use ignore crate instead of walkdir
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Blob {
    pub hash: String,
    pub path: String,
    #[serde(skip)]
    pub content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ProjectIndex {
    /// Mapping from File Path (relative) to Content Hash
    pub blobs: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct IndexResult {
    /// Blobs that are new or modified and need processing
    pub new_blobs: Vec<Blob>,
    /// Paths that were removed
    pub removed_blobs: Vec<String>,
    pub total_files: usize,
}

#[derive(Clone)]
pub struct Indexer {
    project_root: PathBuf,
    index_file: PathBuf,
    cas_dir: PathBuf,
}

impl Indexer {
    pub fn new(project_root: &Path) -> Self {
        let context_dir = project_root.join(".star").join("context");
        let index_file = context_dir.join("index.json");
        let cas_dir = context_dir.join("cas");
        Self {
            project_root: project_root.to_path_buf(),
            index_file,
            cas_dir,
        }
    }

    fn load_index(&self) -> ProjectIndex {
        if self.index_file.exists() {
            if let Ok(content) = fs::read_to_string(&self.index_file) {
                if let Ok(index) = serde_json::from_str(&content) {
                    return index;
                }
            }
        }
        ProjectIndex::default()
    }

    fn save_index(&self, index: &ProjectIndex) -> Result<(), std::io::Error> {
        if let Some(parent) = self.index_file.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(index)?;
        fs::write(&self.index_file, content)?;
        Ok(())
    }

    fn save_blob_to_cas(&self, blob: &Blob) -> Result<(), std::io::Error> {
        if !self.cas_dir.exists() {
            fs::create_dir_all(&self.cas_dir)?;
        }
        // Store by hash
        let blob_path = self.cas_dir.join(&blob.hash);
        if !blob_path.exists() {
            if let Some(content) = &blob.content {
                fs::write(blob_path, content)?;
            }
        }
        Ok(())
    }

    fn calculate_hash(&self, content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content);
        format!("{:x}", hasher.finalize())
    }

    pub fn index_project(&self) -> Result<IndexResult, Box<dyn std::error::Error + Send + Sync>> {
        if !self.project_root.exists() {
            return Ok(IndexResult {
                new_blobs: Vec::new(),
                removed_blobs: Vec::new(),
                total_files: 0,
            });
        }

        let mut current_index = self.load_index();
        let mut new_blobs = Vec::new();
        let mut found_paths = HashSet::new();

        // Three-layer ignore (same model as Claude Code):
        //   1. ~/.star/ignore   — user-level global, applies to every project
        //   2. .starignore      — project-level, committed to repo, team-shared
        //   3. .gitignore       — standard VCS ignore (require_git(false): honoured
        //                          even without a .git directory)
        // No hardcoded directory list.  Users configure exclusions in these files.
        let mut builder = WalkBuilder::new(&self.project_root);
        builder.hidden(false).git_ignore(true).require_git(false);
        // Layer 1: user-global ignore.
        if let Some(home) = dirs::home_dir() {
            let global_ignore = home.join(".star").join("ignore");
            if global_ignore.exists() {
                let _ = builder.add_ignore(&global_ignore);
            }
        }
        // Layer 2: project-level ignore.
        let project_ignore = self.project_root.join(".starignore");
        if project_ignore.exists() {
            let _ = builder.add_ignore(&project_ignore);
        }
        let walker = builder.build();

        for result in walker {
            match result {
                Ok(entry) => {
                    let path = entry.path();
                    if path.is_file() {
                        // Get relative path
                        let rel_path = match path.strip_prefix(&self.project_root) {
                            Ok(p) => p.to_string_lossy().replace("\\", "/"),
                            Err(_) => continue,
                        };

                        // println!("Indexing: {}", rel_path);
                        found_paths.insert(rel_path.clone());

                        // Read and Hash
                        match read_file_with_encoding(path) {
                            Ok(content) => {
                                let hash = self.calculate_hash(&content);
                                // println!("Hash for {}: {}", rel_path, hash);

                                // Check if changed
                                let is_new = match current_index.blobs.get(&rel_path) {
                                    Some(old_hash) => old_hash != &hash,
                                    None => true,
                                };

                                if is_new {
                                    current_index.blobs.insert(rel_path.clone(), hash.clone());
                                    let blob = Blob {
                                        hash,
                                        path: rel_path,
                                        content: Some(content),
                                    };

                                    // Save to CAS (simulating server-side stateless storage)
                                    if let Err(e) = self.save_blob_to_cas(&blob) {
                                        eprintln!("Failed to save blob to CAS: {}", e);
                                    }

                                    new_blobs.push(blob);
                                }
                            }
                            Err(_) => continue, // Skip unreadable files
                        }
                    }
                }
                Err(err) => crate::utils::logging::append_debug_log_line(&format!(
                    "[Context] Walk error while indexing {}: {}",
                    self.project_root.display(),
                    err
                )),
            }
        }

        // Identify removed files
        let mut removed_blobs = Vec::new();
        let old_paths: Vec<String> = current_index.blobs.keys().cloned().collect();
        for path in old_paths {
            if !found_paths.contains(&path) {
                current_index.blobs.remove(&path);
                removed_blobs.push(path);
            }
        }

        // Save updated index
        self.save_index(&current_index)?;

        Ok(IndexResult {
            new_blobs,
            removed_blobs,
            total_files: current_index.blobs.len(),
        })
    }
}
