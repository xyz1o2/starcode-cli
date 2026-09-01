/// 提示词常量

/// 系统提示词
pub const SYSTEM_PROMPT_DEFAULT: &str = "You are a helpful AI coding assistant.";
pub const SYSTEM_PROMPT_CODING: &str = "You are an expert software developer. Help the user with their coding tasks.";
pub const SYSTEM_PROMPT_ANALYSIS: &str = "You are a code analyst. Analyze the provided code and provide insights.";
pub const SYSTEM_PROMPT_REVIEW: &str = "You are a code reviewer. Review the code and provide feedback.";
pub const SYSTEM_PROMPT_DEBUG: &str = "You are a debugging expert. Help the user debug their code.";

/// 用户提示词模板
pub const PROMPT_TEMPLATE_CODE_EXPLAIN: &str = "Explain the following code:\n\n{code}";
pub const PROMPT_TEMPLATE_CODE_REVIEW: &str = "Review the following code:\n\n{code}";
pub const PROMPT_TEMPLATE_CODE_FIX: &str = "Fix the following code:\n\n{code}";
pub const PROMPT_TEMPLATE_CODE_OPTIMIZE: &str = "Optimize the following code:\n\n{code}";
pub const PROMPT_TEMPLATE_CODE_TEST: &str = "Write tests for the following code:\n\n{code}";
pub const PROMPT_TEMPLATE_CODE_DOC: &str = "Write documentation for the following code:\n\n{code}";
pub const PROMPT_TEMPLATE_CODE_REFACTOR: &str = "Refactor the following code:\n\n{code}";

/// 工具提示词
pub const TOOL_PROMPT_SEARCH: &str = "Search for relevant code or information.";
pub const TOOL_PROMPT_READ: &str = "Read the contents of a file.";
pub const TOOL_PROMPT_WRITE: &str = "Write content to a file.";
pub const TOOL_PROMPT_EDIT: &str = "Edit the contents of a file.";
pub const TOOL_PROMPT_RUN: &str = "Execute a command or script.";
pub const TOOL_PROMPT_ANALYZE: &str = "Analyze code or project structure.";

/// 错误提示词
pub const ERROR_PROMPT_RETRY: &str = "Would you like to try again?";
pub const ERROR_PROMPT_DIFFERENT_APPROACH: &str = "Would you like to try a different approach?";
pub const ERROR_PROMPT_MORE_INFO: &str = "Could you provide more information?";
pub const ERROR_PROMPT_CLARIFY: &str = "Could you clarify your request?";

/// 完成提示词
pub const DONE_PROMPT_ANYTHING_ELSE: &str = "Is there anything else I can help you with?";
pub const DONE_PROMPT_SATISFIED: &str = "Are you satisfied with the result?";
pub const DONE_PROMPT_FOLLOW_UP: &str = "Do you have any follow-up questions?";
