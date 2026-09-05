//! 系统提示词里的"当前目录结构"片段。
//!
//! 遍历走 [`crate::utils::file_walk`] 的统一口径：`.gitignore` / `.starignore`
//! 生效、dotfile 可见、6 个 VCS 目录永不进入。以前是 `WalkDir` +
//! `filter_entry(!is_hidden)`，既不认 `.gitignore`（50 条名额很容易被
//! `target/` 的一层子目录吃掉），又把 `.github/` 这类目录整棵剪掉。

use std::path::Path;

use crate::utils::file_walk::{walk_builder, WalkOptions};

/// 深度上限 —— 只要一眼能看出项目布局就够，再深就是浪费上下文。
const MAX_DEPTH: usize = 2;
/// 条目上限。
const MAX_ENTRIES: usize = 50;

pub fn get_directory_context_string(cwd: &Path) -> String {
    let folder_structure = get_folder_structure(cwd);

    let working_dir_preamble = format!("I'm currently working in the directory: {}", cwd.display());

    format!(
        "{}\nHere is the folder structure of the current working directories:\n\n{}",
        working_dir_preamble, folder_structure
    )
}

fn get_folder_structure(path: &Path) -> String {
    let mut structure = String::new();
    let mut entry_count = 0;

    let opts = WalkOptions::new().max_depth(MAX_DEPTH);
    let walker = walk_builder(path, &opts)
        .sort_by_file_name(Ord::cmp)
        .build();

    for entry in walker.flatten() {
        if entry_count >= MAX_ENTRIES {
            structure.push_str("    ...\n");
            break;
        }

        let depth = entry.depth();
        if depth == 0 {
            continue; // Skip root directory itself in the tree view
        }

        let indent = "  ".repeat(depth - 1);
        let file_name = entry.file_name().to_string_lossy();

        // Add file type indicator
        let indicator = if entry.file_type().is_some_and(|t| t.is_dir()) {
            "/"
        } else {
            ""
        };

        structure.push_str(&format!("{}- {}{}\n", indent, file_name, indicator));
        entry_count += 1;
    }

    structure
}
