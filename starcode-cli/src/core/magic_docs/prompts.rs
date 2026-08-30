/// 文档提示词

/// 文档提示词
pub struct DocPrompts {
    /// README提示词
    readme_prompt: String,
    /// API文档提示词
    api_prompt: String,
    /// 架构文档提示词
    architecture_prompt: String,
}

impl DocPrompts {
    /// 创建新的文档提示词
    pub fn new() -> Self {
        Self {
            readme_prompt: Self::default_readme_prompt(),
            api_prompt: Self::default_api_prompt(),
            architecture_prompt: Self::default_architecture_prompt(),
        }
    }

    /// 默认README提示词
    fn default_readme_prompt() -> String {
        r#"Generate a comprehensive README.md for this project.

Include:
1. Project title and description
2. Installation instructions
3. Usage examples
4. Configuration options
5. Contributing guidelines
6. License information

Make it clear, concise, and helpful for new users."#.to_string()
    }

    /// 默认API文档提示词
    fn default_api_prompt() -> String {
        r#"Generate API documentation for this project.

Include:
1. API overview
2. Authentication
3. Endpoints/Functions
4. Request/Response formats
5. Error handling
6. Examples

Make it comprehensive and easy to understand."#.to_string()
    }

    /// 默认架构文档提示词
    fn default_architecture_prompt() -> String {
        r#"Generate architecture documentation for this project.

Include:
1. System overview
2. Component diagram
3. Data flow
4. Key design decisions
5. Technology stack
6. Deployment architecture

Make it clear and informative."#.to_string()
    }

    /// 获取README提示词
    pub fn readme_prompt(&self) -> &str {
        &self.readme_prompt
    }

    /// 获取API文档提示词
    pub fn api_prompt(&self) -> &str {
        &self.api_prompt
    }

    /// 获取架构文档提示词
    pub fn architecture_prompt(&self) -> &str {
        &self.architecture_prompt
    }
}
