use super::types::ContextDefinition;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct ContextCache {
    cache: Arc<Mutex<HashMap<String, ContextDefinition>>>,
}

impl ContextCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get(&self, id: &str) -> Option<ContextDefinition> {
        let cache = self.cache.lock().unwrap();
        cache.get(id).cloned()
    }

    pub fn put(&self, context: ContextDefinition) {
        let mut cache = self.cache.lock().unwrap();
        cache.insert(context.id.clone(), context);
    }

    pub fn remove(&self, id: &str) {
        let mut cache = self.cache.lock().unwrap();
        cache.remove(id);
    }

    pub fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }
}

pub struct ContextStore {
    // Placeholder for persistent storage (e.g. SQLite or JSON files)
}

impl ContextStore {
    pub fn new() -> Self {
        Self {}
    }
}
