use std::process::Command;

pub fn get_git_info() -> Option<(String, String)> {
    // Check if we are in a git repo
    let status_output = Command::new("git")
        .args(&["status", "--porcelain", "-b"])
        .output()
        .ok()?;

    if !status_output.status.success() {
        return None;
    }

    let output = String::from_utf8_lossy(&status_output.stdout);
    let mut lines = output.lines();

    // First line contains branch info: ## main...origin/main [ahead 1]
    let first_line = lines.next()?;

    let branch_info = if let Some(info) = first_line.strip_prefix("## ") {
        info
    } else {
        return None;
    };

    // Parse branch name and remote status
    // Examples:
    // main...origin/main
    // main...origin/main [ahead 1]
    // main...origin/main [behind 1]
    // No commits yet on main
    // Initial commit on main

    let (branch_name, remote_info) = if let Some(dot_pos) = branch_info.find("...") {
        let branch = &branch_info[..dot_pos];
        let rest = &branch_info[dot_pos + 3..];
        (branch, Some(rest))
    } else {
        (branch_info, None)
    };

    let mut status_flags = Vec::new();

    // Check for dirty state (remaining lines)
    let mut dirty = false;
    if lines.next().is_some() {
        dirty = true;
    }

    if dirty {
        status_flags.push("*"); // Dirty symbol
    }

    // Parse remote info for ahead/behind
    if let Some(remote) = remote_info {
        if let Some(start) = remote.find('[') {
            if let Some(end) = remote.find(']') {
                let status_part = &remote[start + 1..end];
                // e.g. ahead 1, behind 2, ahead 1, behind 2
                if status_part.contains("ahead") {
                    status_flags.push("↑");
                }
                if status_part.contains("behind") {
                    status_flags.push("↓");
                }
            }
        }
    }

    // Construct final branch string
    // e.g. "main" or "main *" or "main ↑"
    let display_branch = branch_name.trim().to_string();

    // Status string: "Clean", "Dirty", "↑", "↓", "↑↓"
    // Better: return (branch_name, status_summary)
    // status_summary: "Clean", "Dirty", "↑ 1", etc.

    // Let's refine status_str
    let mut detailed_status = String::new();
    if dirty {
        detailed_status.push_str("Dirty");
    } else {
        detailed_status.push_str("Clean");
    }

    if let Some(remote) = remote_info {
        if let Some(start) = remote.find('[') {
            if let Some(end) = remote.find(']') {
                let status_part = &remote[start + 1..end];
                detailed_status.push_str(" ");
                detailed_status.push_str(status_part);
            }
        }
    }

    Some((display_branch, detailed_status))
}
