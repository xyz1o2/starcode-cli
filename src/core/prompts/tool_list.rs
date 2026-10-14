use crate::core::prompts::loader;
use crate::core::prompts::routing_matrix;

/// 目录表与 Workflow 段的占位符。
///
/// 这两段以前是手写在这个文件里的，和 `tool-description-*.md` 各说各话，最后
/// 模型被告知了两套互相矛盾的"何时用哪个工具"。现在它们由
/// [`routing_matrix::ROUTING_MATRIX`] 生成，本文件只保留散文。
pub const ROUTING_MATRIX_PLACEHOLDER: &str = "{{ROUTING_MATRIX}}";
pub const TOOL_WORKFLOW_PLACEHOLDER: &str = "{{TOOL_WORKFLOW}}";

pub fn render(_is_thinking_model: bool) -> String {
    // Load tool guidance from file (external dir overrides embedded)
    let raw = loader::load_prompt("system-prompt-tool-guidance.md");

    raw.replace(
        ROUTING_MATRIX_PLACEHOLDER,
        &routing_matrix::render_matrix_markdown(),
    )
    .replace(
        TOOL_WORKFLOW_PLACEHOLDER,
        &routing_matrix::render_workflow_markdown(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 渲染后不得残留占位符。
    ///
    /// 外部目录可以覆盖 `system-prompt-tool-guidance.md`；一旦覆盖版本里没有这两个
    /// 占位符，`render` 会**静默**什么都不生成——模型就看不到工具分工表了。
    /// 这里只守内嵌版本（外部覆盖的正确性无从校验，但至少失败方式要知道）。
    #[test]
    fn placeholders_are_substituted() {
        let out = render(false);
        assert!(
            !out.contains(ROUTING_MATRIX_PLACEHOLDER),
            "路由表占位符没被替换"
        );
        assert!(
            !out.contains(TOOL_WORKFLOW_PLACEHOLDER),
            "Workflow 占位符没被替换"
        );
    }

    /// 内嵌模板必须真的带这两个占位符（防止有人改 md 时把它们删了）。
    #[test]
    fn embedded_template_declares_both_placeholders() {
        let raw = loader::load_prompt("system-prompt-tool-guidance.md");
        assert!(raw.contains(ROUTING_MATRIX_PLACEHOLDER));
        assert!(raw.contains(TOOL_WORKFLOW_PLACEHOLDER));
    }

    /// 抽出渲染结果里由矩阵生成的那张表（`| Job | Tool | Never via bash |` 开头的那段）。
    ///
    /// 断言必须只盯着**这张表**而不是整段输出：表外的散文会合法地讨论历史
    /// （"它曾经被描述成 search by meaning"），用整段子串匹配会被自己的说明文字误伤。
    fn rendered_table(out: &str) -> String {
        let header = "| Job | Tool | Never via bash |";
        let start = out
            .find(header)
            .unwrap_or_else(|| panic!("表头缺失：{out}"));
        let rest = &out[start..];
        let end = rest[1..].find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// 回归：这张表曾经手写在 md 里，写的是 `| Search by meaning | CodebaseSearch |`，
    /// 而工具自己的 schema 描述说的是"按词法、不按语义"。矛盾之下模型永远选 Grep——
    /// 那正是 codebase_search 长期零触发的直接原因。
    #[test]
    fn rendered_table_no_longer_claims_semantic_search() {
        let table = rendered_table(&render(false));
        assert!(
            !table.to_lowercase().contains("semantic"),
            "表格仍在宣称'按语义搜索'，与工具实际的词法能力矛盾：\n{table}"
        );
        assert!(!table.contains("Search by meaning"), "\n{table}");

        // CodebaseSearch 必须被归到它真正的职责上，而不是一个空泛的标签。
        let slot = routing_matrix::ROUTING_MATRIX
            .iter()
            .find(|s| s.primary == "CodebaseSearch")
            .expect("矩阵里必须有 CodebaseSearch 的槽");
        assert!(
            table.contains(slot.job),
            "表格里没有 CodebaseSearch 的职责行：\n{table}"
        );
    }
}
