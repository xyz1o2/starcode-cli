//! Local conversation-activity scanner shared by commands that learn from
//! transcripts (`/fewer-permission-prompts`, `/team-onboarding`).
//!
//! The scanner deliberately reads JSON values instead of deserializing the
//! session model: saved session snapshots under `.star/sessions` and the UI
//! transcript (`.star/transcript.jsonl`) use slightly different shapes, and a
//! single unknown field must not make the whole history unreadable. Anything
//! that cannot be parsed is counted and skipped. Both sources are project-local
//! on purpose: commands that write project state must not learn defaults from
//! another repository's history.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool: String,
    pub arguments: Value,
    pub timestamp: Option<DateTime<Utc>>,
}

impl ToolCallRecord {
    pub fn command(&self) -> Option<&str> {
        ["command", "cmd", "script", "shell_command"]
            .iter()
            .find_map(|key| self.arguments.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .filter(|command| !command.is_empty())
    }
}

#[derive(Debug, Clone)]
pub struct UserPromptRecord {
    pub content: String,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Aggregated local history. `sources` counts files that were readable and
/// yielded at least one recognizable event; `malformed` counts unreadable
/// files/lines rather than individual JSON fields.
#[derive(Debug, Clone, Default)]
pub struct ActivityScan {
    pub sources: usize,
    pub tool_calls: Vec<ToolCallRecord>,
    pub user_prompts: Vec<UserPromptRecord>,
    pub malformed: usize,
}

impl ActivityScan {
    pub fn is_empty(&self) -> bool {
        self.tool_calls.is_empty() && self.user_prompts.is_empty()
    }

    pub fn tool_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for call in &self.tool_calls {
            *counts.entry(call.tool.clone()).or_insert(0) += 1;
        }
        counts
    }

