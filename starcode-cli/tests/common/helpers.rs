//! Reusable test helpers: temp dirs, fixtures, mock data.

use std::path::{Path, PathBuf};

/// Create a temporary project directory with Cargo.toml and a basic structure.
/// Returns the path; the directory is cleaned up when the returned guard drops.
pub struct TempProject {
    pub path: PathBuf,
}

impl TempProject {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("starcode_test_{}", name));
        if path.exists() {
            std::fs::remove_dir_all(&path).ok();
        }
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    /// Write a Cargo.toml into the project root.
    pub fn with_cargo_toml(mut self, content: &str) -> Self {
        std::fs::write(self.path.join("Cargo.toml"), content).unwrap();
        self
    }

    /// Create a file under the project.
    pub fn write_file(&self, relative: &str, content: &str) {
        let p = self.path.join(relative);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
    }

    /// Ensure a `.star/` config directory exists.
    pub fn ensure_star_dir(&self) {
        std::fs::create_dir_all(self.path.join(".star")).unwrap();
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

/// Simple helper: check if a string contains all given substrings (case-insensitive).
pub fn contains_all_ignore_case(haystack: &str, needles: &[&str]) -> bool {
    let lower = haystack.to_lowercase();
    needles.iter().all(|n| lower.contains(&n.to_lowercase()))
}
