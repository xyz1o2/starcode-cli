/// Command system core module
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: &'static str,
    pub alt_names: &'static [&'static str],
    pub description: &'static str,
    pub category: &'static str,
    pub sub_commands: &'static [SlashCommand],
}

/// All available commands
pub const ALL_COMMANDS: &[SlashCommand] = &[
    // === 核心交互 (Core Interaction) ===
    SlashCommand {
        name: "chat",
        alt_names: &[],
        description: "Session management",
        category: "Session",
        sub_commands: &[
            SlashCommand {
                name: "save",
                alt_names: &[],
                description: "Save current session",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "resume",
                alt_names: &[],
                description: "Restore saved session",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "export",
                alt_names: &[],
                description: "Export conversation to markdown file or clipboard",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "rename",
                alt_names: &[],
                description: "Rename the current session",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "rewind",
                alt_names: &["checkpoint"],
                description: "List file-history snapshots, or revert: /rewind [latest|<id>]",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "diff",
                alt_names: &[],
                description: "Show uncommitted git changes",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "files",
                alt_names: &[],
                description: "List files read/edited this session",
                category: "Tools",
                sub_commands: &[],
            },
            SlashCommand {
                name: "model",
                alt_names: &[],
                description: "Switch model (alias of /models)",
                category: "Configuration",
                sub_commands: &[],
            },
            SlashCommand {
                name: "ext",
                alt_names: &["extension"],
                description: "Extension marketplace (list/install/search)",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "voice",
                alt_names: &[],
                description: "Voice settings (lang/rate/volume)",
                category: "Configuration",
                sub_commands: &[],
            },
            SlashCommand {
                name: "teleport",
                alt_names: &["tp"],
                description: "Connect to a remote session",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List available session tags",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "delete",
                alt_names: &[],
                description: "Delete session checkpoint",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "share",
                alt_names: &[],
                description: "Export session to file",
                category: "Session",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "share",
        alt_names: &[],
        description: "Export current session to a markdown file",
        category: "Session",
        sub_commands: &[],
    },
    SlashCommand {
        name: "clear",
        alt_names: &[],
        description: "Clear chat history",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "copy",
        alt_names: &[],
        description: "Copy last reply to clipboard",
        category: "Utility",
        sub_commands: &[],
    },
    SlashCommand {
        name: "help",
        alt_names: &["?"],
        description: "Show help information",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "exit",
        alt_names: &[],
        description: "Exit application",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "init",
        alt_names: &[],
        description: "Initialize project config",
        category: "Config",
        sub_commands: &[],
    },
    SlashCommand {
        name: "plan",
        alt_names: &[],
        description: "Enter plan mode",
        category: "Session",
        sub_commands: &[],
    },
    SlashCommand {
        name: "loop",
        alt_names: &[],
        description: "Scheduled task management",
        category: "Automation",
        sub_commands: &[
            SlashCommand {
                name: "add",
                alt_names: &[],
                description: "Create scheduled task",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List scheduled tasks",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "remove",
                alt_names: &[],
                description: "Delete scheduled task",
                category: "Automation",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "agents",
        alt_names: &[],
        description: "Subagent definition management",
        category: "Automation",
        sub_commands: &[
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List available agent definitions",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "create",
                alt_names: &[],
                description: "Create agent definition (frontmatter + prompt)",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "edit",
                alt_names: &[],
                description: "Edit existing agent definition",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "delete",
                alt_names: &[],
                description: "Delete agent definition (supports project/user)",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "add",
                alt_names: &[],
                description: "Import agent definition file to project",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "remove",
                alt_names: &[],
                description: "Delete project agent definition",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "team",
                alt_names: &[],
                description: "Agent Teams parallel orchestration",
                category: "Automation",
                sub_commands: &[
                    SlashCommand {
                        name: "list",
                        alt_names: &[],
                        description: "List team built-in members",
                        category: "Automation",
                        sub_commands: &[],
                    },
                    SlashCommand {
                        name: "run",
                        alt_names: &[],
                        description: "Execute multi-agent parallel task",
                        category: "Automation",
                        sub_commands: &[],
                    },
                    SlashCommand {
                        name: "save",
                        alt_names: &[],
                        description: "Save team preset",
                        category: "Automation",
                        sub_commands: &[],
                    },
                    SlashCommand {
                        name: "show",
                        alt_names: &[],
                        description: "Show team preset details",
                        category: "Automation",
                        sub_commands: &[],
                    },
                    SlashCommand {
                        name: "remove",
                        alt_names: &[],
                        description: "Delete team preset",
                        category: "Automation",
                        sub_commands: &[],
                    },
                    SlashCommand {
                        name: "apply",
                        alt_names: &[],
                        description: "Apply team run artifacts",
                        category: "Automation",
                        sub_commands: &[],
                    },
                    SlashCommand {
                        name: "runs",
                        alt_names: &[],
                        description: "View team run history",
                        category: "Automation",
                        sub_commands: &[],
                    },
                    SlashCommand {
                        name: "show-run",
                        alt_names: &[],
                        description: "Show single team run details",
                        category: "Automation",
                        sub_commands: &[],
                    },
                    SlashCommand {
                        name: "clean",
                        alt_names: &[],
                        description: "Clean team run artifacts and worktree",
                        category: "Automation",
                        sub_commands: &[],
                    },
                ],
            },
        ],
    },
    SlashCommand {
        name: "plugin",
        alt_names: &[],
        description: "Plugin management (no args: open marketplace UI)",
        category: "Automation",
        sub_commands: &[
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List installed plugins",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "install",
                alt_names: &[],
                description: "Install plugin from local or Git source",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "remove",
                alt_names: &[],
                description: "Remove plugin",
                category: "Automation",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "skills",
        alt_names: &[],
        description: "Skill management (custom SKILL.md + built-in sub-agents)",
        category: "Automation",
        sub_commands: &[
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List all custom Skills",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "agents",
                alt_names: &[],
                description: "List built-in and custom sub-agents",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "show",
                alt_names: &[],
                description: "Show Skill content",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "new",
                alt_names: &[],
                description: "Create new custom Skill",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "delete",
                alt_names: &[],
                description: "Delete custom Skill",
                category: "Automation",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "remote",
        alt_names: &[],
        description: "Remote control protocol and inbox",
        category: "Automation",
        sub_commands: &[
            SlashCommand {
                name: "status",
                alt_names: &[],
                description: "View remote inbox status",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "send",
                alt_names: &[],
                description: "Write a remote request (debug)",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "protocol",
                alt_names: &[],
                description: "Show remote protocol format",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "drain",
                alt_names: &[],
                description: "Manually consume remote inbox",
                category: "Automation",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "connect",
        alt_names: &[],
        description: "Connect MCP service",
        category: "MCP",
        sub_commands: &[],
    },
    // === Context and Memory ===
    SlashCommand {
        name: "provider",
        alt_names: &[],
        description: "Manage model providers",
        category: "Config",
        sub_commands: &[
            SlashCommand {
                name: "select",
                alt_names: &[],
                description: "Select provider (optional API Key)",
                category: "Config",
                sub_commands: &[],
            },
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List available providers",
                category: "Config",
                sub_commands: &[],
            },
            SlashCommand {
                name: "set-key",
                alt_names: &[],
                description: "Set provider API Key",
                category: "Config",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "memory",
        alt_names: &["remember"],
        description: "Manage project memory",
        category: "Memory",
        sub_commands: &[
            SlashCommand {
                name: "show",
                alt_names: &[],
                description: "Show current memory",
                category: "Memory",
                sub_commands: &[],
            },
            SlashCommand {
                name: "add",
                alt_names: &[],
                description: "Add memory",
                category: "Memory",
                sub_commands: &[],
            },
            SlashCommand {
                name: "refresh",
                alt_names: &[],
                description: "Refresh memory",
                category: "Memory",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "compress",
        alt_names: &["compact"],
        description: "Compress conversation context",
        category: "Utility",
        sub_commands: &[],
    },
    SlashCommand {
        name: "resume",
        alt_names: &[],
        description: "Browse and restore previous sessions",
        category: "Session",
        sub_commands: &[],
    },
    SlashCommand {
        name: "restore",
        alt_names: &[],
        description: "Revert to the most recent file-history snapshot (= /undo)",
        category: "Session",
        sub_commands: &[],
    },
    SlashCommand {
        name: "undo",
        alt_names: &[],
        description: "Revert files to the most recent file-history snapshot",
        category: "Session",
        sub_commands: &[],
    },
    // === Tools and Status ===
    SlashCommand {
        name: "tools",
        alt_names: &[],
        description: "List available tools",
        category: "Tools",
        sub_commands: &[
            SlashCommand {
                name: "desc",
                alt_names: &[],
                description: "Show tool detailed description",
                category: "Tools",
                sub_commands: &[],
            },
            SlashCommand {
                name: "nodesc",
                alt_names: &[],
                description: "Hide tool description",
                category: "Tools",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "stats",
        alt_names: &[],
        description: "Show session statistics",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "cost",
        alt_names: &["tokens", "usage"],
        description: "Show token usage and cost info",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "review",
        alt_names: &[],
        description: "Initiate code review task (read-only)",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "code-review",
        alt_names: &[],
        description: "Strict code review for correctness, bugs, and security",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "security-review",
        alt_names: &[],
        description: "Security audit for the current changes",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "simplify",
        alt_names: &[],
        description: "Code simplification and efficiency review",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "run",
        alt_names: &[],
        description: "Launch and drive the project app",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "lint",
        alt_names: &[],
        description: "Run linting/static analysis",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "tasks",
        alt_names: &["todos"],
        description: "Show and manage project tasks",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "workflows",
        alt_names: &[],
        description: "List project workflow definitions",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "context",
        alt_names: &[],
        description: "Show context stats (messages, tokens)",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "pr_comments",
        alt_names: &["pr-comments"],
        description: "Pull GitHub PR comments (requires gh CLI)",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "prs",
        alt_names: &[],
        description: "List open GitHub pull requests (requires gh CLI)",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "bug",
        alt_names: &[],
        description: "Generate bug report template and diagnostic context",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "status",
        alt_names: &[],
        description: "Show application status",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "terminal-setup",
        alt_names: &[],
        description: "Show terminal environment setup suggestions",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "vim",
        alt_names: &[],
        description: "Vim mode entry (compat)",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "sandbox",
        alt_names: &[],
        description: "Sandbox mode management",
        category: "Security",
        sub_commands: &[
            SlashCommand {
                name: "on",
                alt_names: &[],
                description: "Enable sandbox isolation",
                category: "Security",
                sub_commands: &[],
            },
            SlashCommand {
                name: "off",
                alt_names: &[],
                description: "Disable sandbox isolation",
                category: "Security",
                sub_commands: &[],
            },
            SlashCommand {
                name: "status",
                alt_names: &[],
                description: "Show sandbox status",
                category: "Security",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "permissions",
        alt_names: &[],
        description: "View or switch approval mode",
        category: "Security",
        sub_commands: &[],
    },
    SlashCommand {
        name: "hooks",
        alt_names: &[],
        description: "Hook management and testing",
        category: "Security",
        sub_commands: &[
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List configured hooks",
                category: "Security",
                sub_commands: &[],
            },
            SlashCommand {
                name: "add",
                alt_names: &[],
                description: "Add hook (supports blocking)",
                category: "Security",
                sub_commands: &[],
            },
            SlashCommand {
                name: "remove",
                alt_names: &[],
                description: "Remove hook",
                category: "Security",
                sub_commands: &[],
            },
            SlashCommand {
                name: "run",
                alt_names: &[],
                description: "Manually execute event hooks",
                category: "Security",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "mcp",
        alt_names: &[],
        description: "MCP server management",
        category: "MCP",
        sub_commands: &[
            SlashCommand {
                name: "add",
                alt_names: &[],
                description: "Add MCP server (stdio/http/sse)",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "install",
                alt_names: &[],
                description: "Install new MCP server (npm package)",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "remove",
                alt_names: &[],
                description: "Remove MCP server",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List MCP servers",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "status",
                alt_names: &[],
                description: "Check MCP connection status and tool count",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "tools",
                alt_names: &[],
                description: "List MCP server tools (can specify server)",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "desc",
                alt_names: &[],
                description: "List MCP server and tool descriptions",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "schema",
                alt_names: &[],
                description: "List MCP server and tool schemas",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "refresh",
                alt_names: &[],
                description: "Restart all MCP servers",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "import",
                alt_names: &[],
                description: "Import MCP servers from config file",
                category: "MCP",
                sub_commands: &[],
            },
            SlashCommand {
                name: "export",
                alt_names: &[],
                description: "Export MCP servers to config file",
                category: "MCP",
                sub_commands: &[],
            },
        ],
    },
    // === Configuration ===
    SlashCommand {
        name: "models",
        alt_names: &[],
        description: "Switch model",
        category: "Configuration",
        sub_commands: &[],
    },
    SlashCommand {
        name: "settings",
        alt_names: &["config"],
        description: "Open settings editor",
        category: "Configuration",
        sub_commands: &[],
    },
    SlashCommand {
        name: "lang",
        alt_names: &[],
        description: "View or switch UI language (auto / en / zh-CN)",
        category: "Configuration",
        sub_commands: &[
            SlashCommand {
                name: "auto",
                alt_names: &[],
                description: "Follow system language",
                category: "Configuration",
                sub_commands: &[],
            },
            SlashCommand {
                name: "en",
                alt_names: &[],
                description: "Switch to English",
                category: "Configuration",
                sub_commands: &[],
            },
            SlashCommand {
                name: "zh-CN",
                alt_names: &[],
                description: "Switch to Simplified Chinese",
                category: "Configuration",
                sub_commands: &[],
            },
        ],
    },
    SlashCommand {
        name: "theme",
        alt_names: &[],
        description: "Switch UI theme (reserved)",
        category: "Configuration",
        sub_commands: &[],
    },
    SlashCommand {
        name: "login",
        alt_names: &[],
        description: "Write API login config (api key/base url/model)",
        category: "Configuration",
        sub_commands: &[],
    },
    SlashCommand {
        name: "logout",
        alt_names: &[],
        description: "Clear login info (default: only API key)",
        category: "Configuration",
        sub_commands: &[],
    },
    // === Misc ===
    SlashCommand {
        name: "about",
        alt_names: &["version"],
        description: "Show version info",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "commit-and-push",
        alt_names: &[],
        description: "AI commit and push to remote",
        category: "Git",
        sub_commands: &[],
    },
    SlashCommand {
        name: "eval",
        alt_names: &[],
        description: "Run agent evaluation suite",
        category: "Automation",
        sub_commands: &[],
    },
    // === Debug ===
    SlashCommand {
        name: "test",
        alt_names: &[],
        description: "运行内置功能测试",
        category: "Debug",
        sub_commands: &[],
    },
    SlashCommand {
        name: "doctor",
        alt_names: &[],
        description: "诊断环境与配置问题",
        category: "Debug",
        sub_commands: &[],
    },
    // === New compat commands ===
    SlashCommand {
        name: "bashes",
        alt_names: &[],
        description: "Show recent bash history",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "feedback",
        alt_names: &[],
        description: "Feedback template with version info",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "upgrade",
        alt_names: &[],
        description: "Check for version upgrades",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "ide",
        alt_names: &[],
        description: "IDE integration information",
        category: "General",
        sub_commands: &[],
    },
    SlashCommand {
        name: "forget",
        alt_names: &[],
        description: "Remove memory entries by keyword",
        category: "Memory",
        sub_commands: &[],
    },
    SlashCommand {
        name: "todos",
        alt_names: &[],
        description: "Show and manage project tasks (alias for tasks)",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "index",
        alt_names: &[],
        description: "Show index status or trigger reindex",
        category: "Tools",
        sub_commands: &[
            SlashCommand {
                name: "status",
                alt_names: &[],
                description: "Show current index status",
                category: "Tools",
                sub_commands: &[],
            },
            SlashCommand {
                name: "rebuild",
                alt_names: &[],
                description: "Trigger full reindex",
                category: "Tools",
                sub_commands: &[],
            },
        ],
    },
    // === Git Subcommands ===
    SlashCommand {
        name: "git",
        alt_names: &[],
        description: "Git operations",
        category: "Git",
        sub_commands: &[
            SlashCommand {
                name: "status",
                alt_names: &[],
                description: "Show git status",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "log",
                alt_names: &[],
                description: "Show git log",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "diff",
                alt_names: &[],
                description: "Show git diff",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "branch",
                alt_names: &[],
                description: "Branch management (list/create/delete/switch)",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "merge",
                alt_names: &[],
                description: "Merge branch",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "rebase",
                alt_names: &[],
                description: "Rebase branch",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "stash",
                alt_names: &[],
                description: "Stash management (list/save/pop/apply/drop/clear)",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "tag",
                alt_names: &[],
                description: "Tag management (list/create/delete)",
                category: "Git",
                sub_commands: &[],
            },
            SlashCommand {
                name: "blame",
                alt_names: &[],
                description: "File blame",
                category: "Git",
                sub_commands: &[],
            },
        ],
    },
    // === Config Subcommands ===
    SlashCommand {
        name: "config",
        alt_names: &[],
        description: "Configuration management",
        category: "Config",
        sub_commands: &[
            SlashCommand {
                name: "show",
                alt_names: &[],
                description: "Show current config",
                category: "Config",
                sub_commands: &[],
            },
            SlashCommand {
                name: "set",
                alt_names: &[],
                description: "Set config value (key value)",
                category: "Config",
                sub_commands: &[],
            },
            SlashCommand {
                name: "reset",
                alt_names: &[],
                description: "Reset to defaults",
                category: "Config",
                sub_commands: &[],
            },
            SlashCommand {
                name: "export",
                alt_names: &[],
                description: "Export config",
                category: "Config",
                sub_commands: &[],
            },
            SlashCommand {
                name: "import",
                alt_names: &[],
                description: "Import config from file",
                category: "Config",
                sub_commands: &[],
            },
        ],
    },
    // === Session Subcommands ===
    SlashCommand {
        name: "session",
        alt_names: &[],
        description: "Session management",
        category: "Session",
        sub_commands: &[
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List sessions",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "resume",
                alt_names: &[],
                description: "Resume session by ID",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "delete",
                alt_names: &[],
                description: "Delete session by ID",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "export",
                alt_names: &[],
                description: "Export session to file",
                category: "Session",
                sub_commands: &[],
            },
            SlashCommand {
                name: "title",
                alt_names: &[],
                description: "Set session title",
                category: "Session",
                sub_commands: &[],
            },
        ],
    },
    // === Debug Subcommands ===
    SlashCommand {
        name: "debug",
        alt_names: &[],
        description: "Debug and diagnostics",
        category: "Debug",
        sub_commands: &[
            SlashCommand {
                name: "log",
                alt_names: &[],
                description: "Toggle debug logging",
                category: "Debug",
                sub_commands: &[],
            },
            SlashCommand {
                name: "tokens",
                alt_names: &[],
                description: "Show token usage details",
                category: "Debug",
                sub_commands: &[],
            },
            SlashCommand {
                name: "tools",
                alt_names: &[],
                description: "Show tool call history",
                category: "Debug",
                sub_commands: &[],
            },
            SlashCommand {
                name: "state",
                alt_names: &[],
                description: "Show internal state",
                category: "Debug",
                sub_commands: &[],
            },
            SlashCommand {
                name: "perf",
                alt_names: &[],
                description: "Show performance metrics",
                category: "Debug",
                sub_commands: &[],
            },
        ],
    },
    // === Agent Subcommands ===
    SlashCommand {
        name: "agent",
        alt_names: &[],
        description: "Agent management",
        category: "Automation",
        sub_commands: &[
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List available agents",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "switch",
                alt_names: &[],
                description: "Switch to named agent",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "create",
                alt_names: &[],
                description: "Create new agent",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "delete",
                alt_names: &[],
                description: "Delete agent",
                category: "Automation",
                sub_commands: &[],
            },
        ],
    },
    // === Workflow Subcommands ===
    SlashCommand {
        name: "workflow",
        alt_names: &[],
        description: "Workflow management",
        category: "Automation",
        sub_commands: &[
            SlashCommand {
                name: "list",
                alt_names: &[],
                description: "List workflows",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "run",
                alt_names: &[],
                description: "Run workflow by name",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "create",
                alt_names: &[],
                description: "Create new workflow",
                category: "Automation",
                sub_commands: &[],
            },
            SlashCommand {
                name: "edit",
                alt_names: &[],
                description: "Edit existing workflow",
                category: "Automation",
                sub_commands: &[],
            },
        ],
    },
    // === Extended Utility Commands ===
    SlashCommand {
        name: "paste",
        alt_names: &[],
        description: "Paste from clipboard",
        category: "Utility",
        sub_commands: &[],
    },
    SlashCommand {
        name: "clear-screen",
        alt_names: &[],
        description: "Clear screen",
        category: "Utility",
        sub_commands: &[],
    },
    SlashCommand {
        name: "compact",
        alt_names: &[],
        description: "Force context compression",
        category: "Utility",
        sub_commands: &[],
    },
    SlashCommand {
        name: "cost-breakdown",
        alt_names: &[],
        description: "Show cost breakdown",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "model-info",
        alt_names: &[],
        description: "Show/switch model info",
        category: "Config",
        sub_commands: &[],
    },
    SlashCommand {
        name: "provider-info",
        alt_names: &[],
        description: "Show/switch provider info",
        category: "Config",
        sub_commands: &[],
    },
    SlashCommand {
        name: "temperature",
        alt_names: &[],
        description: "Show/set temperature",
        category: "Config",
        sub_commands: &[],
    },
    SlashCommand {
        name: "token-count",
        alt_names: &[],
        description: "Show token count",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "undo-edit",
        alt_names: &[],
        description: "Undo last edit",
        category: "Session",
        sub_commands: &[],
    },
    SlashCommand {
        name: "redo-edit",
        alt_names: &[],
        description: "Redo last edit",
        category: "Session",
        sub_commands: &[],
    },
    SlashCommand {
        name: "pending-diff",
        alt_names: &[],
        description: "Show pending changes",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "code-review-ext",
        alt_names: &[],
        description: "Review code changes",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "run-tests",
        alt_names: &[],
        description: "Run tests",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "run-lint",
        alt_names: &[],
        description: "Run linter",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "run-format",
        alt_names: &[],
        description: "Run formatter",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "gen-docs",
        alt_names: &[],
        description: "Generate documentation",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "explain-code",
        alt_names: &[],
        description: "Explain code",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "suggest-refactor",
        alt_names: &[],
        description: "Suggest refactoring",
        category: "Tools",
        sub_commands: &[],
    },
    SlashCommand {
        name: "suggest-optimize",
        alt_names: &[],
        description: "Suggest optimizations",
        category: "Tools",
        sub_commands: &[],
    },
];

/// 命令解析结果
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub path: Vec<String>,
    pub args: String,
}

/// 解析命令路径
/// 输入: "/chat save my-tag"
/// 输出: ParsedCommand { command: chat.save, path: ["chat", "save"], args: "my-tag" }
pub fn parse_command(input: &str) -> Option<ParsedCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let without_slash = &trimmed[1..];
    let parts: Vec<&str> = without_slash.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let mut current_commands = ALL_COMMANDS;
    let mut found_command: Option<&'static SlashCommand> = None;
    let mut path = Vec::new();
    let mut consumed_parts = 0;

    for (idx, part) in parts.iter().enumerate() {
        // 先尝试主名称匹配
        let mut matched = current_commands.iter().find(|cmd| cmd.name == *part);

        // 如果主名称没匹配，尝试别名
        if matched.is_none() {
            matched = current_commands
                .iter()
                .find(|cmd| cmd.alt_names.contains(part));
        }

        if let Some(cmd) = matched {
            found_command = Some(cmd);
            path.push(cmd.name.to_string());
            consumed_parts = idx + 1;

            if cmd.sub_commands.is_empty() {
                break;
            }
            current_commands = cmd.sub_commands;
        } else {
            break;
        }
    }

    found_command.map(|cmd| {
        let args = parts
            .get(consumed_parts..)
            .map(|s| s.join(" "))
            .unwrap_or_default();

        ParsedCommand { path, args }
    })
}

fn format_command_hint(prefix: &str, cmd: &SlashCommand) -> String {
    format!("{}{} - {}", prefix, cmd.name, cmd.description)
}

/// 获取命令补全提示
pub fn get_command_hints(input: &str) -> Vec<String> {
    if !input.starts_with('/') {
        return Vec::new();
    }

    let without_slash = &input[1..];
    let parts: Vec<&str> = without_slash.split_whitespace().collect();

    if parts.is_empty() {
        // 显示所有顶级命令
        return ALL_COMMANDS
            .iter()
            .map(|cmd| format_command_hint("/", cmd))
            .collect();
    }

    // 解析已输入的命令路径
    let mut current_commands = ALL_COMMANDS;
    let mut consumed = 0;

    for (idx, part) in parts.iter().enumerate() {
        let matched = current_commands.iter().find(|cmd| cmd.name == *part);

        if let Some(cmd) = matched {
            consumed = idx + 1;
            if !cmd.sub_commands.is_empty() {
                current_commands = cmd.sub_commands;
            } else {
                // Leaf command: no sub-commands exist.
                // Whether there is more text after it (arguments) or it is an
                // exact match, there are no further completions to offer.
                return Vec::new();
            }
        } else {
            break;
        }
    }

    let partial = parts.get(consumed).unwrap_or(&"");
    let partial_lower = partial.to_lowercase();

    // 加权 fuzzy 匹配排序（对标 Claude Code fuse.js 权重）：
    // 精确名 1000 > 前缀 800 > 名称分段 600 > alias 500/450 > 子序列 300 > 描述词 100
    let mut scored: Vec<(i32, &SlashCommand)> = current_commands
        .iter()
        .filter_map(|cmd| fuzzy_score(cmd, &partial_lower).map(|s| (s, cmd)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    let filtered: Vec<String> = scored
        .into_iter()
        .map(|(_, cmd)| {
            let prefix = if consumed > 0 {
                format!("/{} ", parts[..consumed].join(" "))
            } else {
                "/".to_string()
            };
            format_command_hint(&prefix, cmd)
        })
        .collect();

    if filtered.is_empty() {
        // 显示当前级别所有命令
        current_commands
            .iter()
            .map(|cmd| {
                let prefix = if consumed > 0 {
                    format!("/{} ", parts[..consumed].join(" "))
                } else {
                    "/".to_string()
                };
                format_command_hint(&prefix, cmd)
            })
            .collect()
    } else {
        filtered
    }
}

/// 加权 fuzzy 评分；None 表示不匹配。权重参考 Claude Code 的 fuse.js 配置
/// （commandName 3 > name-parts/aliases 2 > description words 0.5）。
pub(crate) fn fuzzy_score(cmd: &SlashCommand, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let name = cmd.name.to_lowercase();
    let q = query.to_lowercase();

    if name == q {
        return Some(1000);
    }
    if name.starts_with(&q) {
        return Some(800 - (name.len().saturating_sub(q.len())) as i32);
    }
    // 名称分段：按 - _ / 空格及 camelCase 边界拆分
    let name_parts: Vec<String> = split_name_parts(&name);
    if name_parts.iter().any(|p| p.starts_with(&q)) {
        return Some(600);
    }
    // alias 精确 > alias 前缀
    for a in cmd.alt_names {
        let a = a.to_lowercase();
        if a == q {
            return Some(500);
        }
        if a.starts_with(&q) {
            return Some(450);
        }
    }
    // 名称子序列匹配（越紧凑分越高）
    if let Some(gaps) = subsequence_gaps(&q, &name) {
        return Some(300 - (gaps as i32).min(200));
    }
    // 描述词前缀
    if cmd
        .description
        .to_lowercase()
        .split_whitespace()
        .any(|w| w.starts_with(&q))
    {
        return Some(100);
    }
    None
}

/// 按 - _ / 空格及 camelCase 边界拆分命令名
fn split_name_parts(name: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch == '-' || ch == '_' || ch == '/' || ch == ' ' {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            prev_lower = false;
        } else if ch.is_uppercase() && prev_lower {
            // camelCase 边界
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            current.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else {
            current.push(ch.to_ascii_lowercase());
            prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// 子序列匹配：q 是否为 name 的子序列，返回 Some(跳过的字符数) 或 None
fn subsequence_gaps(q: &str, name: &str) -> Option<usize> {
    let mut qi = q.chars().peekable();
    let mut gaps = 0usize;
    for ch in name.chars() {
        if qi.peek() == Some(&ch) {
            qi.next();
        } else if qi.peek().is_some() {
            gaps += 1;
        }
    }
    if qi.peek().is_none() {
        Some(gaps)
    } else {
        None
    }
}

/// 获取命令分类视图
pub fn get_categorized_commands() -> HashMap<&'static str, Vec<&'static SlashCommand>> {
    let mut categorized: HashMap<&'static str, Vec<&'static SlashCommand>> = HashMap::new();

    for cmd in ALL_COMMANDS {
        categorized.entry(cmd.category).or_default().push(cmd);
    }

    categorized
}

/// 格式化帮助信息
pub fn format_help() -> String {
    let mut help_text = " Starcode CLI - 可用命令\n\n".to_string();

    let categorized = get_categorized_commands();
    let categories = [
        "General",
        "Configuration",
        "Config",
        "Tools",
        "Session",
        "MCP",
        "Git",
        "Security",
        "Automation",
        "Utility",
        "Memory",
        "Debug",
    ];

    for category in &categories {
        if let Some(cmds) = categorized.get(category) {
            help_text.push_str(&format!("{}:\n", category));
            for cmd in cmds {
                let aliases = if !cmd.alt_names.is_empty() {
                    format!(" (别名: {})", cmd.alt_names.join(", "))
                } else {
                    String::new()
                };
                help_text.push_str(&format!(
                    "  /{}{} - {}\n",
                    cmd.name, aliases, cmd.description
                ));

                // 显示子命令
                if !cmd.sub_commands.is_empty() {
                    for sub in cmd.sub_commands {
                        help_text.push_str(&format!(
                            "    /{} {} - {}\n",
                            cmd.name, sub.name, sub.description
                        ));
                    }
                }
            }
            help_text.push('\n');
        }
    }

    help_text.push_str("提示: 输入命令后使用 --help 获取更多信息\n");
    help_text.push_str("使用 Ctrl+C 退出应用\n");

    help_text
}
