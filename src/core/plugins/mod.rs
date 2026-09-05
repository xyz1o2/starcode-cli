mod lifecycle;
pub mod marketplace;
pub mod mcp;
mod tool;

pub use lifecycle::{
    run_plugin_lifecycle, run_plugin_lifecycle_for_plugin, PluginLifecycleExecution,
    PluginLifecycleStage, PluginRuntimeLifecycle,
};
pub use mcp::{
    is_valid_plugin_mcp_server_name, qualify_plugin_mcp_server_name, PluginMcpServerConfig,
    ResolvedPluginMcpServer,
};
pub use tool::build_plugin_declarative_tools;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;

mod types;
pub(crate) use types::*;

fn storage(project_root: &Path) -> crate::core::config::storage::Storage {
    crate::core::config::storage::Storage::new(project_root.to_path_buf())
}

/// 安装范围对应的存储根目录：user → 用户主目录，project → 项目根
pub fn scope_target_dir(project_root: &Path, scope: &str) -> PathBuf {
    if scope == SCOPE_USER {
        dirs::home_dir().unwrap_or_else(|| project_root.to_path_buf())
    } else {
        project_root.to_path_buf()
    }
}

fn scope_storage(project_root: &Path, scope: &str) -> crate::core::config::storage::Storage {
    crate::core::config::storage::Storage::new(scope_target_dir(project_root, scope))
}

pub fn plugins_dir(project_root: &Path) -> PathBuf {
    plugins_dir_scoped(project_root, SCOPE_PROJECT)
}

pub fn plugins_dir_scoped(project_root: &Path, scope: &str) -> PathBuf {
    scope_storage(project_root, scope).extensions_dir()
}

pub fn manifest_path(project_root: &Path) -> PathBuf {
    manifest_path_scoped(project_root, SCOPE_PROJECT)
}

pub fn manifest_path_scoped(project_root: &Path, scope: &str) -> PathBuf {
    scope_storage(project_root, scope).extensions_config_path()
}

async fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }
    }
    Ok(())
}

pub async fn load_manifest(project_root: &Path) -> Result<PluginManifest, String> {
    load_manifest_scoped(project_root, SCOPE_PROJECT).await
}

pub async fn load_manifest_scoped(
    project_root: &Path,
    scope: &str,
) -> Result<PluginManifest, String> {
    let path = manifest_path_scoped(project_root, scope);
    if !path.exists() {
        return Ok(PluginManifest::default());
    }

    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;

    if text.trim().is_empty() {
        return Ok(PluginManifest::default());
    }

    serde_json::from_str(&text).map_err(|e| format!("failed to parse {}: {}", path.display(), e))
}

pub async fn save_manifest(project_root: &Path, manifest: &PluginManifest) -> Result<(), String> {
    save_manifest_scoped(project_root, SCOPE_PROJECT, manifest).await
}

pub async fn save_manifest_scoped(
    project_root: &Path,
    scope: &str,
    manifest: &PluginManifest,
) -> Result<(), String> {
    let path = manifest_path_scoped(project_root, scope);
    ensure_parent(&path).await?;

    let text = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("failed to serialize plugin manifest: {}", e))?;

    tokio::fs::write(&path, text)
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

pub async fn list_plugins(project_root: &Path) -> Result<Vec<PluginEntry>, String> {
    let manifest = load_manifest(project_root).await?;
    Ok(manifest.plugins)
}

pub async fn resolve_installed_plugins(project_root: &Path) -> Result<Vec<ResolvedPlugin>, String> {
    // 合并两个安装范围（对标 Claude Code）：project 优先于 user（同名覆盖）
    let mut entries: Vec<PluginEntry> = Vec::new();
    for scope in [SCOPE_USER, SCOPE_PROJECT] {
        let mut list = load_manifest_scoped(project_root, scope).await?.plugins;
        for e in &mut list {
            e.scope = scope.to_string();
        }
        entries.extend(list);
    }
    // 去重：project 覆盖 user（后写入者保留）
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::with_capacity(entries.len());
    for e in entries.into_iter().rev() {
        if seen.insert(e.name.clone()) {
            deduped.push(e);
        }
    }
    deduped.reverse();

    let mut resolved = Vec::with_capacity(deduped.len());

    for entry in deduped {
        let root = plugins_dir_scoped(project_root, &entry.scope).join(&entry.name);
        let root_exists = root.exists();
        let (manifest_path, runtime_manifest, runtime_error) = if root_exists {
            resolve_runtime_manifest(&root).await
        } else {
            (
                None,
                None,
                Some(format!("plugin directory not found: {}", root.display())),
            )
        };
        let runtime_warnings = runtime_manifest
            .as_ref()
            .map(|manifest| validate_runtime_manifest(&entry.name, manifest))
            .unwrap_or_default();

        resolved.push(ResolvedPlugin {
            entry,
            root,
            root_exists,
            manifest_path,
            runtime_manifest,
            runtime_error,
            runtime_warnings,
        });
    }

    Ok(resolved)
}

