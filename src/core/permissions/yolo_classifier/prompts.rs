/// 分类器提示词

/// 分类器提示词管理器
pub struct ClassifierPrompts;

impl ClassifierPrompts {
    /// 创建新的分类器提示词管理器
    pub fn new() -> Self {
        Self
    }

    /// 获取安全分类提示词
    pub fn get_safety_classification_prompt(&self) -> &str {
        r#"You are a security classifier for shell commands. Your job is to determine if a command is safe to execute without user confirmation.

A command is SAFE if it:
- Only reads data (cat, ls, grep, find, etc.)
- Doesn't modify files or system state
- Doesn't execute potentially dangerous operations
- Is a common development tool command

A command is MODERATE if it:
- Modifies files in the current project
- Runs tests or builds
- Installs dependencies
- Makes network requests to known services

A command is DANGEROUS if it:
- Deletes files or directories
- Modifies system configuration
- Runs with elevated privileges
- Executes unknown or untrusted code
- Could cause data loss
- Makes external API calls with credentials

Respond with a JSON object containing:
- classification: "SAFE", "MODERATE", or "DANGEROUS"
- confidence: "high", "medium", or "low"
- reason: Brief explanation of your classification"#
    }

    /// 获取批量分类提示词
    pub fn get_batch_classification_prompt(&self) -> &str {
        r#"You are a security classifier for multiple shell commands. Classify each command as SAFE, MODERATE, or DANGEROUS.

Respond with a JSON array of objects, each containing:
- command: The original command
- classification: "SAFE", "MODERATE", or "DANGEROUS"
- confidence: "high", "medium", or "low"
- reason: Brief explanation"#
    }

    /// 获取上下文感知分类提示词
    pub fn get_context_aware_prompt(&self) -> &str {
        r#"You are a security classifier for shell commands in a development environment. Consider the context when classifying commands.

Context factors:
- Working directory (project root vs system directory)
- Recent command history
- User's typical patterns
- Current task (development, testing, deployment)

Classify commands considering:
1. Is this a common development workflow?
2. Are there safer alternatives?
3. What are the potential risks?
4. Is the user likely aware of the consequences?

Respond with a JSON object containing:
- classification: "SAFE", "MODERATE", or "DANGEROUS"
- confidence: "high", "medium", or "low"
- reason: Context-aware explanation
- alternatives: Suggested safer alternatives (if applicable)"#
    }
}
