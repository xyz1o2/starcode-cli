pub struct WikiManager {
    pub pages: Vec<WikiPage>,
    pub index_path: String,
}

pub struct WikiPage {
    pub id: String,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl WikiManager {
    pub fn new(index_path: &str) -> Self {
        let mut manager = Self {
            pages: Vec::new(),
            index_path: index_path.to_string(),
        };
        manager.load();
        manager
    }

    fn load(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.index_path) {
            for entry in entries.flatten() {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Some(page) = self.parse_page(&content) {
                        self.pages.push(page);
                    }
                }
            }
        }
    }

    fn parse_page(&self, content: &str) -> Option<WikiPage> {
        if content.starts_with("---") {
            let end = content[3..].find("---")?;
            let frontmatter = &content[3..end + 3];
            let body = &content[end + 6..];

            let mut title = String::new();
            let mut tags = Vec::new();

            for line in frontmatter.lines() {
                if let Some(val) = line.strip_prefix("title:") {
                    title = val.trim().trim_matches('"').to_string();
                }
                if let Some(val) = line.strip_prefix("tags:") {
                    tags = val
                        .split(',')
                        .map(|t| t.trim().trim_matches('"').to_string())
                        .collect();
                }
            }

            if title.is_empty() {
                title = "Untitled".to_string();
            }

            Some(WikiPage {
                id: uuid::Uuid::new_v4().to_string(),
                title,
                content: body.to_string(),
                tags,
                created_at: chrono::Utc::now().timestamp(),
                updated_at: chrono::Utc::now().timestamp(),
            })
        } else {
            Some(WikiPage {
                id: uuid::Uuid::new_v4().to_string(),
                title: content.lines().next()?.to_string(),
                content: content.to_string(),
                tags: Vec::new(),
                created_at: chrono::Utc::now().timestamp(),
                updated_at: chrono::Utc::now().timestamp(),
            })
        }
    }

    pub fn search(&self, query: &str) -> Vec<&WikiPage> {
        self.pages
            .iter()
            .filter(|p| {
                p.title.contains(query)
                    || p.content.contains(query)
                    || p.tags.iter().any(|t| t.contains(query))
            })
            .collect()
    }

    pub fn get_page(&self, id: &str) -> Option<&WikiPage> {
        self.pages.iter().find(|p| p.id == id)
    }

    pub fn create_page(&mut self, title: &str, content: &str, tags: Vec<String>) -> &WikiPage {
        let page = WikiPage {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            content: content.to_string(),
            tags,
            created_at: chrono::Utc::now().timestamp(),
            updated_at: chrono::Utc::now().timestamp(),
        };
        self.pages.push(page);
        self.pages.last().unwrap()
    }

    pub fn update_page(&mut self, id: &str, content: &str) -> bool {
        if let Some(page) = self.pages.iter_mut().find(|p| p.id == id) {
            page.content = content.to_string();
            page.updated_at = chrono::Utc::now().timestamp();
            true
        } else {
            false
        }
    }

    pub fn delete_page(&mut self, id: &str) -> bool {
        let len = self.pages.len();
        self.pages.retain(|p| p.id != id);
        self.pages.len() < len
    }
}