pub async fn inspect_plugin(
    project_root: &Path,
    plugin_name: &str,
) -> Result<Option<ResolvedPlugin>, String> {
    let plugins = resolve_installed_plugins(project_root).await?;
    Ok(plugins
        .into_iter()
        .find(|plugin| plugin.entry.name == plugin_name))
}

pub async fn discover_plugin_hooks(
    project_root: &Path,
) -> Result<Vec<PluginHookRegistration>, String> {
    let plugins = resolve_installed_plugins(project_root).await?;
    let mut hooks = Vec::new();

    for plugin in plugins {
        if !plugin.entry.enabled {
            continue;
        }

        let Some(runtime_manifest) = plugin.runtime_manifest.as_ref() else {
            continue;
        };

        for (event, specs) in &runtime_manifest.hooks {
            for (index, spec) in specs.iter().enumerate() {
                let spec = spec.command_spec();
                if spec.enabled == Some(false) || spec.command.trim().is_empty() {
                    continue;
                }

                hooks.push(PluginHookRegistration {
                    name: spec.name.unwrap_or_else(|| {
                        format!(
                            "plugin:{}:{}:{}",
                            plugin.entry.name,
                            event.trim(),
                            index + 1
                        )
                    }),
                    event: event.trim().to_string(),
                    command: spec.command.trim().to_string(),
                    timeout_secs: spec.timeout.unwrap_or(20).max(1),
                    blocking: spec.blocking.unwrap_or(false),
                    source: format!("plugin:{}", plugin.entry.name),
                    working_dir: plugin.root.clone(),
                });
            }
        }
    }

    Ok(hooks)
}

pub async fn discover_plugin_commands(
    project_root: &Path,
) -> Result<Vec<ResolvedPluginCommand>, String> {
    let plugins = resolve_installed_plugins(project_root).await?;
    let mut commands = Vec::new();

    for plugin in plugins {
        if !plugin.entry.enabled {
            continue;
        }

        let Some(runtime_manifest) = plugin.runtime_manifest.as_ref() else {
            continue;
        };

        for command in &runtime_manifest.commands {
            let name = command.name.trim();
            if !command.is_enabled()
                || name.is_empty()
                || command.command.trim().is_empty()
                || !is_valid_plugin_command_name(name)
            {
                continue;
            }

            commands.push(ResolvedPluginCommand {
                name: name.to_string(),
                description: command.description.trim().to_string(),
                command: command.command.trim().to_string(),
                timeout_secs: command.timeout.unwrap_or(120).max(1),
                source: format!("plugin:{}", plugin.entry.name),
                plugin_name: plugin.entry.name.clone(),
                working_dir: plugin.root.clone(),
            });
        }
    }

    Ok(commands)
}

pub async fn discover_plugin_tools(project_root: &Path) -> Result<Vec<ResolvedPluginTool>, String> {
    let plugins = resolve_installed_plugins(project_root).await?;
    let mut tools = Vec::new();

    for plugin in plugins {
        if !plugin.entry.enabled {
            continue;
        }

        let Some(runtime_manifest) = plugin.runtime_manifest.as_ref() else {
            continue;
        };

        for tool in &runtime_manifest.tools {
            let name = tool.name.trim();
            if !tool.is_enabled()
                || name.is_empty()
                || tool.command.trim().is_empty()
                || !is_valid_plugin_tool_name(name)
            {
                continue;
            }

            tools.push(ResolvedPluginTool {
                name: name.to_string(),
                description: tool.description.trim().to_string(),
                input_schema: tool.input_schema.clone(),
                command: tool.command.trim().to_string(),
                args: tool.args.clone(),
                required_permission: tool.required_permission,
                source: format!("plugin:{}", plugin.entry.name),
                plugin_name: plugin.entry.name.clone(),
                working_dir: plugin.root.clone(),
                project_root: project_root.to_path_buf(),
            });
        }
    }

    Ok(tools)
}

