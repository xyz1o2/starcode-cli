use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct TerminalSetup {
    pub shell: ShellType,
    pub config_path: String,
    pub integration_installed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Cmd,
}

impl TerminalSetup {
    pub fn detect() -> Self {
        let shell = if cfg!(target_os = "windows") {
            if std::env::var("PSModulePath").is_ok() {
                ShellType::PowerShell
            } else {
                ShellType::Cmd
            }
        } else {
            match std::env::var("SHELL").as_deref() {
                Ok(s) if s.contains("zsh") => ShellType::Zsh,
                Ok(s) if s.contains("fish") => ShellType::Fish,
                Ok(s) if s.contains("bash") => ShellType::Bash,
                _ => ShellType::Bash,
            }
        };

        let config_path = Self::get_config_path(&shell);

        Self {
            shell,
            config_path,
            integration_installed: false,
        }
    }

    fn get_config_path(shell: &ShellType) -> String {
        match shell {
            ShellType::Bash => dirs::home_dir()
                .map(|p| p.join(".bashrc").to_string_lossy().to_string())
                .unwrap_or_else(|| "~/.bashrc".to_string()),
            ShellType::Zsh => dirs::home_dir()
                .map(|p| p.join(".zshrc").to_string_lossy().to_string())
                .unwrap_or_else(|| "~/.zshrc".to_string()),
            ShellType::Fish => dirs::home_dir()
                .map(|p| {
                    p.join(".config")
                        .join("fish")
                        .join("config.fish")
                        .to_string_lossy()
                        .to_string()
                })
                .unwrap_or_else(|| "~/.config/fish/config.fish".to_string()),
            ShellType::PowerShell => "$PROFILE".to_string(),
            ShellType::Cmd => String::new(),
        }
    }

    pub fn install_integration(&mut self) -> Result<(), String> {
        let integration_code = match self.shell {
            ShellType::Bash | ShellType::Zsh => {
                r#"
# StarCode CLI integration
starcode() {
    command starcode "$@"
}
"#
            }
            ShellType::Fish => {
                r#"
# StarCode CLI integration
function starcode
    command starcode $argv
end
"#
            }
            ShellType::PowerShell => {
                r#"
# StarCode CLI integration
function starcode { & starcode.exe @args }
"#
            }
            ShellType::Cmd => return Ok(()),
        };

        if self.config_path.is_empty() {
            return Ok(());
        }

        let path = PathBuf::from(&self.config_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing.contains("# StarCode CLI integration") {
            self.integration_installed = true;
            return Ok(());
        }

        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("Failed to open config file: {}", e))?;

        file.write_all(integration_code.as_bytes())
            .map_err(|e| format!("Failed to write integration: {}", e))?;

        self.integration_installed = true;
        Ok(())
    }

    pub fn uninstall_integration(&mut self) -> Result<(), String> {
        if self.config_path.is_empty() {
            return Ok(());
        }

        let path = PathBuf::from(&self.config_path);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();

        if !existing.contains("# StarCode CLI integration") {
            self.integration_installed = false;
            return Ok(());
        }

        let lines: Vec<&str> = existing.lines().collect();
        let mut new_content = String::new();
        let mut in_integration = false;

        for line in &lines {
            if line.contains("# StarCode CLI integration") {
                in_integration = true;
                continue;
            }

            if in_integration {
                if line.trim().is_empty() && !new_content.ends_with("\n\n") {
                    in_integration = false;
                }
                continue;
            }

            new_content.push_str(line);
            new_content.push('\n');
        }

        std::fs::write(&path, new_content)
            .map_err(|e| format!("Failed to update config file: {}", e))?;

        self.integration_installed = false;
        Ok(())
    }

    pub fn get_setup_instructions(&self) -> String {
        match self.shell {
            ShellType::Bash => {
                format!(
                    "Add to {}:\n  eval \"$(starcode init bash)\"",
                    self.config_path
                )
            }
            ShellType::Zsh => {
                format!(
                    "Add to {}:\n  eval \"$(starcode init zsh)\"",
                    self.config_path
                )
            }
            ShellType::Fish => {
                format!(
                    "Add to {}:\n  starcode init fish | source",
                    self.config_path
                )
            }
            ShellType::PowerShell => {
                "Add to $PROFILE:\n  Invoke-Expression (starcode init powershell)".to_string()
            }
            ShellType::Cmd => "CMD is not supported. Use PowerShell instead.".to_string(),
        }
    }

    pub fn get_shell_name(&self) -> &str {
        match self.shell {
            ShellType::Bash => "bash",
            ShellType::Zsh => "zsh",
            ShellType::Fish => "fish",
            ShellType::PowerShell => "powershell",
            ShellType::Cmd => "cmd",
        }
    }

    pub fn get_init_script(&self) -> String {
        match self.shell {
            ShellType::Bash | ShellType::Zsh => {
                r#"# StarCode CLI init script
_starcode_init() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    COMPREPLY=($(compgen -W "help clear exit about status chat resume restore mcp memory provider stats cost review terminal-setup vim login logout" -- "$cur"))
}
complete -F _starcode_init starcode
"#
                .to_string()
            }
            ShellType::Fish => {
                r#"# StarCode CLI init script
complete -c starcode -f
complete -c starcode -n '__fish_use_subcommand' -a 'help' -d 'Show help'
complete -c starcode -n '__fish_use_subcommand' -a 'clear' -d 'Clear screen'
complete -c starcode -n '__fish_use_subcommand' -a 'exit' -d 'Exit'
complete -c starcode -n '__fish_use_subcommand' -a 'about' -d 'About'
complete -c starcode -n '__fish_use_subcommand' -a 'status' -d 'Status'
complete -c starcode -n '__fish_use_subcommand' -a 'chat' -d 'Start chat'
complete -c starcode -n '__fish_use_subcommand' -a 'mcp' -d 'MCP commands'
complete -c starcode -n '__fish_use_subcommand' -a 'memory' -d 'Memory commands'
complete -c starcode -n '__fish_use_subcommand' -a 'config' -d 'Configuration'
"#
                .to_string()
            }
            ShellType::PowerShell => {
                r#"# StarCode CLI init script
Register-ArgumentCompleter -CommandName starcode -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $commands = @('help', 'clear', 'exit', 'about', 'status', 'chat', 'resume', 'restore', 'mcp', 'memory', 'provider', 'stats', 'cost', 'review', 'terminal-setup', 'vim', 'login', 'logout', 'config', 'model', 'tools')
    $commands | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object { [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }
}
"#
                .to_string()
            }
            ShellType::Cmd => String::new(),
        }
    }
}
