//! `/team-onboarding` — turn a project's recent local history into a concrete
//! onboarding brief a teammate can paste as their first message.
//!
//! Claude Code's command asks a model to read the last 30 days of sessions. We
//! keep the same shape but derive the factual parts locally: project stack and
//! commands come from marker files, while command/MCP/tool/file frequency comes
//! from the shared [`crate::commands::activity`] scanner. The output is a normal
//! markdown assistant message (or a file with `--write`), so it works headless
//! and remains deterministic enough to test.

use crate::commands::activity::{self, ActivityScan, ToolCallRecord};
use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;
use chrono::{Duration, Utc};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

const DEFAULT_DAYS: i64 = 30;
const GUIDE_FILE: &str = ".star/team-onboarding.md";
const USAGE: &str = concat!(
    "Usage: /team-onboarding [--days N] [--write]\n",
    "\n",
    "Builds a markdown onboarding guide from this project's recent history and\n",
    "the repository's build/test markers.\n",
    "\n",
    "  --days N   Look back N days (default 30; 0 = all recorded history)\n",
    "  --write    Also save the guide to `.star/team-onboarding.md`\n",
    "  -h, --help Show this help\n",
);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    days: i64,
    write: bool,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        days: DEFAULT_DAYS,
        write: false,
    };
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].trim();
        match arg {
            "--write" => options.write = true,
            "--days" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--days requires a value".to_string())?;
                options.days = parse_days(value)?;
                index += 1;
            }
            _ if arg.starts_with("--days=") => {
                options.days = parse_days(&arg["--days=".len()..])?;
            }
            _ => return Err(format!("Unknown argument `{}`.\n\n{}", arg, USAGE)),
        }
        index += 1;
    }
    Ok(options)
}