pub async fn discover_plugin_mcp_servers(
    project_root: &Path,
) -> Result<Vec<ResolvedPluginMcpServer>, String> {
    let plugins = resolve_installed_plugins(project_root).await?;
    let mut servers = Vec::new();

    for plugin in plugins {
        if !plugin.entry.enabled {
            continue;
        }

        let Some(runtime_manifest) = plugin.runtime_manifest.as_ref() else {
            continue;
        };

        for (server_name, config) in &runtime_manifest.mcp_servers {
            let server_name = server_name.trim();
            if config.disabled == Some(true)
                || server_name.is_empty()
                || config.command.trim().is_empty()
                || !is_valid_plugin_mcp_server_name(server_name)
            {
                continue;
            }

            servers.push(ResolvedPluginMcpServer {
                plugin_name: plugin.entry.name.clone(),
                server_name: server_name.to_string(),
                qualified_name: qualify_plugin_mcp_server_name(&plugin.entry.name, server_name),
                config: config.clone(),
                working_dir: plugin.root.clone(),
                project_root: project_root.to_path_buf(),
            });
        }
    }

    Ok(servers)
}

pub async fn resolve_plugin_command(
    project_root: &Path,
    command_name: &str,
) -> Result<Option<ResolvedPluginCommand>, String> {
    let name = command_name.trim();
    if name.is_empty() {
        return Ok(None);
    }

    let matches = discover_plugin_commands(project_root)
        .await?
        .into_iter()
        .filter(|command| command.name == name)
        .collect::<Vec<_>>();

    if matches.len() > 1 {
        let sources = matches
            .iter()
            .map(|command| command.source.clone())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "plugin command `{}` is ambiguous across {}",
            name, sources
        ));
    }

    Ok(matches.into_iter().next())
}

pub async fn execute_plugin_command(
    project_root: &Path,
    command_name: &str,
    args: &[String],
) -> Result<Option<PluginCommandExecution>, String> {
    let Some(command) = resolve_plugin_command(project_root, command_name).await? else {
        return Ok(None);
    };

    let args_json =
        serde_json::to_string(args).map_err(|e| format!("failed to encode plugin args: {}", e))?;
    let args_joined = args.join(" ");

    let mut process = if cfg!(target_os = "windows") {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(&command.command);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-lc").arg(&command.command);
        cmd
    };

    process.current_dir(&command.working_dir);
    process.env("STAR_PLUGIN_COMMAND_NAME", &command.name);
    process.env("STAR_PLUGIN_SOURCE", &command.source);
    process.env("STAR_PLUGIN_WORKING_DIR", &command.working_dir);
    process.env("STAR_PLUGIN_PROJECT_ROOT", project_root);
    process.env("STAR_PLUGIN_ARGS", &args_joined);
    process.env("STAR_PLUGIN_ARGS_JSON", &args_json);
    process.env("STAR_PLUGIN_ARGC", args.len().to_string());
    process.env("PAGER", "cat");
    process.env("GIT_PAGER", "cat");
    process.env("GIT_TERMINAL_PROMPT", "0");
    process.env("CI", "1");
    process.stdin(Stdio::null());
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(command.timeout_secs),
        process.output(),
    )
    .await;

    let execution = match output {
        Ok(Ok(output)) => PluginCommandExecution {
            command_name: command.name,
            plugin_name: command.plugin_name,
            source: command.source,
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            exit_code: output.status.code(),
            success: output.status.success(),
            timed_out: false,
        },
        Ok(Err(error)) => PluginCommandExecution {
            command_name: command.name,
            plugin_name: command.plugin_name,
            source: command.source,
            stdout: String::new(),
            stderr: format!("failed to execute plugin command: {}", error),
            exit_code: None,
            success: false,
            timed_out: false,
        },
        Err(_) => PluginCommandExecution {
            command_name: command.name,
            plugin_name: command.plugin_name,
            source: command.source,
            stdout: String::new(),
            stderr: format!("plugin command timed out after {}s", command.timeout_secs),
            exit_code: None,
            success: false,
            timed_out: true,
        },
    };

    Ok(Some(execution))
}

