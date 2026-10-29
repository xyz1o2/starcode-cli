//! 敏感文件防护 —— agent 工具调用前的一道「询问」门闸。
//!
//! 对标 Claude Code 的权限演进史：它先给 `Read(.env*)` 建议写 deny 规则，
//! 后来 Bash 的 `grep -r` / 重定向读取也开始被拦（issue #45200 记录了这次
//! 反转），再到 Grep 出现过按模式绕过读出密码的系列 issue（#57525/#44379）。
//! 教训是：只挡一个工具挡不住，必须挡在所有工具公共的参数入口上。
//!
//! 这里做的比 deny 温和一档：**每次都弹询问**（ask 语义），不是拒绝——
//! 用户可能就是想让 agent 帮忙改 `.env`。但有两点必须钉死：
//! - 会话级/持久级授权**不能降级它**：`AllowSession`/`ProceedAlways`
//!   不落库，下一个碰敏感文件的调用照样弹框（见 `tool_executor::record_approval`）；
//! - Bash 走 `command` 字符串扫描：tokenizer + basename 边界检查，
//!   `cat .env` 会命中，`cat .environment` 不会。
//!
//! 环境变量退出机制：`STAR_SENSITIVE_FILE_ASK=0` 整体关闭（0/false/off）。
//!
//! 两个边界语义（都是有意的，不是漏）：
//! - **只读结果缓存会短路第二次同参数读取**：`Read .env` 第二次命中
//!   tool cache 时连 `pass_gate` 都不进，直接返回已读内容 —— 内容已在
//!   上下文里，不算「新的一次访问」。第一次被拒（未缓存）时不会有这个
//!   效果，再读还是要弹。
//! - MCP 工具不扫：schema 形态未知，强扫的误报远多于拦下的暴露。
//!
//! 与 `compact::post_compact_cleanup` 的 sensitive_patterns 不同：
//! 那边是**内容**脱敏（把已读进上下文的 api_key 值抹掉），这里是
//! **访问路径**门控（根本不让你读）。两者互补，口径不必共用。

use std::path::Path;
use std::sync::OnceLock;

/// `.env.<模板后缀>` 的豁免集合：这些是给用户照抄的模板，里面没有真密钥。
/// 注意只豁免**精确同名**，`.env.example.local` 这种背了后缀的仍算敏感。
const ENV_TEMPLATE_SUFFIXES: [&str; 4] = ["example", "template", "sample", "dist"];

/// SSH 私钥的 stem 集合：`id_rsa`、`id_rsa.bak`、`id_rsa1` 都算；
/// `id_rsa.pub` 是公钥，不算。
const KEY_STEMS: [&str; 3] = ["id_rsa", "id_ed25519", "id_ecdsa"];

/// 敏感门控开关：默认开启，`STAR_SENSITIVE_FILE_ASK=0` 显式关闭。
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| parse_flag(std::env::var("STAR_SENSITIVE_FILE_ASK").ok().as_deref()))
}

/// 开关解析：`0/false/off` 关，其余（含未设置）开。提出来单独测，
/// 因为 `enabled()` 用 OnceLock 缓存全局值，测试里没法动态改环境变量。
fn parse_flag(raw: Option<&str>) -> bool {
    raw.map(|v| {
        let v = v.to_lowercase();
        !(v == "0" || v == "false" || v == "off")
    })
    .unwrap_or(true)
}

/// 判断一个 basename（文件名，不含目录）是否命中敏感清单。
///
/// 口径（与用户确认的一致）：
/// - `.env` 精确 + `.env.*` 前缀，豁免 `.env.example/.template/.sample/.dist`；
/// - `*.pem`（大小写不敏感：Windows 文件系统本身就不区分）；
/// - `.git-credentials`；
/// - `id_rsa*` / `id_ed25519*` / `id_ecdsa*`，跳过 `.pub`；
/// - **只认 basename**，任何目录下的都算——`.env` 挪进 `config/` 还是密钥。
///
/// 返回命中类别（用于卡片文案），`None` 表示不敏感。
fn sensitive_basename(name: &str) -> Option<&'static str> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let lower = name.to_ascii_lowercase();

    if lower == ".env" {
        return Some(".env");
    }
    if let Some(rest) = lower.strip_prefix(".env.") {
        if !ENV_TEMPLATE_SUFFIXES.contains(&rest) {
            return Some(".env.*");
        }
        return None;
    }
    if lower.ends_with(".pem") {
        return Some("*.pem");
    }
    if lower == ".git-credentials" {
        return Some(".git-credentials");
    }
    for stem in KEY_STEMS {
        if let Some(rest) = lower.strip_prefix(stem) {
            if rest.is_empty() {
                return Some(SSH_KEY_KIND);
            }
            // `id_rsa.pub` 是公钥放行；其余尾巴（.bak/.old/.old1/ombia）一律算私钥
            if rest != ".pub" {
                return Some(SSH_KEY_KIND);
            }
        }
    }
    None
}

