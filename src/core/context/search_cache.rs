// ── Search Engine Cache Manager ──────────────────────────────────────────────
//
// Replaces the global static Mutex caches that were in semantic_search.rs.
// Uses parking_lot::RwLock (no poisoning) + LRU eviction for bounded memory.
//
// Architecture decision: caches are owned by ContextEngine and passed to
// semantic search functions via Arc. When None (standalone/test), caching
// is skipped entirely and search engine is built fresh each call.

use super::search_engine::SearchEngine;
use lru::LruCache;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::SystemTime;

/// Cached search engine with validity signal (mtime of index.json).
#[derive(Clone)]
pub struct CachedSearchEngine {
    pub engine: SearchEngine,
    pub index_mtime: Option<SystemTime>,
}

/// Maximum number of cached search engines (one per project + budget profile).
const MAX_CACHED_ENGINES: usize = 3;
/// Maximum number of cached query results.
const MAX_CACHED_QUERIES: usize = 64;

/// Thread-safe cache manager for search engines and query results.
///
/// Uses parking_lot locks exclusively: they do not track poisoning, so a
/// panic in a lock-holding thread simply releases the lock. This is the
/// correct behaviour for caches where data can always be rebuilt.
pub struct SearchEngineCacheManager {
    /// Cached pre-built SearchEngines, keyed by (canonical_project_root, budget_profile_key).
    engine_cache: RwLock<HashMap<(PathBuf, String), CachedSearchEngine>>,
    /// LRU query result cache, keyed by (canonical_root, query, index_mtime).
    query_cache: RwLock<LruCache<(PathBuf, String, Option<SystemTime>), String>>,
}

impl SearchEngineCacheManager {
    pub fn new() -> Self {
        Self {
            engine_cache: RwLock::new(HashMap::new()),
            query_cache: RwLock::new(LruCache::new(
                NonZeroUsize::new(MAX_CACHED_QUERIES).unwrap(),
            )),
        }
    }

    /// Look up a cached search engine. Returns None if not found or mtime mismatch.
    pub fn get_engine(
        &self,
        key: &(PathBuf, String),
        current_mtime: Option<SystemTime>,
    ) -> Option<SearchEngine> {
        let cache = self.engine_cache.read();
        cache.get(key).and_then(|cached| {
            if cached.index_mtime == current_mtime && current_mtime.is_some() {
                Some(cached.engine.clone())
            } else {
                None
            }
        })
    }

    /// Store a search engine in the cache. Evicts oldest entry if at capacity.
    pub fn put_engine(&self, key: (PathBuf, String), cached: CachedSearchEngine) {
        let mut cache = self.engine_cache.write();
        if cache.len() >= MAX_CACHED_ENGINES && !cache.contains_key(&key) {
            // Evict a random entry (simple FIFO via drain)
            if let Some(oldest_key) = cache.iter().next().map(|(k, _)| k.clone()) {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(key, cached);
    }

    /// Look up a cached query result.
    pub fn get_query(&self, key: &(PathBuf, String, Option<SystemTime>)) -> Option<String> {
        self.query_cache.write().get(key).cloned()
    }

    /// Store a query result in the LRU cache.
    pub fn put_query(&self, key: (PathBuf, String, Option<SystemTime>), output: String) {
        self.query_cache.write().put(key, output);
    }

    /// Clear all caches.
    pub fn clear(&self) {
        self.engine_cache.write().clear();
        self.query_cache.write().clear();
    }
}

impl Default for SearchEngineCacheManager {
    fn default() -> Self {
        Self::new()
    }
}
