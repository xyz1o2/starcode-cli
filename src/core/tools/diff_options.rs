use crate::core::tools::tools::DiffStat;

pub const DEFAULT_DIFF_CONTEXT: usize = 3;

pub struct DiffOptions {
    pub context: usize,
    pub ignore_whitespace: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            context: DEFAULT_DIFF_CONTEXT,
            ignore_whitespace: true,
        }
    }
}

pub fn get_diff_stat(_file_name: &str, old_str: &str, ai_str: &str, user_str: &str) -> DiffStat {
    let model_stats = calculate_stats(old_str, ai_str);
    let user_stats = calculate_stats(ai_str, user_str);

    DiffStat {
        model_added_lines: model_stats.added_lines,
        model_removed_lines: model_stats.removed_lines,
        model_added_chars: model_stats.added_chars,
        model_removed_chars: model_stats.removed_chars,
        user_added_lines: user_stats.added_lines,
        user_removed_lines: user_stats.removed_lines,
        user_added_chars: user_stats.added_chars,
        user_removed_chars: user_stats.removed_chars,
    }
}

#[derive(Debug, Clone, Default)]
struct DiffStats {
    added_lines: usize,
    removed_lines: usize,
    added_chars: usize,
    removed_chars: usize,
}

fn calculate_stats(old: &str, new: &str) -> DiffStats {
    let mut stats = DiffStats::default();

    let diff = similar::TextDiff::from_lines(old, new);

    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Delete => {
                stats.removed_lines += 1;
                stats.removed_chars += change.value().len();
            }
            similar::ChangeTag::Insert => {
                stats.added_lines += 1;
                stats.added_chars += change.value().len();
            }
            similar::ChangeTag::Equal => {}
        }
    }

    stats
}

pub fn create_patch(
    file_name: &str,
    old_str: &str,
    new_str: &str,
    old_label: &str,
    new_label: &str,
) -> String {
    let diff = similar::TextDiff::from_lines(old_str, new_str);
    let old_header = format!("{} {}", old_label, file_name);
    let new_header = format!("{} {}", new_label, file_name);
    diff.unified_diff()
        .header(&old_header, &new_header)
        .to_string()
}