const SSH_KEY_KIND: &str = "SSH private key (id_rsa/id_ed25519/id_ecdsa)";

/// 取一个**路径或文件名**的 basename 并判断。
fn sensitive_path(path: &str) -> Option<&'static str> {
    let path = path.trim().trim_matches(|c| c == '"' || c == '\'');
    if path.is_empty() {
        return None;
    }
    // Windows Path 同时接受 / 和 \，正是这里想要的（agent 命令多为 POSIX 风格，
    // 模块自己也可能被 Windows 上的工具调用）。
    let basename = Path::new(path).file_name().map(|s| s.to_string_lossy());
    let Some(basename) = basename else {
        return None;
    };
    sensitive_basename(&basename)
}

/// 扫描 Bash 类工具的 `command` 字符串。
///
/// 不做子串匹配（`.environment` 会包住 `.env`），而是：先按 shell 语义
/// 分词 —— 引号内的空格和元字符**不算**分隔符（`-m 'have a .env day'`
/// 里整个短句是一个词），引号本身结束当前词；再对每个词取最后一段路径
/// 做 basename 判断。引号内空格不分词，`cat '.env'` 照样命中 —— 和
/// shell 眼里的 argv 拆法一致，这就是我们要的「字符边界检查」。
///
/// 已知的取舍：`=` 不当边界，`FOO=.env`（赋值字符串）不会命中——
/// 宁可为一个极端罕误报让路，也不为 `--config=.env` 这种写法引入误报。
/// `~/.ssh/id_rsa*` 这类 glob 直接命中（token 以 cwd 展开后反正也是目
/// 标名）。
fn command_hit(command: &str) -> Option<&'static str> {
    let mut token = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut flush = |token: &mut String| -> Option<&'static str> {
        let hit = if token.is_empty() {
            None
        } else {
            sensitive_path(token)
        };
        token.clear();
        hit
    };
    for c in command.chars() {
        match c {
            '\'' if !in_double => {
                // 引号本身结束一个词（shell 里 'ab'cd' 的 argv 是 abcd，
                // 我们拆成 ab 与 cd —— 少数拼接写法会多拆两次，但每次
                // 拆出的片段照样过 basename 检查，不丢命中率）。
                if let Some(hit) = flush(&mut token) {
                    return Some(hit);
                }
                in_single = !in_single;
            }
            '"' if !in_single => {
                if let Some(hit) = flush(&mut token) {
                    return Some(hit);
                }
                in_double = !in_double;
            }
            ' ' | '\t' | '\n' | '\r' | '|' | ';' | '&' | '<' | '>' | '(' | ')' | '\\' | ','
                if !in_single && !in_double =>
            {
                if let Some(hit) = flush(&mut token) {
                    return Some(hit);
                }
            }
            _ => token.push(c),
        }
    }
    flush(&mut token)
}

/// 从工具参数里提取所有「可能是路径」的字符串。
///
/// starcode 工具参数口径：`file_path`（编辑类）、`path`（Grep/Glob/LS/
/// ReadManyFiles pattern 形态）、`notebook_path`（Notebook 系列）、
/// `paths`（字符串数组）。没被列出的键一律不动——宁可少查也不能把
/// `pattern="*.pem"` 这种搜索词误当路径。
fn path_args(args: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["file_path", "path", "notebook_path"] {
        if let Some(s) = args.get(key).and_then(|v| v.as_str()) {
            out.push(s.to_string());
        }
    }
    if let Some(arr) = args.get("paths").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                out.push(s.to_string());
            }
        }
    }
    out
}

