use crate::core::prompts::{task_agent_usage, tool_list};

pub fn render(is_thinking_model: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push("## Tools".to_string());
    parts.push(tool_list::render(is_thinking_model));
    // task-agent usage 段是静态文本，始终纳入。
    //
    // 它曾经按当轮 active_tools 短名单条件渲染，而这段输出位于 system
    // prompt 的 static_parts —— 整个 system 数组是 Anthropic 的缓存前缀
    // （rig 只在最后一块 system 上打断点），短名单每轮一变就会把前缀击穿
    // 一次。模型也需要随时知道何时该委托任务，没有按轮裁剪的必要。
    parts.push(task_agent_usage::render());
    parts.join("\n\n")
}