    pub fn mcp_server_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for call in &self.tool_calls {
            if let Some((server, _tool)) = split_mcp_tool(&call.tool) {
                *counts.entry(server.to_string()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Count slash commands typed by the user. Only a leading `/command` is
    /// considered; a slash in the middle of prose is not a command invocation.
    pub fn slash_command_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for prompt in &self.user_prompts {
            if let Some(command) = leading_slash_command(&prompt.content) {
                *counts.entry(command).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Names passed to the `skill` tool. Both `{"skill": ...}` (StarCode) and
    /// `{"name": ...}` (older aliases) are accepted, and local `SKILL.md` paths
    /// are reduced to their containing directory name.
    pub fn skill_invocations(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for call in &self.tool_calls {
            if !is_skill_tool(&call.tool) {
                continue;
            }
            let Some(name) = call
                .arguments
                .get("skill")
                .or_else(|| call.arguments.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            *counts.entry(normalize_skill_name(name)).or_insert(0) += 1;
        }
        counts
    }

    pub fn distinct_tools(&self) -> BTreeSet<String> {
        self.tool_calls
            .iter()
            .map(|call| call.tool.clone())
            .collect()
    }
}

fn is_skill_tool(name: &str) -> bool {
    name.eq_ignore_ascii_case("skill")
        || name.eq_ignore_ascii_case("skilltool")
        || name.eq_ignore_ascii_case("skill_tool")
}

fn normalize_skill_name(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches(['/', '\\']);
    let path = Path::new(trimmed);
    if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
        if let Some(parent) = path.parent().and_then(Path::file_name) {
            let parent = parent.to_string_lossy();
            if !parent.is_empty() {
                return parent.to_string();
            }
        }
        if let Some(stem) = path.file_stem() {
            return stem
                .to_string_lossy()
                .trim_end_matches(".skill")
                .to_string();
        }
    }
    trimmed.to_string()
}

pub fn split_mcp_tool(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    if server.is_empty() || tool.is_empty() {
        None
    } else {
        Some((server, tool))
    }
}

pub fn leading_slash_command(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix('/')?;
    let command: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':'))
        .collect();
    if command.is_empty() {
        None
    } else {
        Some(command.to_ascii_lowercase())
    }
}

/// Scan the current project's history. Saved session snapshots win when they
/// contain tool calls (they are the curated native transcript), otherwise the
/// UI transcript is used. This avoids counting every call twice after a
/// `/chat save` snapshot has been written.
pub fn scan_project_activity(project_root: &Path, cutoff: Option<DateTime<Utc>>) -> ActivityScan {
    let star = project_root.join(".star");
    let saved = scan_sessions_dir(&star.join("sessions"), cutoff);
    if !saved.tool_calls.is_empty() {
        return saved;
    }

    let mut merged = saved;
    let transcript = scan_transcript_file(&star.join("transcript.jsonl"), cutoff);
    merge_scan(&mut merged, transcript);
    merged
}

/// Scan an explicit transcript file. Public for tests and for commands that
/// want to inspect a single artifact.
pub fn scan_transcript_file(path: &Path, cutoff: Option<DateTime<Utc>>) -> ActivityScan {
    let mut scan = ActivityScan::default();
    let Ok(contents) = std::fs::read_to_string(path) else {
        return scan;
    };
    if contents.trim().is_empty() {
        return scan;
    }

    let mut seen_tool_call_ids = BTreeSet::new();
    let mut found = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            scan.malformed += 1;
            continue;
        };
        if collect_transcript_line(&value, cutoff, &mut scan, &mut seen_tool_call_ids) {
            found = true;
        }
    }
    if found {
        scan.sources += 1;
    }
    scan
}

/// Scan a directory of saved session snapshots (`<id>.json`).
pub fn scan_sessions_dir(dir: &Path, cutoff: Option<DateTime<Utc>>) -> ActivityScan {
    let mut scan = ActivityScan::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return scan;
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    paths.sort();

    for path in paths {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            scan.malformed += 1;
            continue;
        };
        let Ok(session) = serde_json::from_str::<Value>(&contents) else {
            scan.malformed += 1;
            continue;
        };
        if collect_session(&session, cutoff, &mut scan) {
            scan.sources += 1;
        }
    }
    scan
}

fn merge_scan(target: &mut ActivityScan, other: ActivityScan) {
    target.sources += other.sources;
    target.tool_calls.extend(other.tool_calls);
    target.user_prompts.extend(other.user_prompts);
    target.malformed += other.malformed;
}

fn collect_session(
    session: &Value,
    cutoff: Option<DateTime<Utc>>,
    scan: &mut ActivityScan,
) -> bool {
    let fallback_time = session
        .get("created_at")
        .and_then(Value::as_i64)
        .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0))
        .map(|time| time.with_timezone(&Utc));
    if let Some(cutoff) = cutoff {
        if fallback_time.is_some_and(|time| time < cutoff) {
            return false;
        }
    }

    let mut found = false;
    let mut history_tool_calls = 0usize;

    if let Some(history) = session.get("chat_history").and_then(Value::as_array) {
        for entry in history {
            let at = entry_time(entry).or(fallback_time);
            if !is_after_cutoff(at, cutoff) {
                continue;
            }
            if entry_is(entry, "user") {
                if let Some(content) = entry.get("content").and_then(Value::as_str) {
                    if !content.trim().is_empty() {
                        scan.user_prompts.push(UserPromptRecord {
                            content: content.to_string(),
                            timestamp: at,
                        });
                        found = true;
                    }
                }
            }
            if let Some(calls) = entry.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    if collect_tool_call(call, at, scan) {
                        history_tool_calls += 1;
                        found = true;
                    }
                }
            }
            if let Some(call) = entry.get("tool_call") {
                if collect_tool_call(call, at, scan) {
                    history_tool_calls += 1;
                    found = true;
                }
            }
        }
    }

    // Native agent messages duplicate the display transcript. Only consult them
    // when the display history did not expose any tool calls.
    if history_tool_calls == 0 {
        if let Some(messages) = session.get("agent_messages").and_then(Value::as_array) {
            for message in messages {
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        if collect_tool_call(call, fallback_time, scan) {
                            found = true;
                        }
                    }
                }
            }
        }
    }

    found
}

