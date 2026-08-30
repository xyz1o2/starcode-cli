use crate::types::RiskLevel;

pub fn estimate_bash_risk(command: &str) -> RiskLevel {
    let cmd0 = command.trim();
    if cmd0.is_empty() {
        return RiskLevel::Low;
    }

    let cmd = cmd0.to_lowercase();
    let head = cmd.split_whitespace().next().unwrap_or("").to_string();

    // 0) 命令注入/组合执行信号：出现时应触发更严格的确认
    // 参考：StarCode 的 bash-prefix-detection 规范
    let has_substitution = cmd.contains("$(") || cmd.contains('`');
    let has_chain = cmd.contains("&&") || cmd.contains("||") || cmd.contains(';');
    let has_comment_like = cmd.contains('#');

    // 1) 结构性高风险：重定向/管道往往会改文件或组合危险链
    let has_pipe = cmd.contains('|');
    let has_redirect = cmd.contains(" >")
        || cmd.contains(">>")
        || cmd.contains("<")
        || cmd.contains("2>")
        || cmd.contains("1>");

    // 2) 极度危险关键字/模式
    let has_rm_rf = cmd.contains("rm -rf") || cmd.contains("rm -fr") || cmd.contains("rm -r -f");
    let has_del_recursive = cmd.contains("del /s") && (cmd.contains("/q") || cmd.contains("/f"));
    let has_rmdir_recursive = cmd.contains("rmdir /s") || cmd.contains("rd /s");
    let has_format = cmd.contains("format ") || cmd.contains(" format ");
    let has_diskpart = cmd.contains("diskpart");
    let has_reg_delete = cmd.contains("reg delete") || cmd.contains("regdel");
    let has_bcdedit = cmd.contains("bcdedit");
    let has_service_delete = cmd.contains("sc delete");

    // 下载执行链：curl/wget/irm/iwr -> sh/iex
    let has_curl_or_wget = cmd.contains("curl ") || cmd.contains("wget ");
    let has_pipe_to_sh =
        has_pipe && (cmd.contains("| sh") || cmd.contains("|bash") || cmd.contains("| bash"));
    let has_powershell = cmd.contains("powershell") || cmd.contains("pwsh");
    let has_irm_or_iwr = cmd.contains("irm ")
        || cmd.contains("iwr ")
        || cmd.contains("invoke-webrequest")
        || cmd.contains("invoke-restmethod");
    let has_pipe_to_iex = has_pipe
        && (cmd.contains("| iex")
            || cmd.contains("|invoke-expression")
            || cmd.contains("| invoke-expression"));

    if has_rm_rf
        || has_format
        || has_diskpart
        || has_del_recursive
        || has_rmdir_recursive
        || has_reg_delete
        || has_bcdedit
        || has_service_delete
        || (has_curl_or_wget && has_pipe_to_sh)
        || (has_powershell && has_irm_or_iwr && has_pipe_to_iex)
    {
        return RiskLevel::Critical;
    }

    // 2.5) 注入信号：不一定是“必然恶意”，但需要用户确认
    if has_substitution || has_chain || has_comment_like {
        return RiskLevel::High;
    }

    // 3) Git 高风险子命令
    if head == "git" {
        if cmd.contains(" reset --hard")
            || cmd.contains(" clean -fd")
            || cmd.contains(" clean -xdf")
            || cmd.contains(" push --force")
            || cmd.contains(" push -f")
        {
            return RiskLevel::High;
        }
        return RiskLevel::Medium;
    }

    // 4) 按 head 的基础分级
    let mut base = match head.as_str() {
        // 安全查询
        "pwd" | "whoami" | "echo" => RiskLevel::Safe,
        // 读取/低风险
        "type" | "where" | "dir" => RiskLevel::Low,
        // 中风险（可能改状态）
        "cargo" | "npm" | "pnpm" | "yarn" => RiskLevel::Medium,
        // 高风险（明显改文件）
        "mv" | "move" | "copy" | "cp" | "ren" | "rename" => RiskLevel::High,
        "rm" | "del" | "erase" | "rmdir" | "rd" => RiskLevel::High,
        // 默认
        _ => RiskLevel::Medium,
    };

    // 5) 管道/重定向抬升风险（但不强行到 Critical）
    if matches!(base, RiskLevel::Safe) {
        base = RiskLevel::Low;
    }
    if has_pipe || has_redirect {
        if matches!(base, RiskLevel::Low) {
            base = RiskLevel::Medium;
        }
        if matches!(base, RiskLevel::Medium) {
            base = RiskLevel::High;
        }
    }

    base
}
