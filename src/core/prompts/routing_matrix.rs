//! 工具选择的**单一决策表**。
//!
//! # 为什么需要这个模块
//!
//! 以前"某个活儿该用哪个工具"这句话被手抄在四个地方：
//!
//! - `system-prompt-tool-guidance.md` 的表格 + Workflow 段
//! - `tool-description-codebase_search.md` 自己的正文
//! - `tool-description-code_graph.md` 的 `NOT for` 段（反过来把活儿推给 CodebaseSearch）
//! - `tool-description-projectmap.md` 的 `NOT for` 段（同上）
//!
//! 四份没有任何校验，于是它们各自漂移。最致命的一次漂移：目录表写着
//! 「Search **by meaning** | CodebaseSearch」，而 CodebaseSearch 自己的描述写着
//! 「Lexical, **NOT** meaning-based —— exact strings use Grep」。模型在同一段 system
//! prompt 里同时被告知两件相反的事，裁决权归**离调用点最近**的那份（工具 schema），
//! 于是它选 Grep。工具就这么永远不触发。
//!
//! 现在这张表是**代码里的唯一真相**：
//! - `tool_list::render` 从这里渲染出模型看到的那张表（md 里的手写表格已删除）；
//! - 每个 `tool-description-*.md` 在 frontmatter 里用 `routing: <slot.id>` 声明
//!   **它主张的职责槽**，测试强制"一个槽有且只有一个主张者"。
//!
//! 于是"两个工具抢同一个活儿"和"某个活儿没人认领"都会直接让测试变红，
//! 不再依赖人眼去比对四份文案。

/// 一个职责槽：某类活儿唯一的主管工具。
///
/// `id` 同时是：
/// 1. 描述文件 frontmatter 里 `routing:` 字段的取值；
/// 2. 文案一致性测试的比对键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteSlot {
    /// 稳定标识，出现在 `tool-description-*.md` 的 frontmatter 里。
    pub id: &'static str,
    /// 这一槽负责的活儿（渲染进模型看到的表格，也是判分依据）。
    pub job: &'static str,
    /// 唯一的主管工具（用注册名）。
    pub primary: &'static str,
    /// "绝不要绕道 bash 做这件事"的提醒；没有就写 `—`。
    pub never_via_bash: &'static str,
}

/// 全部职责槽。顺序即渲染顺序，按"读 → 找 → 改 → 验"组织。
pub const ROUTING_MATRIX: &[RouteSlot] = &[
    RouteSlot {
        id: "read-known-file",
        job: "Read a file you already know the path of",
        primary: "Read",
        never_via_bash: "`cat`, `head`, `tail`",
    },
    RouteSlot {
        id: "exact-content",
        job: "Exact string, symbol or regex inside file contents",
        primary: "Grep",
        never_via_bash: "`grep`, `rg`",
    },
    RouteSlot {
        id: "ranked-cross-file-locate",
        job: "You know the rough domain words but not the exact token, and want the top-ranked regions across many files instead of every match",
        primary: "CodebaseSearch",
        never_via_bash: "—",
    },
    RouteSlot {
        id: "file-name",
        job: "Find files by name / glob pattern",
        primary: "Glob",
        never_via_bash: "`find`",
    },
    RouteSlot {
        id: "call-structure",
        job: "Who calls this, what does it call, what breaks if I change it",
        primary: "CodeGraph",
        never_via_bash: "—",
    },
    RouteSlot {
        id: "project-overview",
        job: "Statistical overview of what this repo contains",
        primary: "ProjectMap",
        never_via_bash: "—",
    },
    RouteSlot {
        id: "edit-one",
        job: "Change one site in a file you have read",
        primary: "Edit",
        never_via_bash: "`sed`, `awk`",
    },
    RouteSlot {
        id: "edit-many",
        job: "Change several coupled sites in one pass",
        primary: "multi_edit",
        never_via_bash: "`sed`, `awk`",
    },
    RouteSlot {
        id: "create-file",
        job: "Create a new file",
        primary: "Write",
        never_via_bash: "`echo >`",
    },
    RouteSlot {
        id: "list-directory",
        job: "List a directory",
        primary: "ListDir",
        never_via_bash: "`ls`",
    },
    RouteSlot {
        id: "run-command",
        job: "Run builds, tests, package installs, git",
        primary: "Bash",
        never_via_bash: "—",
    },
    RouteSlot {
        id: "track-work",
        job: "Track multi-step work",
        primary: "TodoWrite",
        never_via_bash: "—",
    },
    RouteSlot {
        id: "check-compile",
        job: "Check compile / lint errors after an edit",
        primary: "get_diagnostics",
        never_via_bash: "—",
    },
    RouteSlot {
        id: "run-tests",
        job: "Run the test suite",
        primary: "run_tests",
        never_via_bash: "—",
    },
    RouteSlot {
        id: "discover-long-tail",
        job: "Find a long-tail tool you don't already know by name",
        primary: "tool_search",
        never_via_bash: "—",
    },
];

