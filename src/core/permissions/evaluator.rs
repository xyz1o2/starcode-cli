//! Permission evaluator for checking access rights

use super::rules::{PermissionEffect, PermissionRule};
use std::collections::HashMap;

/// Permission evaluator
pub struct PermissionEvaluator {
    rules: Vec<PermissionRule>,
    saved: HashMap<String, Vec<PermissionRule>>,
}

impl PermissionEvaluator {
    /// Create a new permission evaluator
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            saved: HashMap::new(),
        }
    }

    /// Create with initial rules
    pub fn with_rules(rules: Vec<PermissionRule>) -> Self {
        Self {
            rules,
            saved: HashMap::new(),
        }
    }

    /// Add a rule
    pub fn add_rule(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
    }

    /// Set rules for a project
    pub fn set_saved_rules(&mut self, project_id: &str, rules: Vec<PermissionRule>) {
        self.saved.insert(project_id.to_string(), rules);
    }

    /// Add a saved rule for a project
    pub fn add_saved_rule(&mut self, project_id: &str, rule: PermissionRule) {
        self.saved
            .entry(project_id.to_string())
            .or_insert_with(Vec::new)
            .push(rule);
    }

    /// Evaluate permission for an action on a resource
    pub fn evaluate(&self, action: &str, resource: &str, project_id: Option<&str>) -> PermissionEffect {
        // Check saved rules first (project-specific)
        if let Some(pid) = project_id {
            if let Some(rules) = self.saved.get(pid) {
                if let Some(effect) = self.check_rules(rules, action, resource) {
                    return effect;
                }
            }
        }

        // Check configured rules (global)
        if let Some(effect) = self.check_rules(&self.rules, action, resource) {
            return effect;
        }

        // Default: ask user
        PermissionEffect::Ask
    }

    /// Check a list of rules for a match
    fn check_rules(&self, rules: &[PermissionRule], action: &str, resource: &str) -> Option<PermissionEffect> {
        // Search from last to first (last match wins)
        for rule in rules.iter().rev() {
            if self.matches_pattern(&rule.action, action) && self.matches_pattern(&rule.resource, resource) {
                return Some(rule.effect.clone());
            }
        }
        None
    }

    /// Check if text matches a pattern (supports wildcards)
    fn matches_pattern(&self, pattern: &str, text: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            return text.starts_with(prefix);
        }
        
        if pattern.starts_with('*') {
            let suffix = &pattern[1..];
            return text.ends_with(suffix);
        }
        
        pattern == text
    }
}

impl Default for PermissionEvaluator {
    fn default() -> Self {
        Self::new()
    }
}
 