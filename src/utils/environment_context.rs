use std::path::Path;
use walkdir::WalkDir;

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
    let max_depth = 2; // Limit depth to avoid excessive context
    let max_files = 50; // Limit total files to avoid context window exhaustion
    let mut file_count = 0;

    // Use WalkDir for efficient recursive traversal
    let walker = WalkDir::new(path)
        .max_depth(max_depth)
        .sort_by_file_name()
        .into_iter();

    for entry in walker.filter_entry(|e| !is_hidden(e)) {
        if file_count >= max_files {
            structure.push_str("    ...\n");
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let depth = entry.depth();
        if depth == 0 {
            continue; // Skip root directory itself in the tree view
        }

        let indent = "  ".repeat(depth - 1);
        let file_name = entry.file_name().to_string_lossy();

        // Add file type indicator
        let indicator = if entry.file_type().is_dir() { "/" } else { "" };

        structure.push_str(&format!("{}- {}{}\n", indent, file_name, indicator));
        file_count += 1;
    }

    structure
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}
