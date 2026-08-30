pub struct EnvInfo<'a> {
    pub today: &'a str,
    pub platform: &'a str,
    pub cwd: &'a str,
    pub shell: &'a str,
    pub is_git_repo: bool,
    pub git_branch: Option<&'a str>,
    pub git_status: Option<&'a str>,
    pub recent_commits: Option<&'a str>,
}

pub fn render(env: EnvInfo<'_>) -> String {
    let mut parts = vec![
        "# Environment".to_string(),
        "You have been invoked in the following environment:".to_string(),
        format!("  Working directory: {}", env.cwd),
        format!("  Platform: {}", env.platform),
        format!("  Shell: {}", env.shell),
    ];

    if env.is_git_repo {
        parts.push("  Is a git repository: yes".to_string());
        if let Some(branch) = env.git_branch {
            parts.push(format!("  Current branch: {}", branch));
        }
        if let Some(status) = env.git_status {
            parts.push(format!("  Git status: {}", status));
        }
        if let Some(commits) = env.recent_commits {
            if !commits.is_empty() {
                parts.push(format!("  Recent commits:\n{}", commits));
            }
        }
    } else {
        parts.push("  Is a git repository: no".to_string());
    }

    parts.join("\n")
}

/// Detect current shell from environment
pub fn detect_shell() -> String {
    if cfg!(windows) {
        if std::env::var("PSModulePath").is_ok() {
            "powershell".to_string()
        } else {
            "cmd".to_string()
        }
    } else {
        std::env::var("SHELL")
            .ok()
            .and_then(|s| std::path::Path::new(&s).file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_else(|| "sh".to_string())
    }
}

/// Detect if running in Docker
pub fn is_docker() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .map(|s| s.contains("docker"))
            .unwrap_or(false)
}

/// Detect if running in WSL
pub fn is_wsl() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/version")
            .map(|s| s.to_lowercase().contains("microsoft"))
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Detect if running in CI
pub fn is_ci() -> bool {
    std::env::var("CI").is_ok()
        || std::env::var("GITHUB_ACTIONS").is_ok()
        || std::env::var("GITLAB_CI").is_ok()
        || std::env::var("JENKINS_URL").is_ok()
        || std::env::var("CIRCLECI").is_ok()
        || std::env::var("TRAVIS").is_ok()
}

/// Render static environment info (does NOT change between turns).
/// This part is cached with cache_control for prompt caching.
pub fn render_static_env_info(
    today: &str,
    platform: &str,
    cwd: &str,
    shell: &str,
) -> String {
    let mut parts = vec![
        "# Environment".to_string(),
        "You have been invoked in the following environment:".to_string(),
        format!("  Working directory: {}", cwd),
        format!("  Platform: {}", platform),
        format!("  Shell: {}", shell),
        format!("  Date: {}", today),
    ];

    parts.join("\n")
}

/// Render dynamic git info (changes between turns).
/// This part is NOT cached to avoid invalidating the prompt cache.
pub fn render_dynamic_git_info(
    git_branch: Option<&str>,
    git_status: Option<&str>,
    recent_commits: Option<&str>,
) -> String {
    let mut parts = vec!["# Git Status".to_string()];

    if let Some(branch) = git_branch {
        parts.push(format!("  Current branch: {}", branch));
    }
    if let Some(status) = git_status {
        parts.push(format!("  Working tree: {}", status));
    }
    if let Some(commits) = recent_commits {
        if !commits.is_empty() {
            parts.push(format!("  Recent commits:\n{}", commits));
        }
    }

    parts.join("\n")
}