fn parse_days(raw: &str) -> Result<i64, String> {
    raw.trim()
        .parse::<i64>()
        .map_err(|_| format!("--days expects a non-negative integer, got `{}`", raw))
        .and_then(|days| {
            if days < 0 {
                Err("--days expects a non-negative integer".to_string())
            } else {
                Ok(days)
            }
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectProfile {
    kind: &'static str,
    install: Option<String>,
    build: Option<String>,
    test: Option<String>,
    lint: Option<String>,
    run: Option<String>,
}

impl ProjectProfile {
    fn unknown() -> Self {
        Self {
            kind: "Unknown stack",
            install: None,
            build: None,
            test: None,
            lint: None,
            run: None,
        }
    }
}

fn detect_project_profile(root: &Path) -> ProjectProfile {
    if root.join("Cargo.toml").is_file() {
        return ProjectProfile {
            kind: "Rust (Cargo)",
            install: Some("cargo fetch".to_string()),
            build: Some("cargo build".to_string()),
            test: Some("cargo test".to_string()),
            lint: Some("cargo clippy --all-targets -- -D warnings".to_string()),
            run: Some("cargo run".to_string()),
        };
    }

    if root.join("package.json").is_file() {
        return detect_node_profile(root);
    }

    if root.join("go.mod").is_file() {
        return ProjectProfile {
            kind: "Go",
            install: Some("go mod download".to_string()),
            build: Some("go build ./...".to_string()),
            test: Some("go test ./...".to_string()),
            lint: Some("go vet ./...".to_string()),
            run: None,
        };
    }

    if root.join("pyproject.toml").is_file()
        || root.join("requirements.txt").is_file()
        || root.join("setup.py").is_file()
    {
        let uses_uv = root.join("uv.lock").is_file();
        let install = if uses_uv {
            "uv sync".to_string()
        } else if root.join("pyproject.toml").is_file() {
            "python -m pip install -e .".to_string()
        } else {
            "python -m pip install -r requirements.txt".to_string()
        };
        return ProjectProfile {
            kind: "Python",
            install: Some(install),
            build: None,
            test: Some("pytest".to_string()),
            lint: Some("ruff check .".to_string()),
            run: None,
        };
    }

    if root.join("pom.xml").is_file() {
        return ProjectProfile {
            kind: "Java (Maven)",
            install: None,
            build: Some("mvn -q package".to_string()),
            test: Some("mvn test".to_string()),
            lint: None,
            run: None,
        };
    }

    let gradle = root.join("build.gradle").is_file() || root.join("build.gradle.kts").is_file();
    if gradle {
        return ProjectProfile {
            kind: "Java/Kotlin (Gradle)",
            install: None,
            build: Some("./gradlew build".to_string()),
            test: Some("./gradlew test".to_string()),
            lint: None,
            run: None,
        };
    }

    if root.join("Makefile").is_file() {
        return ProjectProfile {
            kind: "Make",
            install: None,
            build: Some("make".to_string()),
            test: Some("make test".to_string()),
            lint: Some("make lint".to_string()),
            run: Some("make run".to_string()),
        };
    }

    if root.join("CMakeLists.txt").is_file() {
        return ProjectProfile {
            kind: "C/C++ (CMake)",
            install: None,
            build: Some("cmake -S . -B build && cmake --build build".to_string()),
            test: Some("ctest --test-dir build".to_string()),
            lint: None,
            run: None,
        };
    }

    ProjectProfile::unknown()
}

fn detect_node_profile(root: &Path) -> ProjectProfile {
    let manager = if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else if root.join("bun.lockb").is_file() || root.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    };
    let install = match manager {
        "pnpm" => "pnpm install",
        "yarn" => "yarn",
        "bun" => "bun install",
        _ => "npm install",
    }
    .to_string();

    let scripts = std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| value.get("scripts").cloned())
        .unwrap_or(Value::Null);
    let script = |name: &str| {
        scripts
            .get(name)
            .and_then(Value::as_str)
            .map(|_| match manager {
                "pnpm" => format!("pnpm {}", name),
                "yarn" => format!("yarn {}", name),
                "bun" => format!("bun run {}", name),
                _ => format!("npm run {}", name),
            })
    };

    ProjectProfile {
        kind: "Node.js",
        install: Some(install),
        build: script("build"),
        test: script("test"),
        lint: script("lint"),
        run: script("dev").or_else(|| script("start")),
    }
}

#[derive(Debug, Clone, Default)]
struct TeamHistory {
    sources: usize,
    prompts: usize,
    tool_calls: usize,
    top_tools: Vec<(String, usize)>,
    top_commands: Vec<(String, usize)>,
    mcp_servers: Vec<(String, usize)>,
    skills: Vec<(String, usize)>,
    top_files: Vec<(String, usize)>,
    malformed: usize,
}

fn aggregate_history(scan: &ActivityScan, root: &Path) -> TeamHistory {
    let tool_counts = scan.tool_counts();
    let top_tools = top_entries(
        tool_counts
            .into_iter()
            .filter(|(tool, _)| activity::split_mcp_tool(tool).is_none()),
        8,
    );
    let top_commands = top_entries(scan.slash_command_counts(), 8);
    let mcp_servers = top_entries(scan.mcp_server_counts(), 8);
    let skills = top_entries(scan.skill_invocations(), 8);
    let top_files = top_entries(count_file_paths(&scan.tool_calls, root), 8);

    TeamHistory {
        sources: scan.sources,
        prompts: scan.user_prompts.len(),
        tool_calls: scan.tool_calls.len(),
        top_tools,
        top_commands,
        mcp_servers,
        skills,
        top_files,
        malformed: scan.malformed,
    }
}

fn top_entries(
    entries: impl IntoIterator<Item = (String, usize)>,
    limit: usize,
) -> Vec<(String, usize)> {
    let mut entries: Vec<(String, usize)> = entries
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .collect();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    entries.truncate(limit);
    entries
}

fn count_file_paths(calls: &[ToolCallRecord], root: &Path) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for call in calls {
        for key in [
            "path",
            "file_path",
            "target_file",
            "notebook_path",
            "relative_path",
        ] {
            let Some(raw) = call.arguments.get(key).and_then(Value::as_str) else {
                continue;
            };
            if let Some(path) = normalize_project_path(raw, root) {
                *counts.entry(path).or_insert(0) += 1;
            }
        }
    }
    counts
}

fn normalize_project_path(raw: &str, root: &Path) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('*') || trimmed.contains('?') {
        return None;
    }
    let path = Path::new(trimmed);
    let relative = if path.is_absolute() {
        path.strip_prefix(root).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    let rendered = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if rendered.is_empty() || rendered == "." {
        None
    } else {
        Some(rendered)
    }
}

fn existing_paths(root: &Path, candidates: &[&str]) -> Vec<String> {
    candidates
        .iter()
        .filter(|candidate| root.join(candidate).exists())
        .map(|candidate| candidate.to_string())
        .collect()
}

