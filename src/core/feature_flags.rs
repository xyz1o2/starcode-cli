use std::collections::HashMap;

pub struct FeatureFlags {
    flags: HashMap<String, FeatureFlag>,
}

pub struct FeatureFlag {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub rollout_percentage: u8, // 0-100
    pub allowed_users: Vec<String>,
    pub denied_users: Vec<String>,
}

impl FeatureFlags {
    pub fn new() -> Self {
        let mut flags = Self {
            flags: HashMap::new(),
        };
        flags.register_defaults();
        flags
    }

    fn register_defaults(&mut self) {
        self.register("vim_mode", "Enable Vim keybindings", true, 100);
        self.register("voice_mode", "Enable voice input/output", false, 0);
        self.register(
            "proactive_suggestions",
            "Enable proactive suggestions",
            true,
            50,
        );
        self.register("context_collapse", "Enable context collapse", true, 100);
        self.register("auto_compact", "Enable auto context compression", true, 100);
    }

    pub fn register(&mut self, name: &str, desc: &str, enabled: bool, rollout: u8) {
        self.flags.insert(
            name.to_string(),
            FeatureFlag {
                name: name.to_string(),
                description: desc.to_string(),
                enabled,
                rollout_percentage: rollout,
                allowed_users: Vec::new(),
                denied_users: Vec::new(),
            },
        );
    }

    pub fn is_enabled(&self, name: &str, user_id: Option<&str>) -> bool {
        self.flags
            .get(name)
            .map(|flag| {
                if !flag.enabled {
                    return false;
                }
                if let Some(uid) = user_id {
                    if flag.denied_users.contains(&uid.to_string()) {
                        return false;
                    }
                    if !flag.allowed_users.is_empty() {
                        return flag.allowed_users.contains(&uid.to_string());
                    }
                }
                flag.rollout_percentage >= 100
            })
            .unwrap_or(false)
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(flag) = self.flags.get_mut(name) {
            flag.enabled = enabled;
        }
    }

    pub fn list_flags(&self) -> Vec<(&str, bool)> {
        self.flags
            .iter()
            .map(|(k, v)| (k.as_str(), v.enabled))
            .collect()
    }

    pub fn get_flag(&self, name: &str) -> Option<&FeatureFlag> {
        self.flags.get(name)
    }

    pub fn add_allowed_user(&mut self, name: &str, user_id: &str) {
        if let Some(flag) = self.flags.get_mut(name) {
            if !flag.allowed_users.contains(&user_id.to_string()) {
                flag.allowed_users.push(user_id.to_string());
            }
        }
    }

    pub fn add_denied_user(&mut self, name: &str, user_id: &str) {
        if let Some(flag) = self.flags.get_mut(name) {
            if !flag.denied_users.contains(&user_id.to_string()) {
                flag.denied_users.push(user_id.to_string());
            }
        }
    }

    pub fn remove_user(&mut self, name: &str, user_id: &str) {
        if let Some(flag) = self.flags.get_mut(name) {
            flag.allowed_users.retain(|u| u != user_id);
            flag.denied_users.retain(|u| u != user_id);
        }
    }
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self::new()
    }
}