fn collect_transcript_line(
    line: &Value,
    cutoff: Option<DateTime<Utc>>,
    scan: &mut ActivityScan,
    seen_tool_call_ids: &mut BTreeSet<String>,
) -> bool {
    let at = transcript_time(line);
    if !is_after_cutoff(at, cutoff) {
        return false;
    }

    let event = line
        .get("event")
        .or_else(|| line.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    if event.eq_ignore_ascii_case("user")
        || line.get("role").and_then(Value::as_str) == Some("user")
    {
        if let Some(content) = line.get("content").and_then(Value::as_str) {
            if !content.trim().is_empty() {
                scan.user_prompts.push(UserPromptRecord {
                    content: content.to_string(),
                    timestamp: at,
                });
                return true;
            }
        }
    }

    if event.eq_ignore_ascii_case("tool_call") {
        if let Some(tool_call_id) = line.get("tool_call_id").and_then(Value::as_str) {
            if !seen_tool_call_ids.insert(tool_call_id.to_string()) {
                return false;
            }
        }
        let mut call = Value::Object(Default::default());
        if let Some(name) = line.get("name").and_then(Value::as_str) {
            call["function"]["name"] = Value::String(name.to_string());
        }
        if let Some(arguments) = line.get("arguments") {
            call["function"]["arguments"] = arguments.clone();
        }
        return collect_tool_call(&call, at, scan);
    }

    if let Some(calls) = line.get("tool_calls").and_then(Value::as_array) {
        let mut found = false;
        for call in calls {
            if collect_tool_call(call, at, scan) {
                found = true;
            }
        }
        return found;
    }

    false
}

fn entry_is(entry: &Value, wanted: &str) -> bool {
    entry
        .get("type")
        .or_else(|| entry.get("entry_type"))
        .and_then(Value::as_str)
        .map(|value| value.eq_ignore_ascii_case(wanted))
        .unwrap_or(false)
}

fn entry_time(entry: &Value) -> Option<DateTime<Utc>> {
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339)
}

fn transcript_time(line: &Value) -> Option<DateTime<Utc>> {
    line.get("ts")
        .or_else(|| line.get("timestamp"))
        .and_then(Value::as_str)
        .and_then(parse_rfc3339)
        .or_else(|| {
            line.get("timestamp")
                .and_then(Value::as_i64)
                .and_then(|ts| {
                    DateTime::<Utc>::from_timestamp(ts, 0).map(|time| time.with_timezone(&Utc))
                })
        })
}

fn parse_rfc3339(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|time| time.with_timezone(&Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|time| time.and_utc())
        })
}

fn is_after_cutoff(at: Option<DateTime<Utc>>, cutoff: Option<DateTime<Utc>>) -> bool {
    match (at, cutoff) {
        (Some(at), Some(cutoff)) => at >= cutoff,
        _ => true,
    }
}