async fn prepare_install_destination(
    project_root: &Path,
    plugin_name: &str,
    scope: &str,
) -> Result<PathBuf, String> {
    let dst_root = plugins_dir_scoped(project_root, scope);
    if !dst_root.exists() {
        tokio::fs::create_dir_all(&dst_root)
            .await
            .map_err(|e| format!("failed to create plugin dir {}: {}", dst_root.display(), e))?;
    }

    let dst = dst_root.join(plugin_name);
    if dst.exists() {
        if dst.is_dir() {
            tokio::fs::remove_dir_all(&dst).await.map_err(|e| {
                format!(
                    "failed to replace existing plugin dir {}: {}",
                    dst.display(),
                    e
                )
            })?;
        } else {
            tokio::fs::remove_file(&dst).await.map_err(|e| {
                format!(
                    "failed to replace existing plugin file {}: {}",
                    dst.display(),
                    e
                )
            })?;
        }
    }

    Ok(dst)
}

async fn upsert_manifest_entry(
    project_root: &Path,
    plugin_name: &str,
    source: String,
    install_type: &str,
    scope: &str,
) -> Result<PluginEntry, String> {
    let mut manifest = load_manifest_scoped(project_root, scope).await?;
    manifest.plugins.retain(|p| p.name != plugin_name);

    let entry = PluginEntry {
        name: plugin_name.to_string(),
        source,
        install_type: install_type.to_string(),
        installed_at: Utc::now().timestamp(),
        enabled: true,
        scope: scope.to_string(),
    };

    manifest.plugins.push(entry.clone());
    save_manifest_scoped(project_root, scope, &manifest).await?;
    Ok(entry)
}

pub async fn install_plugin_local(
    project_root: &Path,
    source: &Path,
    plugin_name: &str,
    scope: &str,
) -> Result<PluginEntry, String> {
    if !source.exists() {
        return Err(format!("source path does not exist: {}", source.display()));
    }

    let dst = prepare_install_destination(project_root, plugin_name, scope).await?;

    if source.is_file() {
        tokio::fs::copy(source, &dst)
            .await
            .map_err(|e| format!("failed to copy plugin file: {}", e))?;
    } else {
        copy_dir_recursive(source, &dst)?;
    }

    upsert_manifest_entry(
        project_root,
        plugin_name,
        source.to_string_lossy().to_string(),
        "local",
        scope,
    )
    .await
}

pub async fn install_plugin_git(
    project_root: &Path,
    source: &str,
    plugin_name: &str,
    git_ref: Option<&str>,
    scope: &str,
) -> Result<PluginEntry, String> {
    let dst = prepare_install_destination(project_root, plugin_name, scope).await?;

    let clone = tokio::process::Command::new("git")
        .arg("clone")
        .arg(source)
        .arg(&dst)
        .output()
        .await
        .map_err(|e| format!("failed to run `git clone`: {}", e))?;
    if !clone.status.success() {
        let _ = tokio::fs::remove_dir_all(&dst).await;
        return Err(format!(
            "git clone failed (source={}): {}",
            source,
            summarize_process_output(&clone)
        ));
    }

    if let Some(reference) = git_ref.filter(|r| !r.trim().is_empty()) {
        let checkout = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&dst)
            .arg("checkout")
            .arg(reference)
            .output()
            .await
            .map_err(|e| format!("failed to run `git checkout`: {}", e))?;
        if !checkout.status.success() {
            let _ = tokio::fs::remove_dir_all(&dst).await;
            return Err(format!(
                "git checkout failed (ref={}): {}",
                reference,
                summarize_process_output(&checkout)
            ));
        }
    }

    let recorded_source = if let Some(reference) = git_ref.filter(|r| !r.trim().is_empty()) {
        format!("{}#{}", source, reference)
    } else {
        source.to_string()
    };

    upsert_manifest_entry(project_root, plugin_name, recorded_source, "git", scope).await
}

pub struct UpdatePluginResult {
    pub plugin_name: String,
    pub install_type: String,
    pub output: String,
    pub success: bool,
    pub error: Option<String>,
}