fn render_guide(root: &Path, profile: &ProjectProfile, history: &TeamHistory, days: i64) -> String {
    let window = if days == 0 {
        "all recorded history".to_string()
    } else {
        format!("the last {} day(s)", days)
    };
    let mut out = String::new();
    out.push_str("# Team onboarding guide\n\n");
    out.push_str(&format!(
        "Generated locally from {} source(s), {} prompt(s), and {} tool call(s) over {}.\n\n",
        history.sources, history.prompts, history.tool_calls, window
    ));

    out.push_str("## Project at a glance\n\n");
    out.push_str(&format!("- Stack: **{}**\n", profile.kind));
    for (label, command) in [
        ("Install", profile.install.as_deref()),
        ("Build", profile.build.as_deref()),
        ("Test", profile.test.as_deref()),
        ("Lint", profile.lint.as_deref()),
        ("Run", profile.run.as_deref()),
    ] {
        if let Some(command) = command {
            out.push_str(&format!("- {}: `{}`\n", label, command));
        }
    }
    if profile.install.is_none()
        && profile.build.is_none()
        && profile.test.is_none()
        && profile.lint.is_none()
        && profile.run.is_none()
    {
        out.push_str(
            "- No build/test marker was detected; ask the team for the canonical command.\n",
        );
    }

    let reading = existing_paths(
        root,
        &[
            "STAR.md",
            "CLAUDE.md",
            "AGENTS.md",
            ".star/settings.json",
            ".mcp.json",
        ],
    );
    let has_skills = root.join(".star/skills").is_dir();
    let has_agents = root.join(".star/agents").is_dir();
    if !reading.is_empty() || has_skills || has_agents {
        out.push_str("\n## Read first\n\n");
        for path in reading {
            out.push_str(&format!("- `{}`\n", path));
        }
        if has_skills {
            out.push_str("- `.star/skills/` — project skills\n");
        }
        if has_agents {
            out.push_str("- `.star/agents/` — project sub-agents\n");
        }
    }

    out.push_str("\n## How this team works\n\n");
    if history.sources == 0 || (history.tool_calls == 0 && history.prompts == 0) {
        out.push_str(
            "No local history was found yet. The setup above still applies; run a few turns in \
             this repository and regenerate this guide for team-specific habits.\n",
        );
    } else {
        render_frequency(
            &mut out,
            "Most-used slash commands",
            &history.top_commands,
            "/",
        );
        render_frequency(&mut out, "MCP servers", &history.mcp_servers, "");
        render_frequency(&mut out, "Tools", &history.top_tools, "");
        render_frequency(&mut out, "Skills", &history.skills, "");
        render_frequency(&mut out, "Frequently touched files", &history.top_files, "");
        if history.malformed > 0 {
            out.push_str(&format!(
                "\n{} malformed source line/file(s) were ignored.\n",
                history.malformed
            ));
        }
    }

    out.push_str("\n## First message to paste\n\n```text\n");
    out.push_str("I'm onboarding to this repository. Please help me get productive without changing code yet.\n\n");
    out.push_str(&format!("Project stack: {}.\n", profile.kind));
    if let Some(command) = profile.install.as_deref() {
        out.push_str(&format!("Setup command from the repo: `{}`.\n", command));
    }
    if let Some(command) = profile.test.as_deref() {
        out.push_str(&format!(
            "Verification command from the repo: `{}`.\n",
            command
        ));
    }
    if !history.top_commands.is_empty() {
        out.push_str(&format!(
            "The team commonly uses: {}.\n",
            history
                .top_commands
                .iter()
                .map(|(command, count)| format!("/{} ({}x)", command, count))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !history.mcp_servers.is_empty() {
        out.push_str(&format!(
            "MCP servers seen in local history: {}.\n",
            history
                .mcp_servers
                .iter()
                .map(|(server, _)| server.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str(
        "Start by reading STAR.md/CLAUDE.md and the project settings, explain the architecture, \
         then propose a small first task and the exact commands you would run to verify it. \
         Ask before making changes.\n",
    );
    out.push_str("```\n");
    out
}

fn render_frequency(out: &mut String, title: &str, entries: &[(String, usize)], prefix: &str) {
    if entries.is_empty() {
        return;
    }
    out.push_str(&format!("\n### {}\n\n", title));
    for (name, count) in entries {
        out.push_str(&format!("- `{}{}` — {} call(s)\n", prefix, name, count));
    }
}

pub async fn run(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args
        .iter()
        .any(|arg| arg == "-h" || arg == "--help" || arg == "help")
    {
        reply(ctx.state, USAGE);
        return Ok(());
    }

    let options = parse_options(&args)?;
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cutoff = if options.days == 0 {
        None
    } else {
        Utc::now().checked_sub_signed(Duration::days(options.days))
    };
    let scan = activity::scan_project_activity(&root, cutoff);
    let profile = detect_project_profile(&root);
    let history = aggregate_history(&scan, &root);
    let mut guide = render_guide(&root, &profile, &history, options.days);

    if options.write {
        let path = write_guide(&root, &guide)?;
        guide.push_str(&format!("\n---\nSaved to `{}`.\n", path.display()));
    }
    reply(ctx.state, guide);
    Ok(())
}

fn write_guide(root: &Path, guide: &str) -> Result<PathBuf, String> {
    let path = root.join(GUIDE_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create `{}`: {}", parent.display(), error))?;
    }
    std::fs::write(&path, guide)
        .map_err(|error| format!("Could not write `{}`: {}", path.display(), error))?;
    Ok(path)
}

fn reply(state: &mut crate::ui::state::store::ChatState, msg: impl Into<String>) {
    state
        .chat_history
        .push(ChatEntry::assistant(msg).with_streaming(false));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::activity::ToolCallRecord;
    use serde_json::json;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "starcode-team-onboarding-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn call(tool: &str, arguments: Value) -> ToolCallRecord {
        ToolCallRecord {
            tool: tool.to_string(),
            arguments,
            timestamp: None,
        }
    }

    #[test]
    fn parses_days_and_write_flags() {
        assert_eq!(
            parse_options(&["--days=7".to_string(), "--write".to_string()]).unwrap(),
            Options {
                days: 7,
                write: true
            }
        );
        assert_eq!(parse_options(&[]).unwrap().days, DEFAULT_DAYS);
        assert!(parse_options(&["--days".to_string()]).is_err());
        assert!(parse_options(&["--days=-1".to_string()]).is_err());
        assert!(parse_options(&["--wat".to_string()]).is_err());
    }

    #[test]
    fn detects_cargo_node_and_python_projects() {
        let cargo = temp_dir("cargo");
        std::fs::write(cargo.join("Cargo.toml"), "[package]\nname=\"demo\"\n").unwrap();
        let profile = detect_project_profile(&cargo);
        assert_eq!(profile.kind, "Rust (Cargo)");
        assert_eq!(profile.test.as_deref(), Some("cargo test"));
        let _ = std::fs::remove_dir_all(&cargo);

        let node = temp_dir("node");
        std::fs::write(
            node.join("package.json"),
            r#"{"scripts":{"build":"tsc","test":"vitest"}}"#,
        )
        .unwrap();
        std::fs::write(node.join("pnpm-lock.yaml"), "").unwrap();
        let profile = detect_project_profile(&node);
        assert_eq!(profile.kind, "Node.js");
        assert_eq!(profile.install.as_deref(), Some("pnpm install"));
        assert_eq!(profile.build.as_deref(), Some("pnpm build"));
        assert_eq!(profile.test.as_deref(), Some("pnpm test"));
        let _ = std::fs::remove_dir_all(&node);

        let python = temp_dir("python");
        std::fs::write(python.join("pyproject.toml"), "[project]\nname=\"demo\"\n").unwrap();
        std::fs::write(python.join("uv.lock"), "").unwrap();
        let profile = detect_project_profile(&python);
        assert_eq!(profile.kind, "Python");
        assert_eq!(profile.install.as_deref(), Some("uv sync"));
        assert_eq!(profile.test.as_deref(), Some("pytest"));
        let _ = std::fs::remove_dir_all(&python);
    }

    #[test]
    fn unknown_project_has_no_invented_commands() {
        let dir = temp_dir("unknown");
        let profile = detect_project_profile(&dir);
        assert_eq!(profile.kind, "Unknown stack");
        assert!(profile.build.is_none());
        assert!(profile.test.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn aggregates_commands_mcp_tools_skills_and_safe_paths() {
        let root = Path::new("/repo");
        let scan = ActivityScan {
            sources: 2,
            tool_calls: vec![
                call("Bash", json!({"command":"git status"})),
                call("Read", json!({"path":"src/main.rs"})),
                call("Read", json!({"path":"src/main.rs"})),
                call("Read", json!({"path":"/etc/passwd"})),
                call("Read", json!({"path":"../outside.txt"})),
                call("mcp__github__list_issues", json!({})),
                call("mcp__github__list_issues", json!({})),
                call("mcp__exa__search", json!({})),
                call("skill", json!({"skill":"review"})),
            ],
            user_prompts: vec![
                activity::UserPromptRecord {
                    content: "/compact keep tests".to_string(),
                    timestamp: None,
                },
                activity::UserPromptRecord {
                    content: "/compact again".to_string(),
                    timestamp: None,
                },
                activity::UserPromptRecord {
                    content: "/review".to_string(),
                    timestamp: None,
                },
            ],
            malformed: 1,
        };
        let history = aggregate_history(&scan, root);
        assert_eq!(history.sources, 2);
        assert_eq!(history.prompts, 3);
        assert_eq!(history.tool_calls, 9);
        assert_eq!(
            history.top_commands,
            vec![("compact".to_string(), 2), ("review".to_string(), 1)]
        );
        assert_eq!(
            history.mcp_servers,
            vec![("github".to_string(), 2), ("exa".to_string(), 1)]
        );
        assert_eq!(history.skills, vec![("review".to_string(), 1)]);
        assert_eq!(history.top_files, vec![("src/main.rs".to_string(), 2)]);
        assert!(history
            .top_tools
            .iter()
            .all(|(tool, _)| !tool.starts_with("mcp__")));
    }

    #[test]
    fn guide_has_setup_history_and_a_pasteable_first_message() {
        let root = Path::new("/repo");
        let profile = ProjectProfile {
            kind: "Rust (Cargo)",
            install: Some("cargo fetch".to_string()),
            build: Some("cargo build".to_string()),
            test: Some("cargo test".to_string()),
            lint: None,
            run: None,
        };
        let history = TeamHistory {
            sources: 1,
            prompts: 2,
            tool_calls: 4,
            top_commands: vec![("compact".to_string(), 2)],
            mcp_servers: vec![("github".to_string(), 1)],
            top_files: vec![("src/main.rs".to_string(), 2)],
            ..TeamHistory::default()
        };
        let guide = render_guide(root, &profile, &history, 30);
        assert!(guide.contains("# Team onboarding guide"));
        assert!(guide.contains("Rust (Cargo)"));
        assert!(guide.contains("cargo test"));
        assert!(guide.contains("Most-used slash commands"));
        assert!(guide.contains("github"));
        assert!(guide.contains("First message to paste"));
        assert!(guide.contains("src/main.rs"));
        assert!(guide.contains("```text"));
    }

    #[test]
    fn guide_lists_existing_instruction_files_and_skill_directories() {
        let root = temp_dir("read-first");
        std::fs::write(root.join("STAR.md"), "# instructions\n").unwrap();
        std::fs::create_dir_all(root.join(".star/skills/review")).unwrap();
        let guide = render_guide(
            &root,
            &ProjectProfile::unknown(),
            &TeamHistory::default(),
            30,
        );
        assert!(guide.contains("`STAR.md`"));
        assert!(guide.contains("`.star/skills/`"));
        assert!(guide.contains("No local history was found yet"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn absolute_paths_inside_the_project_are_rendered_relative() {
        let root = temp_dir("absolute");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/main.rs");
        assert_eq!(
            normalize_project_path(&file.to_string_lossy(), &root),
            Some("src/main.rs".to_string())
        );
        assert_eq!(normalize_project_path("src/*.rs", &root), None);
        assert_eq!(normalize_project_path("../escape.rs", &root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn guide_does_not_leak_command_arguments_or_absolute_paths() {
        let root = Path::new("/repo");
        let profile = ProjectProfile::unknown();
        let scan = ActivityScan {
            sources: 1,
            tool_calls: vec![
                call("Bash", json!({"command":"cat TOPSECRET"})),
                call("Read", json!({"path":"/etc/passwd"})),
            ],
            ..ActivityScan::default()
        };
        let history = aggregate_history(&scan, root);
        let guide = render_guide(root, &profile, &history, 30);
        assert!(!guide.contains("TOPSECRET"));
        assert!(!guide.contains("/etc/passwd"));
        assert!(!guide.contains("Bash("));
    }

    #[test]
    fn write_guide_creates_the_markdown_file() {
        let root = temp_dir("write");
        let path = write_guide(&root, "# Guide\n").unwrap();
        assert_eq!(path, root.join(GUIDE_FILE));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Guide\n");
        let _ = std::fs::remove_dir_all(&root);
    }
}
