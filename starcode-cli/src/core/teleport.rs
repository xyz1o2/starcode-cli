pub struct TeleportManager {
    pub endpoint: Option<String>,
    pub sessions: Vec<TeleportSession>,
}

pub struct TeleportSession {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub status: TeleportStatus,
    pub created_at: i64,
}

#[derive(Debug)]
pub enum TeleportStatus {
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

impl TeleportManager {
    pub fn new() -> Self {
        Self {
            endpoint: None,
            sessions: Vec::new(),
        }
    }

    pub async fn connect(&mut self, endpoint: &str, name: &str) -> Result<String, String> {
        let session = TeleportSession {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            endpoint: endpoint.to_string(),
            status: TeleportStatus::Connecting,
            created_at: chrono::Utc::now().timestamp(),
        };

        let id = session.id.clone();
        self.sessions.push(session);
        self.endpoint = Some(endpoint.to_string());

        Ok(id)
    }

    pub async fn disconnect(&mut self, session_id: &str) -> Result<(), String> {
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.status = TeleportStatus::Disconnected;
        }
        Ok(())
    }

    pub async fn send_message(&self, session_id: &str, _message: &str) -> Result<(), String> {
        if self.sessions.iter().any(|s| s.id == session_id) {
            Ok(())
        } else {
            Err(format!("Session not found: {}", session_id))
        }
    }

    pub fn list_sessions(&self) -> &[TeleportSession] {
        &self.sessions
    }
}
