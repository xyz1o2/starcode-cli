use chrono::Utc;

pub struct NotificationManager {
    notifications: Vec<Notification>,
    max_notifications: usize,
}

pub struct Notification {
    pub id: String,
    pub title: String,
    pub message: String,
    pub notification_type: NotificationType,
    pub timestamp: i64,
    pub read: bool,
}

pub enum NotificationType {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationManager {
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
            max_notifications: 100,
        }
    }

    pub fn add(&mut self, title: &str, message: &str, ntype: NotificationType) {
        let notification = Notification {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            message: message.to_string(),
            notification_type: ntype,
            timestamp: Utc::now().timestamp(),
            read: false,
        };

        self.notifications.push(notification);

        if self.notifications.len() > self.max_notifications {
            self.notifications.remove(0);
        }
    }

    pub fn get_unread(&self) -> Vec<&Notification> {
        self.notifications.iter().filter(|n| !n.read).collect()
    }

    pub fn get_all(&self) -> &[Notification] {
        &self.notifications
    }

    pub fn mark_read(&mut self, id: &str) {
        if let Some(n) = self.notifications.iter_mut().find(|n| n.id == id) {
            n.read = true;
        }
    }

    pub fn mark_all_read(&mut self) {
        for n in &mut self.notifications {
            n.read = true;
        }
    }

    pub fn clear(&mut self) {
        self.notifications.clear();
    }
}
