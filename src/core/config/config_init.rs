use crate::core::config::Config;

impl Config {
    pub async fn initialize(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.initialized {
            return Err("Config was already initialized".into());
        }
        let bootstrap =
            crate::core::config::runtime_bootstrap::build_runtime_services(self, None).await?;
        self.install_runtime_services(bootstrap.services);

        self.initialized = true;
        Ok(())
    }
}
