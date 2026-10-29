//! 把 `.star/` 自动写进本仓库的 `.git/info/exclude`。
//!
//! 对标 Claude Code 对 `.claude/settings.local.json` 的处理：它第一次替
//! 用户写入本地配置时，会顺手把路径写进**全局 git excludes**，让用户的
//! `git status` 永远干净（用户无感知、也不会误提交机器状态）。
//! starcode 的 `.star/` 是同样的东西：agent 的索引快照、语义引擎、日志、
//! 上下文定义，全是本机运行时产物。
//!
//! 为什么写 `.git/info/exclude` 而不是 `.gitignore`：
//! - excludes 是**纯本地**文件（不进版本历史，不会被 push 到远端）——
//!   把「这个用户机器上的状态目录」写进团队共享的 `.gitignore` 是越权；
//! - 不与用户手工维护的 `.gitignore` 内容混在一起，用户 `git diff`
//!   永远看不到这行噪音；
//! - 效果完全相同：三个来源（`.gitignore`、`.git/info/exclude`、
//!   `core.excludesFile`）在 git 里平级。
//!
//! 环境变量退出机制：`STAR_AUTO_GIT_EXCLUDE=0` 关闭自动写入（0/false/off）。
//!
//! 这里的写入是**幂等一次性**的：只在「任意未注释行包含 `.star`」缺失时
//! 追加。用户日后删掉这行，说明他明确重新要让 git 看见 `.star/`，
//! 我们尊重这个选择 —— 不在他的排除文件里打游击战。

use std::path::Path;
use std::sync::OnceLock;

/// 自动写入开关：默认开启，`STAR_AUTO_GIT_EXCLUDE=0` 显式关闭。
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("STAR_AUTO_GIT_EXCLUDE")
            .ok()
            .map(|v| {
                let v = v.to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(true)
    })
}

/// 从项目根解析出 `.git` 目录。
///
/// 两种形态：
/// - `root/.git/` —— 普通仓库（目录）；
/// - `root/.git` 文件内容为 `gitdir: <path>` —— worktree/submodule。
///   `<path>` 可能是相对路径（相对 `.git` 文件所在目录），要拎出来合并；
///   worktree 的 gitdir 里同样有 `info/`，写进它的 exclude 即可。
fn resolve_git_dir(root: &Path) -> Option<std::path::PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if dot_git.is_file() {
        let marker = std::fs::read_to_string(&dot_git).ok()?;
        let line = marker.lines().find_map(|l| l.strip_prefix("gitdir:"))?;
        let dir = Path::new(line.trim());
        if dir.is_absolute() {
            return Some(dir.to_path_buf());
        }
        return Some(dot_git.parent()?.join(dir));
    }
    None
}