pub async fn update_plugin(
    project_root: &Path,
    plugin_name: &str,
) -> Result<UpdatePluginResult, String> {
    let plugin = inspect_plugin(project_root, plugin_name)
        .await?
        .ok_or_else(|| format!("plugin `{}` not found", plugin_name))?;

    if !plugin.root_exists {
        return Ok(UpdatePluginResult {
            plugin_name: plugin_name.to_string(),
            install_type: plugin.entry.install_type.clone(),
            output: String::new(),
            success: false,
            error: Some(format!(
                "plugin directory does not exist: {}",
                plugin.root.display()
            )),
        });
    }

    match plugin.entry.install_type.as_str() {
        "git" => {
            let git_dir = plugin.root.join(".git");
            if !git_dir.exists() {
                return Ok(UpdatePluginResult {
                    plugin_name: plugin_name.to_string(),
                    install_type: "git".to_string(),
                    output: String::new(),
                    success: false,
                    error: Some(format!(
                        "plugin directory is not a git repo: {}",
                        plugin.root.display()
                    )),
                });
            }

            let pull = tokio::process::Command::new("git")
                .arg("-C")
                .arg(&plugin.root)
                .arg("pull")
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .await
                .map_err(|e| format!("failed to run `git pull`: {}", e))?;

            if pull.status.success() {
                let mut manifest = load_manifest(project_root).await?;
                for entry in manifest.plugins.iter_mut() {
                    if entry.name == plugin_name {
                        entry.installed_at = Utc::now().timestamp();
                        break;
                    }
                }
                save_manifest(project_root, &manifest).await?;
            }

            let out = summarize_process_output(&pull);
            Ok(UpdatePluginResult {
                plugin_name: plugin_name.to_string(),
                install_type: "git".to_string(),
                output: out.clone(),
                success: pull.status.success(),
                error: if pull.status.success() {
                    None
                } else {
                    Some(out)
                },
            })
        }
        "local" => {
            let source_path = std::path::PathBuf::from(&plugin.entry.source);
            if !source_path.exists() {
                return Ok(UpdatePluginResult {
                    plugin_name: plugin_name.to_string(),
                    install_type: "local".to_string(),
                    output: String::new(),
                    success: false,
                    error: Some(format!(
                        "local source path no longer exists: {}",
                        plugin.entry.source
                    )),
                });
            }
            match install_plugin_local(project_root, &source_path, plugin_name, &plugin.entry.scope)
                .await
            {
                Ok(_) => Ok(UpdatePluginResult {
                    plugin_name: plugin_name.to_string(),
                    install_type: "local".to_string(),
                    output: format!("re-copied from {}", plugin.entry.source),
                    success: true,
                    error: None,
                }),
                Err(e) => Ok(UpdatePluginResult {
                    plugin_name: plugin_name.to_string(),
                    install_type: "local".to_string(),
                    output: String::new(),
                    success: false,
                    error: Some(e),
                }),
            }
        }
        t => Ok(UpdatePluginResult {
            plugin_name: plugin_name.to_string(),
            install_type: t.to_string(),
            output: String::new(),
            success: false,
            error: Some(format!("unsupported install type for update: `{}`", t)),
        }),
    }
}

pub async fn remove_plugin(project_root: &Path, plugin_name: &str) -> Result<bool, String> {
    // 范围感知卸载（对标 Claude Code）：project 优先，其次 user
    let mut removed_any = false;
    for scope in [SCOPE_PROJECT, SCOPE_USER] {
        let plugin_path = plugins_dir_scoped(project_root, scope).join(plugin_name);
        let mut removed_fs = false;
        if plugin_path.exists() {
            removed_fs = true;
            removed_any = true;
            if plugin_path.is_dir() {
                let _ = tokio::fs::remove_dir_all(&plugin_path).await;
            } else {
                let _ = tokio::fs::remove_file(&plugin_path).await;
            }
        }

        let mut manifest = load_manifest_scoped(project_root, scope).await?;
        let before = manifest.plugins.len();
        manifest.plugins.retain(|p| p.name != plugin_name);
        if before != manifest.plugins.len() {
            removed_any = true;
            save_manifest_scoped(project_root, scope, &manifest).await?;
        }
        if removed_fs {
            break; // 命中即止（同范围 FS+manifest 清理完）
        }
    }

    Ok(removed_any)
}

