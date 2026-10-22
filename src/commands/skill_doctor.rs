//! `/skill-doctor` — validate locally authored skill files.
//!
//! Claude Code's version reports how much of the context window loaded skills
//! consume. StarCode deliberately does not auto-inject `.star/skills` into the
//! model context yet (see `docs/claude-code-alignment.md`), so reporting a
//! context cost here would be fiction. What *is* real and useful is the
//! authoring contract: required frontmatter, name/description, body, duplicate
//! names, disabled skills, and size warnings. The report says explicitly that
//! these files are authoring/list resources, not injected context.

use crate::agent::skills::loader::SkillLoader;
use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const USAGE: &str = concat!(
    "Usage: /skill-doctor [--all]\n",
    "\n",
    "Checks `.star/skills` for missing frontmatter, names, descriptions, empty\n",
    "bodies, duplicate names, disabled skills, and oversized files.\n",
    "\n",
    "  --all       Also scan the user-level skills directory\n",
    "  -h, --help  Show this help\n",
);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    include_user: bool,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        include_user: false,
    };
    for arg in args {
        match arg.trim() {
            "--all" => options.include_user = true,
            other => return Err(format!("Unknown argument `{}`.\n\n{}", other, USAGE)),
        }
    }
    Ok(options)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Severity {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
struct SkillDiagnosis {
    scope: &'static str,
    path: PathBuf,
    name: String,
    directory: String,
    bytes: usize,
    body_chars: usize,
    errors: Vec<String>,
    warnings: Vec<String>,
}

impl SkillDiagnosis {
    fn severity(&self) -> Severity {
        if !self.errors.is_empty() {
            Severity::Error
        } else if !self.warnings.is_empty() {
            Severity::Warning
        } else {
            Severity::Ok
        }
    }
}

fn discover_skill_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skill = path.join("SKILL.md");
            if skill.is_file() {
                paths.push(skill);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".skill.md"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths
}

fn diagnose_file(scope: &'static str, path: &Path) -> SkillDiagnosis {
    let directory = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());
    let mut diagnosis = SkillDiagnosis {
        scope,
        path: path.to_path_buf(),
        name: "(missing)".to_string(),
        directory,
        bytes: 0,
        body_chars: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
    };

    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            diagnosis
                .errors
                .push(format!("cannot read file: {}", error));
            return diagnosis;
        }
    };
    diagnosis.bytes = raw.len();

    let has_frontmatter = has_complete_frontmatter(&raw);
    if !has_frontmatter {
        diagnosis
            .errors
            .push("missing or unclosed YAML frontmatter delimited by `---`".to_string());
    }

    let (metadata, body) = if has_frontmatter {
        SkillLoader::parse_frontmatter(&raw)
    } else {
        (Default::default(), raw.clone())
    };
    diagnosis.body_chars = body.chars().count();
    if metadata.name.trim().is_empty() {
        if has_frontmatter {
            diagnosis
                .errors
                .push("name is empty; /skills list would show `unknown`".to_string());
        }
    } else {
        diagnosis.name = metadata.name.trim().to_string();
    }
    if metadata.description.trim().is_empty() {
        diagnosis
            .warnings
            .push("description is empty; /skills list will not explain the skill".to_string());
    }
    if has_frontmatter && body.trim().is_empty() {
        diagnosis
            .warnings
            .push("skill body is empty; there are no instructions to load".to_string());
    }
    if metadata.disabled {
        diagnosis
            .warnings
            .push("disabled: true; the skill loader will skip this file".to_string());
    }
    if diagnosis.name != "(missing)"
        && diagnosis.directory != "."
        && diagnosis.name != diagnosis.directory
    {
        diagnosis.warnings.push(format!(
            "frontmatter name `{}` does not match directory `{}`",
            diagnosis.name, diagnosis.directory
        ));
    }
    if diagnosis.bytes > 64 * 1024 {
        diagnosis.warnings.push(format!(
            "{} is large; keep SKILL.md focused and move details into references/",
            format_bytes(diagnosis.bytes)
        ));
    }

    diagnosis
}

