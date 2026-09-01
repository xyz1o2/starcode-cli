use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSettings {
    pub endpoint: Option<String>,
    pub settings: serde_json::Value,
    pub last_sync: Option<i64>,
    pub sync_interval_secs: u64,
}

impl RemoteSettings {
    pub fn new() -> Self {
        Self {
            endpoint: None,
            settings: serde_json::json!({}),
            last_sync: None,
            sync_interval_secs: 300,
        }
    }

    pub fn set_endpoint(&mut self, endpoint: &str) {
        self.endpoint = Some(endpoint.to_string());
    }

    pub async fn sync(&mut self) -> Result<(), String> {
        let endpoint = match &self.endpoint {
            Some(e) => e.clone(),
            None => return Ok(()),
        };

        if let Some(last) = self.last_sync {
            let elapsed = chrono::Utc::now().timestamp() - last;
            if elapsed < self.sync_interval_secs as i64 {
                return Ok(());
            }
        }

        match reqwest::get(&endpoint).await {
            Ok(response) => {
                if let Ok(settings) = response.json::<serde_json::Value>().await {
                    self.settings = settings;
                    self.last_sync = Some(chrono::Utc::now().timestamp());
                }
            }
            Err(e) => {
                return Err(format!("Failed to sync settings: {}", e));
            }
        }

        Ok(())
    }

    pub fn get_setting(&self, path: &str) -> Option<&serde_json::Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = &self.settings;

        for part in parts {
            current = current.get(part)?;
        }

        Some(current)
    }

    pub fn get_policy_limit(&self, key: &str) -> Option<serde_json::Value> {
        self.get_setting(&format!("policy_limits.{}", key)).cloned()
    }
}
