pub struct DeepLinkHandler {
    pub registered: bool,
    pub protocol: String,
}

#[derive(Debug)]
pub enum DeepLinkAction {
    OpenFile(String),
    ResumeSession(String),
    RunCommand(String),
}

impl DeepLinkHandler {
    pub fn new() -> Self {
        Self {
            registered: false,
            protocol: "cc".to_string(),
        }
    }

    pub fn register(&mut self) -> Result<(), String> {
        self.registered = true;
        Ok(())
    }

    pub fn unregister(&mut self) -> Result<(), String> {
        self.registered = false;
        Ok(())
    }

    pub fn parse_url(&self, url: &str) -> Option<DeepLinkAction> {
        if !url.starts_with("cc://") {
            return None;
        }

        let path = &url[5..];

        match path.split('/').next()? {
            "open" => {
                let file = path.strip_prefix("open/")?;
                Some(DeepLinkAction::OpenFile(file.to_string()))
            }
            "session" => {
                let id = path.strip_prefix("session/")?;
                Some(DeepLinkAction::ResumeSession(id.to_string()))
            }
            "run" => {
                let cmd = path.strip_prefix("run/")?;
                Some(DeepLinkAction::RunCommand(cmd.to_string()))
            }
            _ => None,
        }
    }

    pub fn handle(&self, action: DeepLinkAction) -> Result<String, String> {
        match action {
            DeepLinkAction::OpenFile(path) => {
                Ok(format!("Opening file: {}", path))
            }
            DeepLinkAction::ResumeSession(id) => {
                Ok(format!("Resuming session: {}", id))
            }
            DeepLinkAction::RunCommand(cmd) => {
                Ok(format!("Running command: {}", cmd))
            }
        }
    }
}