pub async fn set_plugin_enabled(
    project_root: &Path,
    plugin_name: &str,
    enabled: bool,
) -> Result<Option<PluginEnabledUpdate>, String> {
    // 范围感知启停：project 优先，其次 user
    for scope in [SCOPE_PROJECT, SCOPE_USER] {
        let mut manifest = load_manifest_scoped(project_root, scope).await?;
        let Some(entry) = manifest
            .plugins
            .iter_mut()
            .find(|entry| entry.name == plugin_name)
        else {
            continue;
        };

        let previous_enabled = entry.enabled;
        let changed = previous_enabled != enabled;
        entry.enabled = enabled;
        let updated = entry.clone();

        if changed {
            save_manifest_scoped(project_root, scope, &manifest).await?;
        }

        return Ok(Some(PluginEnabledUpdate {
            entry: updated,
            previous_enabled,
            changed,
        }));
    }

    Ok(None)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("failed to create {}: {}", dst.display(), e))?;

    for entry in walkdir::WalkDir::new(src) {
        let entry = entry.map_err(|e| format!("walkdir error: {}", e))?;
        let path = entry.path();
        let rel = path
            .strip_prefix(src)
            .map_err(|e| format!("strip_prefix error: {}", e))?;
        let target = dst.join(rel);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|e| format!("failed to create {}: {}", target.display(), e))?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
            }
            std::fs::copy(path, &target).map_err(|e| {
                format!(
                    "failed to copy {} -> {}: {}",
                    path.display(),
                    target.display(),
                    e
                )
            })?;
        }
    }

    Ok(())
}

pub fn normalize_plugin_name(input: &str) -> String {
    let mut out = String::new();
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        }
    }
    out
}

pub fn resolve_plugin_git_source(input: &str) -> String {
    let source = input.trim();
    if looks_like_github_shorthand(source) {
        format!("https://github.com/{}.git", source)
    } else {
        source.to_string()
    }
}

pub fn infer_plugin_name_from_git_source(input: &str) -> Option<String> {
    let mut source = input.trim().trim_end_matches('/');
    if source.is_empty() {
        return None;
    }
    if let Some((left, _)) = source.split_once('#') {
        source = left;
    }

    let mut tail = source;
    if let Some(idx) = source.rfind('/') {
        tail = &source[(idx + 1)..];
    }
    if let Some(idx) = tail.rfind(':') {
        tail = &tail[(idx + 1)..];
    }
    if let Some(stripped) = tail.strip_suffix(".git") {
        tail = stripped;
    }

    let name = normalize_plugin_name(tail);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

async fn resolve_runtime_manifest(
    plugin_root: &Path,
) -> (
    Option<PathBuf>,
    Option<PluginRuntimeManifest>,
    Option<String>,
) {
    for relative_path in PLUGIN_RUNTIME_MANIFEST_CANDIDATES {
        let candidate = plugin_root.join(relative_path);
        if !candidate.exists() {
            continue;
        }

        let text = match tokio::fs::read_to_string(&candidate).await {
            Ok(text) => text,
            Err(error) => {
                return (
                    Some(candidate.clone()),
                    None,
                    Some(format!("failed to read {}: {}", candidate.display(), error)),
                );
            }
        };

        match crate::core::config::json_with_comments::parse_json_with_comments::<
            PluginRuntimeManifest,
        >(&text)
        {
            Ok(manifest) => return (Some(candidate), Some(manifest), None),
            Err(error) => {
                return (
                    Some(candidate.clone()),
                    None,
                    Some(format!(
                        "failed to parse {}: {}",
                        candidate.display(),
                        error
                    )),
                );
            }
        }
    }

    (None, None, None)
}

fn is_valid_plugin_command_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('/') && !name.chars().any(|ch| ch.is_whitespace())
}

fn is_valid_plugin_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn validate_mcp_servers(servers: &HashMap<String, PluginMcpServerConfig>) -> Vec<String> {
    let mut warnings = Vec::new();

    for (name, config) in servers {
        let name = name.trim();
        if name.is_empty() {
            warnings.push("mcpServers entry has an empty name".to_string());
        } else if !is_valid_plugin_mcp_server_name(name) {
            warnings.push(format!(
                "mcpServers name `{}` is invalid; server names may only use letters, numbers, `_` and `-`",
                name
            ));
        }

        if config.command.trim().is_empty() {
            warnings.push(format!(
                "mcpServers entry `{}` has an empty command",
                if name.is_empty() { "<unnamed>" } else { name }
            ));
        }
    }

    warnings
}

