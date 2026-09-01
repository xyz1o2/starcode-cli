use super::resolve_installed_plugins;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;

const DEFAULT_PLUGIN_LIFECYCLE_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginLifecycleStage {
    Init,
    Shutdown,
}

impl PluginLifecycleStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginRuntimeLifecycle {
    #[serde(rename = "Init", alias = "init", default)]
    pub init: Vec<PluginLifecycleSpec>,
    #[serde(rename = "Shutdown", alias = "shutdown", default)]
    pub shutdown: Vec<PluginLifecycleSpec>,
}

impl PluginRuntimeLifecycle {
    pub fn enabled_command_count(&self) -> usize {
        self.init
            .iter()
            .chain(self.shutdown.iter())
            .filter(|spec| spec.is_enabled())
            .count()
    }

    pub fn enabled_stage_count(&self, stage: PluginLifecycleStage) -> usize {
        self.specs(stage)
            .iter()
            .filter(|spec| spec.is_enabled())
            .count()
    }

    fn specs(&self, stage: PluginLifecycleStage) -> &[PluginLifecycleSpec] {
        match stage {
            PluginLifecycleStage::Init => &self.init,
            PluginLifecycleStage::Shutdown => &self.shutdown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginLifecycleSpec {
    Command(String),
    Detailed(PluginLifecycleCommand),
}

impl PluginLifecycleSpec {
    pub(crate) fn command_spec(&self) -> PluginLifecycleCommand {
        match self {
            Self::Command(command) => PluginLifecycleCommand {
                command: command.clone(),
                ..PluginLifecycleCommand::default()
            },
            Self::Detailed(spec) => spec.clone(),
        }
    }

    fn is_enabled(&self) -> bool {
        self.command_spec().enabled != Some(false)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginLifecycleCommand {
    pub command: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ResolvedPluginLifecycleCommand {
    pub name: String,
    pub stage: PluginLifecycleStage,
    pub command: String,
    pub timeout_secs: u64,
    pub source: String,
    pub plugin_name: String,
    pub working_dir: PathBuf,
    pub project_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PluginLifecycleExecution {
    pub name: String,
    pub stage: PluginLifecycleStage,
    pub plugin_name: String,
    pub source: String,
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
    pub error: Option<String>,
}

pub async fn discover_plugin_lifecycle_commands(
    project_root: &Path,
    stage: PluginLifecycleStage,
) -> Result<Vec<ResolvedPluginLifecycleCommand>, String> {
    let mut plugins = resolve_installed_plugins(project_root).await?;
    if stage == PluginLifecycleStage::Shutdown {
        plugins.reverse();
    }

    let mut commands = Vec::new();

    for plugin in plugins {
        if !plugin.entry.enabled {
            continue;
        }

        let Some(runtime_manifest) = plugin.runtime_manifest.as_ref() else {
            continue;
        };

        for (index, spec) in runtime_manifest.lifecycle.specs(stage).iter().enumerate() {
            let spec = spec.command_spec();
            if spec.enabled == Some(false) || spec.command.trim().is_empty() {
                continue;
            }

            commands.push(ResolvedPluginLifecycleCommand {
                name: spec.name.unwrap_or_else(|| {
                    format!(
                        "plugin:{}:{}:{}",
                        plugin.entry.name,
                        stage.as_str(),
                        index + 1
                    )
                }),
                stage,
                command: spec.command.trim().to_string(),
                timeout_secs: spec
                    .timeout
                    .unwrap_or(DEFAULT_PLUGIN_LIFECYCLE_TIMEOUT_SECS)
                    .max(1),
                source: format!("plugin:{}", plugin.entry.name),
                plugin_name: plugin.entry.name.clone(),
                working_dir: plugin.root.clone(),
                project_root: project_root.to_path_buf(),
            });
        }
    }

    Ok(commands)
}

pub async fn run_plugin_lifecycle(
    project_root: &Path,
    stage: PluginLifecycleStage,
) -> Result<Vec<PluginLifecycleExecution>, String> {
    let commands = discover_plugin_lifecycle_commands(project_root, stage).await?;
    run_plugin_lifecycle_commands(commands).await
}

pub async fn run_plugin_lifecycle_for_plugin(
    project_root: &Path,
    plugin_name: &str,
    stage: PluginLifecycleStage,
) -> Result<Vec<PluginLifecycleExecution>, String> {
    let commands = discover_plugin_lifecycle_commands(project_root, stage)
        .await?
        .into_iter()
        .filter(|command| command.plugin_name == plugin_name)
        .collect::<Vec<_>>();
    run_plugin_lifecycle_commands(commands).await
}

async fn run_plugin_lifecycle_commands(
    commands: Vec<ResolvedPluginLifecycleCommand>,
) -> Result<Vec<PluginLifecycleExecution>, String> {
    let mut results = Vec::with_capacity(commands.len());
    for command in commands {
        results.push(run_one_plugin_lifecycle_command(command).await);
    }
    Ok(results)
}

async fn run_one_plugin_lifecycle_command(
    command: ResolvedPluginLifecycleCommand,
) -> PluginLifecycleExecution {
    let mut process = if cfg!(target_os = "windows") {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(&command.command);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-lc").arg(&command.command);
        cmd
    };

    process
        .current_dir(&command.working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("STAR_PLUGIN_LIFECYCLE_STAGE", command.stage.as_str())
        .env("STAR_PLUGIN_LIFECYCLE_NAME", &command.name)
        .env("STAR_PLUGIN_NAME", &command.plugin_name)
        .env("STAR_PLUGIN_SOURCE", &command.source)
        .env("STAR_PLUGIN_ROOT", &command.working_dir)
        .env("STAR_PLUGIN_PROJECT_ROOT", &command.project_root)
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("CI", "1");

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(command.timeout_secs),
        process.output(),
    )
    .await;

    match output {
        Ok(Ok(output)) => PluginLifecycleExecution {
            name: command.name,
            stage: command.stage,
            plugin_name: command.plugin_name,
            source: command.source,
            command: command.command,
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            exit_code: output.status.code(),
            success: output.status.success(),
            timed_out: false,
            error: None,
        },
        Ok(Err(error)) => PluginLifecycleExecution {
            name: command.name,
            stage: command.stage,
            plugin_name: command.plugin_name,
            source: command.source,
            command: command.command,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            success: false,
            timed_out: false,
            error: Some(format!("failed to execute lifecycle command: {}", error)),
        },
        Err(_) => PluginLifecycleExecution {
            name: command.name,
            stage: command.stage,
            plugin_name: command.plugin_name,
            source: command.source,
            command: command.command,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            success: false,
            timed_out: true,
            error: Some(format!(
                "lifecycle command timed out after {}s",
                command.timeout_secs
            )),
        },
    }
}
