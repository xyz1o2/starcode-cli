use super::types::{HookEvent, HookResult};
use serde_json::Value;
use std::collections::HashMap;

pub type FunctionHook = Box<dyn Fn(&Value) -> HookResult + Send + Sync>;

pub struct FunctionHookRegistry {
    hooks: HashMap<HookEvent, Vec<(String, FunctionHook)>>,
}

impl FunctionHookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    pub fn register(&mut self, event: HookEvent, name: impl Into<String>, hook: FunctionHook) {
        self.hooks
            .entry(event)
            .or_default()
            .push((name.into(), hook));
    }

    pub fn run_all(&self, event: HookEvent, input: &Value) -> Vec<HookResult> {
        let Some(hooks) = self.hooks.get(&event) else {
            return Vec::new();
        };

        hooks.iter().map(|(_, hook)| hook(input)).collect()
    }

    pub fn has_hooks(&self, event: &HookEvent) -> bool {
        self.hooks
            .get(event)
            .map(|h| !h.is_empty())
            .unwrap_or(false)
    }

    pub fn hook_names(&self, event: &HookEvent) -> Vec<&str> {
        self.hooks
            .get(event)
            .map(|hooks| hooks.iter().map(|(name, _)| name.as_str()).collect())
            .unwrap_or_default()
    }

    pub fn remove(&mut self, event: &HookEvent, name: &str) -> bool {
        if let Some(hooks) = self.hooks.get_mut(event) {
            let before = hooks.len();
            hooks.retain(|(n, _)| n != name);
            hooks.len() < before
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.hooks.clear();
    }
}

impl Default for FunctionHookRegistry {
    fn default() -> Self {
        Self::new()
    }
}
 