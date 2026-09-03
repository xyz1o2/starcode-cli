use crate::core::tools::constants::canonical_tool_name;
use crate::core::tools::tools::{BaseDeclarativeTool, FunctionDeclaration};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

pub struct ToolRegistry {
    all_known_tools: RwLock<HashMap<String, Arc<dyn BaseDeclarativeTool>>>,
    plugin_tool_names: RwLock<HashSet<String>>,
    generation: AtomicU64,
    cached_function_declarations: RwLock<Option<(u64, Vec<FunctionDeclaration>)>>,
    config: Arc<crate::core::config::Config>,
}

impl ToolRegistry {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self {
            all_known_tools: RwLock::new(HashMap::new()),
            plugin_tool_names: RwLock::new(HashSet::new()),
            generation: AtomicU64::new(0),
            cached_function_declarations: RwLock::new(None),
            config,
        }
    }

    pub fn register_tool(&self, tool: Arc<dyn BaseDeclarativeTool>) {
        let mut map = self.all_known_tools.write().unwrap();
        let name = tool.name().to_string();
        map.insert(name.clone(), tool);
        if let Ok(mut plugin_names) = self.plugin_tool_names.write() {
            plugin_names.remove(&name);
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut cache) = self.cached_function_declarations.write() {
            *cache = None;
        }
    }

    pub fn sync_plugin_tools(&self, tools: Vec<Arc<dyn BaseDeclarativeTool>>) -> Vec<String> {
        let mut map = self.all_known_tools.write().unwrap();
        let mut plugin_names = self.plugin_tool_names.write().unwrap();
        let mut skipped = Vec::new();
        let mut seen = HashSet::new();

        for name in plugin_names.iter() {
            map.remove(name);
        }
        plugin_names.clear();

        for tool in tools {
            let name = tool.name().to_string();
            if !seen.insert(name.clone()) {
                skipped.push(name.clone());
                crate::utils::logging::append_debug_log_line(&format!(
                    "[PluginTools] Skip duplicate plugin tool `{}` in same sync batch",
                    name
                ));
                continue;
            }

            let canonical = canonical_tool_name(&name);
            let exact_conflict = map.contains_key(&name);
            let canonical_conflict = canonical != name && map.contains_key(&canonical);
            if exact_conflict || canonical_conflict {
                let conflict_name = if exact_conflict { &name } else { &canonical };
                skipped.push(name.clone());
                crate::utils::logging::append_debug_log_line(&format!(
                    "[PluginTools] Skip plugin tool `{}` because it conflicts with existing tool `{}`",
                    name, conflict_name
                ));
                continue;
            }

            map.insert(name.clone(), tool);
            plugin_names.insert(name);
        }

        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut cache) = self.cached_function_declarations.write() {
            *cache = None;
        }

        skipped
    }

    pub fn get_config(&self) -> Arc<crate::core::config::Config> {
        self.config.clone()
    }

    /// Returns all registered tools as (name, description, parameter_schema) tuples
    /// Used by ToolSearchTool to search across all tools.
    pub fn get_all_tool_entries(&self) -> Vec<(String, String, serde_json::Value)> {
        let map = self.all_known_tools.read().unwrap();
        map.iter()
            .map(|(name, tool)| {
                (
                    name.clone(),
                    tool.description().to_string(),
                    tool.parameter_schema(),
                )
            })
            .collect()
    }

    pub fn get_tool(&self, name: &str) -> Option<Arc<dyn BaseDeclarativeTool>> {
        let map = self.all_known_tools.read().unwrap();
        map.get(name).cloned().or_else(|| {
            let canonical = canonical_tool_name(name);
            map.get(canonical.as_str()).cloned()
        })
    }

    pub fn sort_tools(&self) {
        // 目前不做排序；必要时可在 get_function_declarations 中排序
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn get_function_declarations(&self) -> Vec<FunctionDeclaration> {
        let generation = self.generation();
        if let Ok(cache) = self.cached_function_declarations.read() {
            if let Some((cached_generation, declarations)) = cache.as_ref() {
                if *cached_generation == generation {
                    return declarations.clone();
                }
            }
        }

        let map = self.all_known_tools.read().unwrap();
        let mut declarations = map
            .values()
            .map(|tool| {
                let schema = tool.parameter_schema();
                FunctionDeclaration {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    parameters: schema.clone(),
                    parameters_json_schema: schema,
                }
            })
            .collect::<Vec<_>>();

        drop(map);

        // 按名称排序：all_known_tools 是 HashMap，迭代顺序随进程随机。
        // 排序后同一套工具在任何一次启动中都序列化成同一段 JSON，
        // 使 tools 数组可复用 prompt 缓存前缀（跨会话亦然）。
        declarations.sort_by(|a, b| a.name.cmp(&b.name));

        if let Ok(mut cache) = self.cached_function_declarations.write() {
            *cache = Some((generation, declarations.clone()));
        }

        declarations
    }
}