/// 幂等地把 `.star/` 加进 `<gitdir>/info/exclude`。
///
/// 行为约束：
/// - 不是 git 仓库 → 直接返回 `Ok(false)`（研究资料目录这类纯资料目录
///   会走到这里，不是错误，也不该刷日志）；
/// - 已存在（任意未注释行包含 `.star`，不管是 `.star`、`.star/` 还是
///   `.star/` 带尾注释）→ 不动文件；
/// - `info/` 目录缺失则创建；exclude 文件缺失则创建；已存在内容不重排
///   不覆盖，只在文件尾追加（原有内容没以换行结尾时先补一个换行）；
/// - Windows 上多进程同时建索引 → 竞争时保留失败者安静返回。
///
/// 返回是否这次真的写了文件（调用方拿去记日志，没写就不说话）。
pub fn ensure_star_excluded(project_root: &Path) -> std::io::Result<bool> {
    if !enabled() {
        return Ok(false);
    }
    let Some(git_dir) = resolve_git_dir(project_root) else {
        return Ok(false);
    };
    let info_dir = git_dir.join("info");
    let exclude_path = info_dir.join("exclude");

    let existing = match std::fs::read_to_string(&exclude_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };

    let already_covered = existing
        .lines()
        .any(|line| !line.trim_start().starts_with('#') && line.contains(".star"));
    if already_covered {
        return Ok(false);
    }

    std::fs::create_dir_all(&info_dir)?;

    // 用户已有的 exclude 末尾可能没有换行，直接 append 会把两行粘成一
    // 行，模式就失效了 —— 补一个换行再写我们自己的（注释 + 模式）。
    let mut out = String::new();
    // 无论原有内容是否以换行结尾，统一先补一个换行再写我们的注释块：
    // 已有换行就多出一个空行（纯外观），没有就正好在末行断开 —— 两种
    // 情况 git 都解析正确，且永远不把用户末行和注释粘成一个词。
    out.push_str("\n# starcode: agent 的本地状态目录（索引快照/语义引擎/日志），本机不入库\n");
    out.push_str(".star/\n");

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&exclude_path)?;
    file.write_all(out.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注意：测试环境变量在并行测试里互相干扰，所以 enabled() 的开关
    /// 只在 parse 层验证逻辑（与 sensitive_paths 同样的取舍），
    /// 这里集中测 resolve_git_dir 与幂等路径。
    #[test]
    fn resolve_both_git_shapes() {
        let temp = tempfile::tempdir().expect("tempdir");
        // 普通 .git 目录
        std::fs::create_dir(temp.path().join(".git")).unwrap();
        assert_eq!(
            resolve_git_dir(temp.path()),
            Some(temp.path().join(".git"))
        );
        // worktree 形态：.git 是文件，gitdir 是相对主仓库的相对路径
        let linked = temp.path().join("linked");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(
            linked.join(".git"),
            "gitdir: ../main_repo/.git/worktrees/feat\n",
        )
        .unwrap();
        assert_eq!(
            resolve_git_dir(&linked),
            Some(linked.join("../main_repo/.git/worktrees/feat"))
        );
    }

    #[test]
    fn writes_once_then_idempotent() {
        // CAREFUL: enabled() 读全局 env（测试环境默认 on）。若 CI 上有人
        // 设了 STAR_AUTO_GIT_EXCLUDE=0，这个测试会失效——但单测脚本不设。
        let temp = tempfile::tempdir().expect("tempdir");
        let git_dir = temp.path().join(".git");
        std::fs::create_dir_all(git_dir.join("worktrees")).unwrap();

        let exclude_path = git_dir.join("info").join("exclude");
        assert!(!exclude_path.exists());

        // 第一次：写入
        assert!(ensure_star_excluded(temp.path()).expect("write"));
        let content = std::fs::read_to_string(&exclude_path).unwrap();
        assert!(content.contains(".star/"));

        // 第二次：不动
        assert!(!ensure_star_excluded(temp.path()).expect("write"));
        assert_eq!(std::fs::read_to_string(&exclude_path).unwrap(), content);

        // 用户手工加过别的模式 → 不重排，append 到尾部；已有 .star 行 → 不再写
        std::fs::write(
            &exclude_path,
            "# 我自己的排除\nnode_modules/\n# starcode: agent 的本地状态目录（索引快照/语义引擎/日志），本机不入库\n.star/\n",
        )
        .unwrap();
        assert!(!ensure_star_excluded(temp.path()).expect("write"));
    }

    #[test]
    fn appends_after_unterminated_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let git_dir = temp.path().join(".git");
        let exclude_path = git_dir.join("info").join("exclude");
        std::fs::create_dir_all(git_dir.join("info")).unwrap();
        // 末尾无换行的用户内容
        std::fs::write(&exclude_path, "node_modules/").unwrap();
        ensure_star_excluded(temp.path()).expect("write");
        let content = std::fs::read_to_string(&exclude_path).unwrap();
        // 注意新行必须补一个换行开头，否则会把用户的末行和注释粘成一行
        assert_eq!(
            content,
            "node_modules/\n# starcode: agent 的本地状态目录（索引快照/语义引擎/日志），本机不入库\n.star/\n",
            "append 前补换行，粘住的两行 git 解析不了"
        );
    }

    #[test]
    fn non_repo_is_quiet() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(!ensure_star_excluded(temp.path()).expect("quiet"));
    }
}