/// 按 id 取槽。
pub fn slot(id: &str) -> Option<&'static RouteSlot> {
    ROUTING_MATRIX.iter().find(|s| s.id == id)
}

/// 某槽的主管工具；槽不存在返回 `None`。
pub fn primary_for(id: &str) -> Option<&'static str> {
    slot(id).map(|s| s.primary)
}

/// 渲染成模型看到的那张表（markdown）。
pub fn render_matrix_markdown() -> String {
    let mut out =
        String::from("| Job | Tool | Never via bash |\n|------|------|----------------|\n");
    for s in ROUTING_MATRIX {
        out.push_str(&format!(
            "| {} | `{}` | {} |\n",
            s.job, s.primary, s.never_via_bash
        ));
    }
    out.trim_end().to_string()
}

/// Workflow 段：定位 → 读 → 改 → 验。
///
/// 这段话以前也是手写的，并且和表格一起漂移过。现在它由矩阵推导：
/// 前三步点名 `ranked-cross-file-locate` / `exact-content` / `file-name` 三槽的
/// 主管工具，槽换了这里自动跟着换。
pub fn render_workflow_markdown() -> String {
    let ranked = primary_for("ranked-cross-file-locate").unwrap_or("CodebaseSearch");
    let exact = primary_for("exact-content").unwrap_or("Grep");
    let by_name = primary_for("file-name").unwrap_or("Glob");
    let read = primary_for("read-known-file").unwrap_or("Read");
    let edit_one = primary_for("edit-one").unwrap_or("Edit");
    let edit_many = primary_for("edit-many").unwrap_or("multi_edit");
    let diag = primary_for("check-compile").unwrap_or("get_diagnostics");
    let tests = primary_for("run-tests").unwrap_or("run_tests");

    format!(
        "## Workflow\n\
         1. **Locate** — when you know the rough domain words but not the exact token and you want the\n   \
         most relevant regions across the codebase, call `{ranked}` first: it ranks and fuses matches\n   \
         so you get the important files, not every hit. When you already know the literal string or\n   \
         symbol, go straight to `{exact}`. For file names go to `{by_name}`.\n\
         2. **Read** — `{read}` the relevant range, then edit in the same response.\n\
         3. **Edit** — `{edit_one}` for one change; `{edit_many}` when changes are coupled across sites.\n\
         4. **Verify** — `{diag}`, then `{tests}` or the project's test command.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 槽 id 唯一、工具名唯一（一个工具不能霸占两个槽，否则"这个活儿归谁"又变成多解）。
    #[test]
    fn slot_ids_and_primaries_are_unique() {
        let mut ids = HashSet::new();
        let mut primaries = HashSet::new();
        for s in ROUTING_MATRIX {
            assert!(ids.insert(s.id), "重复的槽 id: {}", s.id);
            assert!(!s.job.trim().is_empty(), "槽 {} 没有描述活儿", s.id);
            assert!(
                primaries.insert(s.primary),
                "工具 {} 霸占了多个槽：'何时用它'会重新变成多解",
                s.primary
            );
        }
    }

    /// 每个槽的主管工具都必须是一个注册名，且有对应的 `tool-description-*.md`。
    /// 没有描述文件的工具 = 模型只看到一行 schema 短句，等于没人解释。
    #[test]
    fn every_slot_primary_has_a_description_file() {
        for s in ROUTING_MATRIX {
            assert!(
                crate::core::prompts::tool_descriptions::tool_description_filename(s.primary)
                    .is_some(),
                "槽 `{}` 的主管工具 {:?} 没有 tool-description 映射",
                s.id,
                s.primary
            );
            assert!(
                crate::core::prompts::tool_descriptions::resolve_tool_description(s.primary)
                    .is_some(),
                "槽 `{}` 的主管工具 {:?} 解析不出 schema 描述",
                s.id,
                s.primary
            );
        }
    }

    #[test]
    fn workflow_mentions_the_three_locate_tools() {
        let wf = render_workflow_markdown();
        for tool in ["CodebaseSearch", "Grep", "Glob", "Read"] {
            assert!(wf.contains(tool), "Workflow 段点名了 {tool}：\n{wf}");
        }
    }

    #[test]
    fn rendered_matrix_lists_every_slot() {
        let md = render_matrix_markdown();
        for s in ROUTING_MATRIX {
            assert!(md.contains(s.primary), "表格漏了 {}", s.primary);
        }
    }

    // ── 一致性守卫：让"四份文案各说各话"不再可能悄悄发生 ──────────────────────

    use crate::core::prompts::tool_descriptions;

    /// **每个槽有且只有一个主张者。**
    ///
    /// 这条直接钉死本次事故的根因：`CodebaseSearch` 曾被 `code_graph.md` /
    /// `projectmap.md` 说成"概念问题走这里"，同时它自己的描述又说"不做概念问题"。
    /// 两个工具抢同一个槽（或一个槽没人认领），这里就红。
    #[test]
    fn every_slot_has_exactly_one_claimer() {
        let claims = tool_descriptions::declared_routing_claims();

        for s in ROUTING_MATRIX {
            let claimers: Vec<&str> = claims
                .iter()
                .filter(|(_, routing, _)| routing == s.id)
                .map(|(f, _, _)| f.as_str())
                .collect();

            assert_eq!(
                claimers.len(),
                1,
                "槽 `{}`（{}）有 {} 个主张者：{:?}。一个槽只能有一个主管工具，\
                 否则模型会收到互相矛盾的指令并退回最保守的那个（Grep）。",
                s.id,
                s.job,
                claimers.len(),
                claimers
            );
            assert_eq!(
                tool_descriptions::resolve_tool_routing(s.primary).as_deref(),
                Some(s.id),
                "槽 `{}` 的主管工具是 {}，但它的描述文件没有声明 routing: {}",
                s.id,
                s.primary,
                s.id
            );
        }
    }

    /// 没有任何描述文件可以声明矩阵里不存在的槽。
    /// 挡住"描述文件自己发明了一个职责，模型照着走但矩阵不知道"。
    #[test]
    fn no_description_file_claims_an_unknown_slot() {
        for (filename, routing, tools) in tool_descriptions::declared_routing_claims() {
            assert!(
                slot(&routing).is_some(),
                "{filename} 声明了矩阵里不存在的槽 `routing: {routing}`（工具 {tools:?}）"
            );
        }
    }

    /// 声明了槽的文件必须真的被某个工具用上——否则是复制粘贴留下的孤儿描述，
    /// 而它照样会进 prompt bundle 干扰模型。
    #[test]
    fn every_routing_claim_belongs_to_a_real_tool() {
        for (filename, routing, tools) in tool_descriptions::declared_routing_claims() {
            assert!(
                !tools.is_empty(),
                "{filename} 声明了 `routing: {routing}`，但没有任何注册工具映射到这个描述文件"
            );
        }
    }
}