/// Require a first line of `---` and a later line of `---`. The loader uses a
/// plain substring search, so a file without a real closing delimiter can still
/// produce metadata; the doctor must not bless that shape.
fn has_complete_frontmatter(content: &str) -> bool {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    lines.any(|line| line.trim() == "---")
}

fn diagnose_roots(cwd: &Path, include_user: bool) -> (Vec<SkillDiagnosis>, Vec<String>) {
    let mut roots: Vec<(&'static str, PathBuf)> =
        vec![("project", cwd.join(".star").join("skills"))];
    if include_user {
        roots.push((
            "user",
            crate::core::config::storage::Storage::user_skills_dir(),
        ));
    }

    let mut diagnoses = Vec::new();
    let mut locations = Vec::new();
    for (scope, root) in roots {
        let files = discover_skill_files(&root);
        if root.is_dir() {
            locations.push(root.display().to_string());
        }
        for path in files {
            diagnoses.push(diagnose_file(scope, &path));
        }
    }

    add_duplicate_warnings(&mut diagnoses);
    diagnoses.sort_by(|left, right| left.path.cmp(&right.path));
    (diagnoses, locations)
}

fn add_duplicate_warnings(diagnoses: &mut [SkillDiagnosis]) {
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, diagnosis) in diagnoses.iter().enumerate() {
        if diagnosis.name != "(missing)" {
            by_name
                .entry(diagnosis.name.clone())
                .or_default()
                .push(index);
        }
    }
    for (name, indexes) in by_name {
        if indexes.len() < 2 {
            continue;
        }
        for index in indexes {
            let others = diagnoses
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .filter(|(_, other)| other.name == name)
                .map(|(_, other)| other.path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            diagnoses[index].warnings.push(format!(
                "duplicate skill name `{}` also used by {}",
                name, others
            ));
        }
    }
}

fn render_report(diagnoses: &[SkillDiagnosis], locations: &[String], include_user: bool) -> String {
    let mut out = String::from("# Skill doctor\n\n");
    if locations.is_empty() {
        out.push_str("No skill directories were found.\n");
    } else {
        out.push_str(&format!(
            "Scanned {} location(s): {}.\n",
            locations.len(),
            locations
                .iter()
                .map(|location| format!("`{}`", location))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !include_user {
        out.push_str("User-level skills are not included; pass `--all` to scan them.\n");
    }

    let errors = diagnoses
        .iter()
        .filter(|diagnosis| diagnosis.severity() == Severity::Error)
        .count();
    let warnings = diagnoses
        .iter()
        .filter(|diagnosis| diagnosis.severity() == Severity::Warning)
        .count();
    let healthy = diagnoses.len().saturating_sub(errors + warnings);
    out.push_str(&format!(
        "\nFound {} skill file(s): {} healthy, {} with warnings, {} with errors.\n",
        diagnoses.len(),
        healthy,
        warnings,
        errors
    ));
    out.push_str(
        "\n> `.star/skills` files are authoring/list resources in StarCode; they are not \
         automatically injected into model context. Sizes below are file sizes, not \
         context-window cost.\n",
    );

    if diagnoses.is_empty() {
        out.push_str(
            "\nNo skill files found. Create `.star/skills/<name>/SKILL.md` or run \
             `/skills new <name>`.\n",
        );
        return out;
    }

    let errors: Vec<&SkillDiagnosis> = diagnoses
        .iter()
        .filter(|diagnosis| diagnosis.severity() == Severity::Error)
        .collect();
    let warnings: Vec<&SkillDiagnosis> = diagnoses
        .iter()
        .filter(|diagnosis| diagnosis.severity() == Severity::Warning)
        .collect();
    if !errors.is_empty() {
        out.push_str("\n## Errors\n\n");
        for diagnosis in errors {
            out.push_str(&format!(
                "- `{}`: {}\n",
                diagnosis.path.display(),
                diagnosis.errors.join("; ")
            ));
        }
    }
    if !warnings.is_empty() {
        out.push_str("\n## Warnings\n\n");
        for diagnosis in warnings {
            out.push_str(&format!(
                "- `{}`: {}\n",
                diagnosis.path.display(),
                diagnosis.warnings.join("; ")
            ));
        }
    }

    out.push_str("\n## Skills\n\n");
    for diagnosis in diagnoses {
        let marker = match diagnosis.severity() {
            Severity::Ok => "ok",
            Severity::Warning => "warn",
            Severity::Error => "error",
        };
        out.push_str(&format!(
            "- [{}] `{}` ({}) — {} bytes, {} body char(s), `{}`\n",
            marker,
            diagnosis.name,
            diagnosis.scope,
            diagnosis.bytes,
            diagnosis.body_chars,
            diagnosis.path.display()
        ));
        if diagnosis.description_placeholder() {
            out.push_str("  - description missing\n");
        }
    }
    out
}

impl SkillDiagnosis {
    fn description_placeholder(&self) -> bool {
        self.warnings
            .iter()
            .any(|warning| warning.starts_with("description is empty"))
    }
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
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
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (diagnoses, locations) = diagnose_roots(&cwd, options.include_user);
    reply(
        ctx.state,
        render_report(&diagnoses, &locations, options.include_user),
    );
    Ok(())
}

fn reply(state: &mut crate::ui::state::store::ChatState, msg: impl Into<String>) {
    state
        .chat_history
        .push(ChatEntry::assistant(msg).with_streaming(false));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "starcode-skill-doctor-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_skill(root: &Path, name: &str, content: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn parses_all_flag() {
        assert_eq!(
            parse_options(&["--all".to_string()]).unwrap(),
            Options { include_user: true }
        );
        assert!(!parse_options(&[]).unwrap().include_user);
        assert!(parse_options(&["--wat".to_string()]).is_err());
    }

    #[test]
    fn usage_contains_real_newlines() {
        assert!(USAGE.contains('\n'));
        assert!(!USAGE.contains("\\n"));
        assert!(USAGE.contains("--all"));
    }

    #[test]
    fn format_bytes_switches_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
    }

    #[test]
    fn valid_skill_is_healthy() {
        let root = temp_dir("valid");
        let path = write_skill(
            &root,
            "review",
            "---\nname: review\ndescription: Review a change\n---\n\nRead the diff and report bugs.\n",
        );
        let diagnosis = diagnose_file("project", &path);
        assert_eq!(diagnosis.severity(), Severity::Ok);
        assert_eq!(diagnosis.name, "review");
        assert!(diagnosis.body_chars > 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_frontmatter_is_an_error() {
        let root = temp_dir("missing-fm");
        let path = write_skill(&root, "bad", "No frontmatter here.\n");
        let diagnosis = diagnose_file("project", &path);
        assert_eq!(diagnosis.severity(), Severity::Error);
        assert!(diagnosis.errors[0].contains("frontmatter"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unclosed_frontmatter_is_an_error() {
        let root = temp_dir("unclosed-fm");
        let path = write_skill(&root, "bad", "---\nname: bad\ndescription: x\n");
        let diagnosis = diagnose_file("project", &path);
        assert_eq!(diagnosis.severity(), Severity::Error);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_name_is_an_error_and_missing_description_body_warn() {
        let root = temp_dir("sparse");
        let path = write_skill(&root, "sparse", "---\n---\n");
        let diagnosis = diagnose_file("project", &path);
        assert_eq!(diagnosis.severity(), Severity::Error);
        assert!(diagnosis
            .errors
            .iter()
            .any(|error| error.contains("name is empty")));
        assert!(diagnosis
            .warnings
            .iter()
            .any(|warning| warning.contains("description is empty")));
        assert!(diagnosis
            .warnings
            .iter()
            .any(|warning| warning.contains("body is empty")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn name_mismatch_and_disabled_are_reported() {
        let root = temp_dir("mismatch");
        let path = write_skill(
            &root,
            "review",
            "---\nname: other\ndescription: Review\n---\n\nBody.\n",
        );
        let diagnosis = diagnose_file("project", &path);
        assert!(diagnosis
            .warnings
            .iter()
            .any(|warning| warning.contains("does not match directory")));
        let _ = fs::remove_dir_all(&root);

        let root = temp_dir("disabled");
        let path = write_skill(
            &root,
            "off",
            "---\nname: off\ndescription: Off\ndisabled: true\n---\n\nBody.\n",
        );
        let diagnosis = diagnose_file("project", &path);
        assert!(diagnosis
            .warnings
            .iter()
            .any(|warning| warning.contains("disabled")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn discovers_directories_and_dot_skill_files() {
        let root = temp_dir("discover");
        let dir_skill = write_skill(
            &root,
            "review",
            "---\nname: review\ndescription: Review\n---\n\nBody.\n",
        );
        let flat = root.join("explain.skill.md");
        fs::write(
            &flat,
            "---\nname: explain\ndescription: Explain\n---\n\nBody.\n",
        )
        .unwrap();
        let found = discover_skill_files(&root);
        assert_eq!(found, vec![flat, dir_skill]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_names_get_warnings_on_every_copy() {
        let root = temp_dir("duplicate");
        let first = write_skill(
            &root,
            "one",
            "---\nname: review\ndescription: One\n---\n\nBody.\n",
        );
        let second = write_skill(
            &root,
            "two",
            "---\nname: review\ndescription: Two\n---\n\nBody.\n",
        );
        let mut diagnoses = vec![
            diagnose_file("project", &first),
            diagnose_file("project", &second),
        ];
        add_duplicate_warnings(&mut diagnoses);
        for diagnosis in &diagnoses {
            assert!(diagnosis
                .warnings
                .iter()
                .any(|warning| warning.contains("duplicate skill name `review`")));
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn oversized_skill_gets_a_focused_warning() {
        let root = temp_dir("large");
        let content = format!(
            "---\nname: big\ndescription: Big\n---\n\n{}\n",
            "x".repeat(70 * 1024)
        );
        let path = write_skill(&root, "big", &content);
        let diagnosis = diagnose_file("project", &path);
        assert_eq!(diagnosis.severity(), Severity::Warning);
        assert!(diagnosis
            .warnings
            .iter()
            .any(|warning| warning.contains("is large")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn diagnose_roots_finds_project_skills_under_dot_star() {
        let root = temp_dir("roots");
        let skills_root = root.join(".star").join("skills");
        write_skill(
            &skills_root,
            "review",
            "---\nname: review\ndescription: Review\n---\n\nBody.\n",
        );
        let (diagnoses, locations) = diagnose_roots(&root, false);
        assert_eq!(diagnoses.len(), 1);
        assert_eq!(diagnoses[0].scope, "project");
        assert_eq!(locations, vec![skills_root.display().to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn report_is_explicit_about_context_cost_and_counts() {
        let root = temp_dir("report");
        let healthy = write_skill(
            &root,
            "review",
            "---\nname: review\ndescription: Review\n---\n\nBody.\n",
        );
        let sparse = write_skill(
            &root,
            "sparse",
            "---\nname: sparse\ndescription: Sparse\n---\n",
        );
        let broken = write_skill(&root, "broken", "no frontmatter\n");
        let diagnoses = vec![
            diagnose_file("project", &healthy),
            diagnose_file("project", &sparse),
            diagnose_file("project", &broken),
        ];
        let report = render_report(&diagnoses, &[root.display().to_string()], false);
        assert!(report.contains("1 healthy, 1 with warnings, 1 with errors"));
        assert!(report.contains("not automatically injected into model context"));
        assert!(report.contains("Sizes below are file sizes, not context-window cost"));
        assert!(report.contains("[ok] `review`"));
        assert!(report.contains("[warn] `sparse`"));
        assert!(report.contains("[error] `(missing)`"));
        assert!(report.contains("## Warnings"));
        assert!(report.contains("--all"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_report_suggests_where_to_create_a_skill() {
        let report = render_report(&[], &[], false);
        assert!(report.contains("No skill directories were found"));
        assert!(report.contains("No skill files found"));
        assert!(report.contains(".star/skills/<name>/SKILL.md"));
    }
}
