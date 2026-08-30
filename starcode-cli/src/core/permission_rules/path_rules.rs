use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathPermission {
    Read,
    Write,
    Execute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRule {
    pub pattern: String,
    pub permissions: Vec<PathPermission>,
    pub description: Option<String>,
}

pub struct PathRuleMatcher {
    rules: Vec<PathRule>,
    base_dir: PathBuf,
}

impl PathRuleMatcher {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            rules: Vec::new(),
            base_dir,
        }
    }

    pub fn add_rule(&mut self, rule: PathRule) {
        self.rules.push(rule);
    }

    pub fn set_rules(&mut self, rules: Vec<PathRule>) {
        self.rules = rules;
    }

    pub fn check_permission(&self, path: &Path, permission: &PathPermission) -> bool {
        let canonical = self.resolve_path(path);

        for rule in &self.rules {
            if self.matches_pattern(&rule.pattern, &canonical) {
                return rule.permissions.contains(permission);
            }
        }
        true
    }

    pub fn get_matching_rules(&self, path: &Path) -> Vec<&PathRule> {
        let canonical = self.resolve_path(path);
        self.rules
            .iter()
            .filter(|r| self.matches_pattern(&r.pattern, &canonical))
            .collect()
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir.join(path)
        }
    }

    fn matches_pattern(&self, pattern: &str, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        self.glob_match(pattern, &path_str)
    }

    pub fn glob_match(&self, pattern: &str, text: &str) -> bool {
        if pattern.contains("**") {
            return self.matches_double_star(pattern, text);
        }
        self.matches_simple_glob(pattern, text)
    }

    fn matches_double_star(&self, pattern: &str, text: &str) -> bool {
        let parts: Vec<&str> = pattern.split("**").collect();
        if parts.len() != 2 {
            return false;
        }

        let prefix = parts[0];
        let suffix = parts[1];

        if !prefix.is_empty() && !text.starts_with(prefix.trim_end_matches('/')) {
            return false;
        }

        let remaining = if prefix.is_empty() {
            text
        } else {
            &text[prefix.trim_end_matches('/').len()..]
        };

        if suffix.is_empty() {
            return true;
        }

        let suffix_clean = suffix.trim_start_matches('/');
        if suffix_clean.is_empty() {
            return true;
        }

        for i in 0..=remaining.len() {
            let candidate = &remaining[i..];
            if self.matches_simple_glob(suffix_clean, candidate) {
                return true;
            }
        }

        false
    }

    fn matches_simple_glob(&self, pattern: &str, text: &str) -> bool {
        let pattern = pattern.trim_start_matches('/');
        let text = text.trim_start_matches('/');

        let pattern_chars: Vec<char> = pattern.chars().collect();
        let text_chars: Vec<char> = text.chars().collect();

        self.glob_match_chars(&pattern_chars, &text_chars)
    }

    fn glob_match_chars(&self, pattern: &[char], text: &[char]) -> bool {
        let mut pi = 0;
        let mut ti = 0;
        let mut star_pi = 0;
        let mut star_ti = 0;
        let mut has_star = false;

        while ti < text.len() {
            if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
                pi += 1;
                ti += 1;
            } else if pi < pattern.len() && pattern[pi] == '*' {
                star_pi = pi + 1;
                star_ti = ti;
                has_star = true;
                pi += 1;
            } else if pi < pattern.len() && pattern[pi] == '[' {
                if let Some(end) = pattern[pi..].iter().position(|&c| c == ']') {
                    let set_start = pi + 1;
                    let set_end = pi + end;
                    let negate = set_start < set_end && pattern[set_start] == '^';
                    let actual_start = if negate { set_start + 1 } else { set_start };

                    let matched = pattern[actual_start..set_end].contains(&text[ti]);
                    if (matched && !negate) || (!matched && negate) {
                        pi = set_end + 1;
                        ti += 1;
                    } else if has_star {
                        pi = star_pi;
                        star_ti += 1;
                        ti = star_ti;
                    } else {
                        return false;
                    }
                } else {
                    if pi < pattern.len() && pattern[pi] == text[ti] {
                        pi += 1;
                        ti += 1;
                    } else if has_star {
                        pi = star_pi;
                        star_ti += 1;
                        ti = star_ti;
                    } else {
                        return false;
                    }
                }
            } else if pi < pattern.len() && pattern[pi] == '{' {
                if let Some(end) = pattern[pi..].iter().position(|&c| c == '}') {
                    let options: Vec<&[char]> = pattern[pi + 1..pi + end]
                        .split(|&c| c == ',')
                        .collect();

                    let mut matched = false;
                    for option in options {
                        let opt_str: String = option.iter().collect();
                        let remaining_text: String = text[ti..].iter().collect();
                        if remaining_text.starts_with(&opt_str) {
                            ti += option.len();
                            pi = end + pi + 1;
                            matched = true;
                            break;
                        }
                    }

                    if !matched && has_star {
                        pi = star_pi;
                        star_ti += 1;
                        ti = star_ti;
                    } else if !matched {
                        return false;
                    }
                } else {
                    if pi < pattern.len() && pattern[pi] == text[ti] {
                        pi += 1;
                        ti += 1;
                    } else if has_star {
                        pi = star_pi;
                        star_ti += 1;
                        ti = star_ti;
                    } else {
                        return false;
                    }
                }
            } else if has_star {
                pi = star_pi;
                star_ti += 1;
                ti = star_ti;
            } else {
                return false;
            }
        }

        while pi < pattern.len() && pattern[pi] == '*' {
            pi += 1;
        }

        pi == pattern.len()
    }
}

pub fn is_path_within(base: &Path, target: &Path) -> bool {
    let Ok(base_canon) = base.canonicalize() else {
        return false;
    };
    let Ok(target_canon) = target.canonicalize() else {
        return false;
    };
    target_canon.starts_with(&base_canon)
}
 