fn validate_runtime_manifest(entry_name: &str, manifest: &PluginRuntimeManifest) -> Vec<String> {
    let mut warnings = Vec::new();

    if let Some(runtime_name) = manifest
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let normalized = normalize_plugin_name(runtime_name);
        if !normalized.is_empty() && normalized != entry_name {
            warnings.push(format!(
                "manifest name `{}` does not match installed plugin name `{}`",
                runtime_name, entry_name
            ));
        }
    }

    let mut hook_names = HashSet::new();
    for (event, specs) in &manifest.hooks {
        if crate::core::hooks::store::ManagedHookEvent::parse(event).is_none() {
            warnings.push(format!(
                "hook event `{}` is not recognized and may be ignored by the runtime",
                event
            ));
        }

        for (index, spec) in specs.iter().enumerate() {
            let spec = spec.command_spec();
            if spec.enabled == Some(false) {
                continue;
            }
            if spec.command.trim().is_empty() {
                warnings.push(format!(
                    "hook entry #{} for event `{}` has an empty command",
                    index + 1,
                    event
                ));
            }

            if let Some(name) = spec
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                if !hook_names.insert(format!("{}:{}", event, name)) {
                    warnings.push(format!(
                        "hook name `{}` is duplicated under event `{}`",
                        name, event
                    ));
                }
            }
        }
    }

    let mut command_names = HashSet::new();
    for (index, command) in manifest.commands.iter().enumerate() {
        if !command.is_enabled() {
            continue;
        }

        let name = command.name.trim();
        if name.is_empty() {
            warnings.push(format!("command entry #{} is missing a name", index + 1));
        } else {
            if !is_valid_plugin_command_name(name) {
                warnings.push(format!(
                    "command `{}` is invalid; slash command names cannot contain whitespace or `/`",
                    name
                ));
            }
            if !command_names.insert(name.to_string()) {
                warnings.push(format!("command `{}` is defined more than once", name));
            }
        }

        if command.command.trim().is_empty() {
            warnings.push(format!(
                "command entry #{} (`{}`) has an empty command",
                index + 1,
                if name.is_empty() { "<unnamed>" } else { name }
            ));
        }
    }

    let mut tool_names = HashSet::new();
    for (index, tool) in manifest.tools.iter().enumerate() {
        if !tool.is_enabled() {
            continue;
        }

        let name = tool.name.trim();
        if name.is_empty() {
            warnings.push(format!("tool entry #{} is missing a name", index + 1));
        } else {
            if !is_valid_plugin_tool_name(name) {
                warnings.push(format!(
                    "tool `{}` is invalid; tool names may only use letters, numbers, `_` and `-`",
                    name
                ));
            }
            if !tool_names.insert(name.to_string()) {
                warnings.push(format!("tool `{}` is defined more than once", name));
            }
        }

        if tool.command.trim().is_empty() {
            warnings.push(format!(
                "tool entry #{} (`{}`) has an empty command",
                index + 1,
                if name.is_empty() { "<unnamed>" } else { name }
            ));
        }
    }

    warnings.extend(validate_lifecycle_stage(
        manifest.lifecycle.init.iter(),
        PluginLifecycleStage::Init,
    ));
    warnings.extend(validate_lifecycle_stage(
        manifest.lifecycle.shutdown.iter(),
        PluginLifecycleStage::Shutdown,
    ));

    warnings.extend(validate_mcp_servers(&manifest.mcp_servers));

    warnings
}

fn validate_lifecycle_stage<'a>(
    specs: impl Iterator<Item = &'a lifecycle::PluginLifecycleSpec>,
    stage: PluginLifecycleStage,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut names: HashSet<String> = HashSet::new();

    for (index, spec) in specs.enumerate() {
        let spec = spec.command_spec();
        if spec.enabled == Some(false) {
            continue;
        }

        if spec.command.trim().is_empty() {
            warnings.push(format!(
                "lifecycle {} entry #{} has an empty command",
                stage.as_str(),
                index + 1
            ));
        }

        if let Some(name) = spec
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name: &&str| !name.is_empty())
        {
            if !names.insert(name.to_string()) {
                warnings.push(format!(
                    "lifecycle {} name `{}` is defined more than once",
                    stage.as_str(),
                    name
                ));
            }
        }
    }

    warnings
}

fn default_plugin_tool_input_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

fn summarize_process_output(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("exit status {:?}", output.status.code())
    }
}

fn looks_like_github_shorthand(input: &str) -> bool {
    if input.is_empty() || input.contains("://") || input.starts_with("git@") {
        return false;
    }
    if input.contains(' ') || input.contains('\\') || input.starts_with('/') {
        return false;
    }

    let mut parts = input.split('/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();

    if parts.next().is_some() || owner.is_empty() || repo.is_empty() {
        return false;
    }

    let valid_part = |part: &str| {
        part.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    };
    valid_part(owner) && valid_part(repo)
}