/// 门闸入口：根据工具名和参数判断这次调用是否该先问用户。
///
/// 返回 `Some(命中类别)`（如 `.env`、`*.pem`），调用方用它拼确认卡片。
/// 工具覆盖面：
/// - Bash（含 RunTests 字面意义上的 command 工具）：扫 `command` 字符串；
/// - 其余带路径参数的工具（Read/Write/Edit 家族/Grep/Glob/LS/LSP/Notebook/
///   ReadManyFiles）：扫 `file_path`/`path`/`notebook_path`/`paths`。
/// MCP 工具（`mcp__*`）不在此列——参数 schema 形态未知，强扫只会误报。
pub fn guard_hit(tool_name: &str, args: &serde_json::Value) -> Option<&'static str> {
    if !enabled() || args.is_null() {
        return None;
    }
    if tool_name == "Bash" || tool_name == "RunTests" {
        if let Some(command) = args.get("command").and_then(|v| v.as_str()) {
            return command_hit(command);
        }
        return None;
    }
    if tool_name.starts_with("mcp__") {
        return None;
    }
    for path in path_args(args) {
        if let Some(kind) = sensitive_path(&path) {
            return Some(kind);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hit(tool: &str, args: serde_json::Value) -> Option<&'static str> {
        guard_hit(tool, &args)
    }

    #[test]
    fn env_file_hits_and_template_exempt() {
        assert_eq!(hit("Read", json!({"file_path": ".env"})), Some(".env"));
        assert_eq!(
            hit("Read", json!({"file_path": "D:/proj/.env"})),
            Some(".env")
        );
        assert_eq!(
            hit("Read", json!({"file_path": "D:/proj/.env.local"})),
            Some(".env.*")
        );
        assert_eq!(
            hit("Read", json!({"file_path": ".env.example"})),
            None,
            "模板文件不问"
        );
        assert_eq!(hit("Read", json!({"file_path": ".env.dist"})), None);
        assert_eq!(
            hit("Read", json!({"file_path": "config/.env"})),
            Some(".env"),
            "挪进子目录还是 .env"
        );
        assert_eq!(hit("Read", json!({"file_path": "my.env"})), None);
        assert_eq!(hit("Read", json!({"file_path": ".environment"})), None);
    }

    #[test]
    fn pem_and_ssh_keys() {
        assert_eq!(hit("Read", json!({"file_path": "server.pem"})), Some("*.pem"));
        assert_eq!(hit("Read", json!({"file_path": "server.PEM"})), Some("*.pem"));
        assert_eq!(hit("Read", json!({"file_path": "pem"})), None);
        assert_eq!(
            hit("Read", json!({"file_path": "~/.ssh/id_rsa"})),
            Some(SSH_KEY_KIND)
        );
        assert_eq!(
            hit("Read", json!({"file_path": "id_rsa.pub"})),
            None,
            "公钥放行"
        );
        assert_eq!(
            hit("Read", json!({"file_path": "id_rsa.bak"})),
            Some(SSH_KEY_KIND)
        );
        assert_eq!(
            hit("Read", json!({"file_path": "id_ed25519"})),
            Some(SSH_KEY_KIND)
        );
        assert_eq!(
            hit("Read", json!({"file_path": "id_ecdsa"})),
            Some(SSH_KEY_KIND)
        );
        assert_eq!(hit("Read", json!({"file_path": ".git-credentials"})), Some(".git-credentials"));
    }

    #[test]
    fn bash_command_scan() {
        assert_eq!(
            hit("Bash", json!({"command": "cat .env"})),
            Some(".env")
        );
        assert_eq!(
            hit("Bash", json!({"command": "grep -r token /home/u/proj/.env"})),
            Some(".env")
        );
        assert_eq!(
            hit("Bash", json!({"command": "'cat' '.env'"})),
            Some(".env"),
            "引号不豁免"
        );
        assert_eq!(
            hit("Bash", json!({"command": "cat .environment"})),
            None,
            "字符边界：.environment 不命中"
        );
        assert_eq!(hit("Bash", json!({"command": "cat x.env y.pem"})), Some("*.pem"));
        assert_eq!(
            hit("Bash", json!({"command": "ssh-keygen -t rsa -f id_rsa"})),
            Some(SSH_KEY_KIND)
        );
        assert_eq!(
            hit("Bash", json!({"command": "git commit -m 'have a .env day'"})),
            None,
            "引号内的句子是一个词（shell argv 语义），basename=day 不命中"
        );
        assert_eq!(hit("Bash", json!({"command": "echo hello > out.txt"})), None);
    }

    #[test]
    fn non_path_and_mcp_tools_do_not_scan() {
        assert_eq!(hit("Grep", json!({"path": "src", "pattern": ".env"})), None);
        assert_eq!(
            hit("Grep", json!({"path": "config/.env.mock"})),
            Some(".env.*"),
            "path 参数仍要扫——它就是被翻开的目标"
        );
        assert_eq!(
            hit("mcp__fs__read", json!({"file_path": ".env"})),
            None,
            "MCP 工具 schema 未知，不强扫"
        );
        assert_eq!(hit("Bash", json!({"command": "ls"})), None);
    }

    #[test]
    fn env_var_disables_gate() {
        // enabled() 用 OnceLock 缓存全局值，测试只能测它背后的解析函数。
        assert!(parse_flag(None));
        assert!(parse_flag(Some("1")));
        assert!(parse_flag(Some("true")));
        assert!(!parse_flag(Some("0")));
        assert!(!parse_flag(Some("false")));
        assert!(!parse_flag(Some("OFF")));
    }
}
