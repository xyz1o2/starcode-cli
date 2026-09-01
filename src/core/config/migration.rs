pub struct ConfigMigration {
    pub version: u32,
    pub description: String,
    pub migrate: fn(&mut serde_json::Value) -> Result<(), String>,
}

pub struct MigrationRunner {
    migrations: Vec<ConfigMigration>,
}

impl MigrationRunner {
    pub fn new() -> Self {
        let mut runner = Self {
            migrations: Vec::new(),
        };
        runner.register_migrations();
        runner
    }

    fn register_migrations(&mut self) {
        // Migration 1: Add version field
        self.migrations.push(ConfigMigration {
            version: 1,
            description: "Add config version field".to_string(),
            migrate: |config| {
                if config.get("version").is_none() {
                    config["version"] = serde_json::json!(1);
                }
                Ok(())
            },
        });

        // Migration 2: Rename old fields
        self.migrations.push(ConfigMigration {
            version: 2,
            description: "Rename api_key to provider_api_key".to_string(),
            migrate: |config| {
                if let Some(key) = config.get("api_key").cloned() {
                    config["provider_api_key"] = key;
                    config.as_object_mut().unwrap().remove("api_key");
                }
                Ok(())
            },
        });

        // Add more migrations as needed...
    }

    pub fn run(&self, config: &mut serde_json::Value) -> Result<u32, String> {
        let current_version = config.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        let mut applied = 0;
        for migration in &self.migrations {
            if migration.version > current_version {
                (migration.migrate)(config)?;
                config["version"] = serde_json::json!(migration.version);
                applied += 1;
            }
        }
        Ok(applied)
    }

    pub fn get_pending_migrations(&self, config: &serde_json::Value) -> Vec<&ConfigMigration> {
        let current_version = config.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        self.migrations
            .iter()
            .filter(|m| m.version > current_version)
            .collect()
    }

    pub fn current_version(&self, config: &serde_json::Value) -> u32 {
        config.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32
    }
}

impl Default for MigrationRunner {
    fn default() -> Self {
        Self::new()
    }
}
