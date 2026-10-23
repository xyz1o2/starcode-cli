pub struct WikiManager {
    pub pages: Vec<WikiPage>,
    pub index_path: String,
}

#[derive(Debug, Clone)]
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
                let path = entry.path();
                // 目录里可能混着别的文件（编辑器临时文件之类），只认 .md
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(mut page) = self.parse_page(&content) {
                        // 文件名就是稳定 id。以前每次 load 都现生成 UUID，
                        // 同一个页面的 id 每次都不一样，`/wiki` 里根本没法引用。
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            page.id = stem.to_string();
                        }
                        self.pages.push(page);
                    }
                }
            }
        }
        // `read_dir` 的顺序取决于文件系统，同一目录两次启动可能不一样；
        // 列表按标题排一下，`/wiki` 的输出才是可复现的。
        self.pages
            .sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
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
        let now = chrono::Utc::now().timestamp();
        // id 换成"标题 slug + 时间戳"，不再用随机 UUID：
        // 它是一个文件名，必须在两次启动之间稳定，否则 `/wiki` 里展示的 id
        // 下次就对不上了。
        let page = WikiPage {
            id: format!("{}-{}", slugify(title), now),
            title: title.to_string(),
            content: content.to_string(),
            tags,
            created_at: now,
            updated_at: now,
        };
        self.pages.push(page);
        // 落盘：不写文件的话 `/wiki` 的每一条创建提示都是假的 —— 下次启动
        // 就从 `load()` 里凭空消失。写失败只影响持久化，页面在本次会话仍可用，
        // 所以这里不让调用方去处理错误（签名保持返回 `&WikiPage`）。
        let saved = self.pages.last().cloned();
        if let Some(page) = saved {
            let _ = self.persist(&page);
        }
        self.pages.last().unwrap()
    }

    /// 一页一个 `.md` 文件，frontmatter 与 [`Self::parse_page`] 对称。
    fn persist(&self, page: &WikiPage) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.index_path)?;
        let tags = page
            .tags
            .iter()
            .map(|t| format!("\"{}\"", t))
            .collect::<Vec<_>>()
            .join(", ");
        let body = format!(
            "---\ntitle: {}\ntags: {}\n---\n{}",
            page.title, tags, page.content
        );
        std::fs::write(
            std::path::Path::new(&self.index_path).join(format!("{}.md", page.id)),
            body,
        )
    }

    pub fn delete_page(&mut self, id: &str) -> bool {
        let len = self.pages.len();
        self.pages.retain(|p| p.id != id);
        if self.pages.len() == len {
            return false;
        }
        // 删了列表项却不删文件，下次 `load()` 会把页面原样带回来。
        let _ =
            std::fs::remove_file(std::path::Path::new(&self.index_path).join(format!("{}.md", id)));
        true
    }

    pub fn update_page(&mut self, id: &str, content: &str) -> bool {
        let updated = if let Some(page) = self.pages.iter_mut().find(|p| p.id == id) {
            page.content = content.to_string();
            page.updated_at = chrono::Utc::now().timestamp();
            Some(page.clone())
        } else {
            None
        };
        match updated {
            // 与 create 一样：内存改了就得落盘，否则重启回到旧内容
            Some(page) => {
                let _ = self.persist(&page);
                true
            }
            None => false,
        }
    }
}

/// 标题 → 文件名安全片段：小写、非字母数字压成 `-`、去首尾分隔符。
fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() || !ch.is_ascii() {
            out.extend(ch.to_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "page".to_string()
    } else {
        trimmed
    }
}
