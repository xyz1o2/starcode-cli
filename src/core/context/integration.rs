use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::activity_tracker::ActivityTracker;
use super::incremental_index::IncrementalIndex;
use super::precise_retrieval::{PreciseRetriever, RetrievalResult};
use super::structure_index::{EditContext, FunctionDef, StructureIndex};

/// Integrated context engine - combines all context features
pub struct ContextEngine {
    /// Code structure index
    pub structure_index: Arc<RwLock<StructureIndex>>,
    /// Precise retriever
    pub retriever: Arc<RwLock<PreciseRetriever>>,
    /// Activity tracker
    pub activity: Arc<RwLock<ActivityTracker>>,
    /// Incremental indexer
    pub indexer: Arc<RwLock<IncrementalIndex>>,
    /// Project root path
    pub project_root: PathBuf,
    /// Whether initial indexing is complete
    pub indexed: bool,
}

impl ContextEngine {
    pub fn new(project_root: PathBuf) -> Self {
        let structure_index = StructureIndex::new();
        let retriever = PreciseRetriever::new(structure_index.clone());
        let activity = ActivityTracker::new();
        let indexer = IncrementalIndex::new(&project_root);

        Self {
            structure_index: Arc::new(RwLock::new(structure_index)),
            retriever: Arc::new(RwLock::new(retriever)),
            activity: Arc::new(RwLock::new(activity)),
            indexer: Arc::new(RwLock::new(indexer)),
            project_root,
            indexed: false,
        }
    }

    /// Index the project (full index on startup)
    ///
    /// Heavy synchronous I/O (filesystem walks, file reads, SHA-256 hashing)
    /// is offloaded to `spawn_blocking` threads so the Tokio async runtime
    /// stays responsive — this prevents the TUI from freezing on large projects.
    pub async fn index_project(&mut self) -> Result<IndexResult, String> {
        let start = std::time::Instant::now();
        let root = self.project_root.clone();

        // 1. Run full index — clone the Indexer (lightweight, it only holds
        //    PathBuf fields) and run the synchronous tree walk on a blocking
        //    thread so we don't starve the async runtime.
        let indexer = {
            let inc = self.indexer.read().await;
            inc.clone_indexer()
        };
        let index_result = tokio::task::spawn_blocking(move || indexer.index_project())
            .await
            .map_err(|e| format!("Blocking task join error: {}", e))?
            .map_err(|e| e.to_string())?;
        let files_indexed = index_result.new_blobs.len();

        // Update incremental-index bookkeeping on the async side
        {
            let mut inc = self.indexer.write().await;
            inc.version += 1;
            inc.last_indexed = Some(chrono::Utc::now().timestamp());
        }

        // 2. Update activity tracker from git (already async)
        let mut activity = self.activity.write().await;
        let _ = activity.update_from_git(&root.to_string_lossy()).await;
        drop(activity);

        // 3. Build structure index from indexed files — also on a blocking
        //    thread.  A fresh StructureIndex is built from scratch and then
        //    swapped in atomically, so readers are never blocked during the walk.
        let structure = tokio::task::spawn_blocking(move || {
            let mut structure = StructureIndex::new();

            let walker = ignore::WalkBuilder::new(&root)
                .hidden(false)
                .git_ignore(true)
                .build();

            for entry in walker {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                // Only index source files
                if !is_source_file(&relative) {
                    continue;
                }

                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                structure.index_file(&relative, &content);
            }

            structure.last_indexed = Some(chrono::Utc::now().timestamp());

            // Compute totals once after all files are indexed (O(n) instead
            // of the previous O(n²) pattern that summed inside the loop).
            let functions_found: usize = structure.functions.values().map(|v| v.len()).sum();
            let types_found: usize = structure.types.values().map(|v| v.len()).sum();

            (structure, functions_found, types_found)
        })
        .await
        .map_err(|e| format!("Blocking task join error: {}", e))?;

        let (new_structure, functions_found, types_found) = structure;

        // 4. Swap in the new structure index and rebuild the retriever
        {
            let mut structure_guard = self.structure_index.write().await;
            *structure_guard = new_structure;
        }
        {
            let structure_guard = self.structure_index.read().await;
            let mut retriever = self.retriever.write().await;
            *retriever = PreciseRetriever::new(structure_guard.clone());
        }

        self.indexed = true;
        let elapsed = start.elapsed();

        Ok(IndexResult {
            files_indexed,
            functions_found,
            types_found,
            elapsed_ms: elapsed.as_millis() as u64,
        })
    }

    /// Get context for editing a specific function
    pub async fn get_edit_context(&self, function_name: &str) -> Option<EditContext> {
        let structure = self.structure_index.read().await;
        Some(structure.get_edit_context(function_name))
    }

    /// Get precise retrieval result for a file
    pub async fn get_file_context(&self, file_path: &str) -> RetrievalResult {
        let retriever = self.retriever.read().await;
        retriever.for_file(file_path)
    }

    /// Get precise retrieval result for editing a function
    pub async fn get_function_context(
        &self,
        function_name: &str,
        file_path: &str,
    ) -> RetrievalResult {
        let retriever = self.retriever.read().await;
        retriever.for_edit(function_name, file_path)
    }

    /// Search for functions matching a query
    pub async fn search_functions(&self, query: &str) -> Vec<FunctionDef> {
        let structure = self.structure_index.read().await;
        structure
            .search_functions(query)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Find all references to a function
    pub async fn find_references(
        &self,
        function_name: &str,
    ) -> Vec<super::structure_index::Reference> {
        let structure = self.structure_index.read().await;
        structure.find_references(function_name)
    }

    /// Update index for a single file (called after file edit)
    pub async fn update_file(&self, file_path: &str, content: &str) {
        let mut structure = self.structure_index.write().await;
        structure.index_file(file_path, content);
        drop(structure);

        // Update retriever
        let structure = self.structure_index.read().await;
        let mut retriever = self.retriever.write().await;
        *retriever = PreciseRetriever::new(structure.clone());
    }

    /// Get activity score for a file
    pub async fn get_file_activity(&self, file_path: &str) -> f64 {
        let activity = self.activity.read().await;
        activity.get_file_score(file_path)
    }

    /// Check if a file is active
    pub async fn is_file_active(&self, file_path: &str) -> bool {
        let activity = self.activity.read().await;
        activity.is_file_active(file_path)
    }

    /// Get index status
    pub async fn get_status(&self) -> String {
        let structure = self.structure_index.read().await;
        let activity = self.activity.read().await;
        let indexer = self.indexer.read().await;

        let functions = structure.functions.values().map(|v| v.len()).sum::<usize>();
        let types = structure.types.values().map(|v| v.len()).sum::<usize>();
        let active_files = activity
            .file_activity
            .values()
            .filter(|&&v| v > 0.3)
            .count();

        format!(
            "Index: {} functions, {} types, {} active files | {}",
            functions,
            types,
            active_files,
            indexer.get_status()
        )
    }
}

#[derive(Debug)]
pub struct IndexResult {
    pub files_indexed: usize,
    pub functions_found: usize,
    pub types_found: usize,
    pub elapsed_ms: u64,
}

impl std::fmt::Display for IndexResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Indexed {} files: {} functions, {} types in {}ms",
            self.files_indexed, self.functions_found, self.types_found, self.elapsed_ms
        )
    }
}

fn is_source_file(path: &str) -> bool {
    let extensions = [
        "rs", "py", "js", "jsx", "ts", "tsx", "go", "java", "cpp", "c", "h", "hpp",
    ];
    path.split('.')
        .last()
        .map(|ext| extensions.contains(&ext))
        .unwrap_or(false)
}