fn collect_tool_call(call: &Value, at: Option<DateTime<Utc>>, scan: &mut ActivityScan) -> bool {
    let name = call
        .get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .or_else(|| call.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let Some(name) = name else {
        return false;
    };

    let arguments = call
        .get("function")
        .and_then(|function| function.get("arguments"))
        .or_else(|| call.get("arguments"))
        .or_else(|| call.get("input"));
    let arguments = match arguments {
        Some(Value::String(raw)) => {
            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::Object(Default::default()))
        }
        Some(value) => value.clone(),
        None => Value::Object(Default::default()),
    };

    scan.tool_calls.push(ToolCallRecord {
        tool: name.to_string(),
        arguments,
        timestamp: at,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "starcode-activity-{}-{}-{}",
            label,
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn utc(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn transcript_scan_reads_tool_calls_and_slash_commands() {
        let dir = unique_temp_dir("transcript");
        let path = dir.join("transcript.jsonl");
        fs::write(
            &path,
            concat!(
                r#"{"event":"user","content":"/compact focus on tests","ts":"2026-10-20T10:00:00Z"}"#,
                "\n",
                r#"{"event":"tool_call","name":"Bash","arguments":"{\"command\":\"git status --short\"}","tool_call_id":"c1","ts":"2026-10-20T10:00:01Z"}"#,
                "\n",
                r#"{"event":"tool_result","name":"Bash","tool_call_id":"c1","ts":"2026-10-20T10:00:02Z"}"#,
                "\n",
                r#"{"event":"tool_call","name":"skill","arguments":{"skill":"review","objective":"audit"},"tool_call_id":"c2","ts":"2026-10-20T10:00:03Z"}"#,
                "\n",
                "{not json\n",
            ),
        )
        .unwrap();

        let scan = scan_transcript_file(&path, None);
        assert_eq!(scan.sources, 1);
        assert_eq!(scan.tool_calls.len(), 2, "results must not become calls");
        assert_eq!(scan.malformed, 1);
        assert_eq!(scan.tool_counts().get("Bash"), Some(&1));
        assert_eq!(
            scan.tool_calls[0].command(),
            Some("git status --short"),
            "arguments are decoded from the JSON string used on disk"
        );
        assert_eq!(
            scan.slash_command_counts().get("compact"),
            Some(&1),
            "the command name is normalized without arguments"
        );
        assert_eq!(scan.skill_invocations().get("review"), Some(&1));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn transcript_cutoff_drops_old_events() {
        let dir = unique_temp_dir("cutoff");
        let path = dir.join("transcript.jsonl");
        fs::write(
            &path,
            concat!(
                r#"{"event":"tool_call","name":"Bash","arguments":{"command":"ls"},"ts":"2026-09-01T10:00:00Z"}"#,
                "\n",
                r#"{"event":"tool_call","name":"Bash","arguments":{"command":"pwd"},"ts":"2026-10-20T10:00:00Z"}"#,
                "\n",
            ),
        )
        .unwrap();

        let scan = scan_transcript_file(&path, Some(utc("2026-10-01T00:00:00Z")));
        assert_eq!(scan.tool_calls.len(), 1);
        assert_eq!(scan.tool_calls[0].command(), Some("pwd"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_session_scan_uses_display_history_without_double_counting_native_messages() {
        let dir = unique_temp_dir("session");
        let path = dir.join("saved.json");
        fs::write(
            &path,
            r#"{
                "id":"saved",
                "created_at":1760000000,
                "chat_history":[
                    {"type":"User","content":"/review this","timestamp":"2026-10-20T10:00:00Z"},
                    {"type":"ToolCall","content":"","timestamp":"2026-10-20T10:00:01Z",
                     "tool_call":{"id":"c1","type":"function","function":{"name":"Bash","arguments":"{\"command\":\"git diff\"}"}}}
                ],
                "agent_messages":[
                    {"role":"assistant","tool_calls":[
                        {"id":"c1","type":"function","function":{"name":"Bash","arguments":"{\"command\":\"git diff\"}"}}
                    ]}
                ]
            }"#,
        )
        .unwrap();

        let scan = scan_sessions_dir(&dir, None);
        assert_eq!(scan.sources, 1);
        assert_eq!(
            scan.tool_calls.len(),
            1,
            "the native snapshot duplicates the UI call"
        );
        assert_eq!(scan.user_prompts.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_session_scan_uses_agent_messages_when_display_history_has_no_calls() {
        let dir = unique_temp_dir("native");
        let path = dir.join("legacy.json");
        fs::write(
            &path,
            r#"{
                "id":"legacy",
                "created_at":1760000000,
                "chat_history":[{"type":"User","content":"hello","timestamp":"2026-10-20T10:00:00Z"}],
                "agent_messages":[
                    {"role":"assistant","tool_calls":[
                        {"id":"c1","type":"function","function":{"name":"mcp__exa__search","arguments":"{}"}}
                    ]}
                ]
            }"#,
        )
        .unwrap();

        let scan = scan_sessions_dir(&dir, None);
        assert_eq!(scan.tool_calls.len(), 1);
        assert_eq!(
            split_mcp_tool(&scan.tool_calls[0].tool),
            Some(("exa", "search"))
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_and_skill_helpers_ignore_incomplete_names() {
        assert_eq!(
            split_mcp_tool("mcp__github__list_issues"),
            Some(("github", "list_issues"))
        );
        assert_eq!(split_mcp_tool("mcp__github"), None);
        assert_eq!(split_mcp_tool("Bash"), None);
        assert_eq!(
            normalize_skill_name("C:\\work\\.star\\skills\\review\\SKILL.md"),
            "review"
        );
        assert_eq!(normalize_skill_name("review"), "review");
    }
}
