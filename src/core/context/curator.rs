use std::fs;
use std::path::{Path, PathBuf};

/// Curator 组件
/// 职责：负责维护上下文的“整洁”和“相关性”。
#[derive(Clone)]
pub struct Curator {
    context_dir: PathBuf,
}

impl Curator {
    pub fn new(project_root: &Path) -> Self {
        let context_dir = project_root.join(".star").join("context");
        Self { context_dir }
    }

    /// 清理临时上下文文件 (示例：删除超过7天的临时文件)
    /// 目前仅作为框架占位，实际逻辑需要根据文件命名规范或元数据来判断
    pub fn clean_temporary_files(&self) -> Result<usize, std::io::Error> {
        let mut count = 0;
        if self.context_dir.exists() {
            for entry in fs::read_dir(&self.context_dir)? {
                let entry = entry?;
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("temp_") {
                        // 简单逻辑：如果是 temp_ 开头的文件，直接删除
                        fs::remove_file(path)?;
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    /// 归档旧规则 (当 learned_rules.md 过大时)
    pub fn archive_rules(&self) -> Result<bool, std::io::Error> {
        let rules_path = self.context_dir.join("learned_rules.md");
        if rules_path.exists() {
            let metadata = fs::metadata(&rules_path)?;
            if metadata.len() > 1024 * 100 {
                // 100KB
                let archive_path = self.context_dir.join(format!(
                    "learned_rules_archive_{}.md",
                    chrono::Utc::now().timestamp()
                ));
                fs::rename(&rules_path, archive_path)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}
