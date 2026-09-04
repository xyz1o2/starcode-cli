//! 本次调用用的命令名。
//!
//! `sc`、`starcode`、`starcode-cli` 是同一个二进制的三个入口 —— cargo 里只有
//! `starcode-cli` 一个 bin target，另外两个名字由 `install.sh` / `install.ps1`
//! 在安装目录里做成指向它的链接。这样做而不是声明三个 `[[bin]]`，是因为一个
//! release 二进制 55 MB，声明三个就要链三遍、占三份磁盘。
//!
//! 于是 help、usage、以及"下次这样接着跑"这类提示都不能硬编码 `starcode-cli`：
//! 用 `sc` 启动的人照着提示敲 `starcode-cli --resume …` 未必反应得过来那是同一个东西。

use std::ffi::OsStr;

/// argv[0] 不可用时的兜底名字，也是 cargo 里真正的 bin target 名。
pub const CANONICAL_PROGRAM_NAME: &str = "starcode-cli";

/// 主命令的简称，安装脚本会为它建链接。
pub const SHORT_PROGRAM_NAME: &str = "sc";

/// 全部入口名，按由短到长排列（对外展示时就是这个顺序）。
///
/// 用来拼进程匹配用的正则：因为模式尾部锚了空白或行尾，`sc` 分支不会误吃到
/// `starcode` 的前两个字母，所以这里的先后顺序不影响匹配结果。
pub const PROGRAM_ALIASES: [&str; 3] = [SHORT_PROGRAM_NAME, "starcode", CANONICAL_PROGRAM_NAME];

/// 本次调用用的命令名：argv[0] 的文件名，去掉 `.exe` 后缀。
pub fn program_name() -> String {
    program_name_from_argv0(&std::env::args_os().next().unwrap_or_default())
}

/// 把 argv[0] 折成命令名。
///
/// argv[0] 可能是绝对路径（`/home/u/.cargo/bin/sc`）、相对路径
/// （`./target/release/starcode-cli`），也可能被调用方伪造成空串或纯路径分隔符 ——
/// 后两种取不出文件名，回落到规范名，绝不返回空串。
fn program_name_from_argv0(argv0: &OsStr) -> String {
    let stem = std::path::Path::new(argv0)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if stem.is_empty() {
        CANONICAL_PROGRAM_NAME.to_string()
    } else {
        stem
    }
}

/// 匹配"任意一个入口名启动的进程"的 ERE，供 `pgrep -f` 用。
///
/// 前面锚 `^` 或 `/`、后面锚空白或行尾，`sc` 才不会命中 `discord`、`rustc -o /tmp/x` 之类。
pub fn process_match_pattern() -> String {
    format!("(^|/)({})([[:space:]]|$)", PROGRAM_ALIASES.join("|"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_folds_to_the_bare_command_name() {
        assert_eq!(
            program_name_from_argv0(OsStr::new("/home/u/.cargo/bin/sc")),
            "sc"
        );
        assert_eq!(
            program_name_from_argv0(OsStr::new("./target/release/starcode-cli")),
            "starcode-cli"
        );
    }

    #[test]
    fn exe_suffix_is_stripped() {
        assert_eq!(program_name_from_argv0(OsStr::new("sc.exe")), "sc");
    }

    #[test]
    fn unusable_argv0_falls_back_to_the_canonical_name() {
        // 空 argv[0] 和纯分隔符都取不出文件名 —— 不能因此把提示里的命令名印成空白
        for argv0 in ["", "/", "..", "."] {
            assert_eq!(
                program_name_from_argv0(OsStr::new(argv0)),
                CANONICAL_PROGRAM_NAME,
                "argv[0]={:?} 没有回落到规范名",
                argv0
            );
        }
    }

    #[test]
    fn aliases_cover_both_named_constants() {
        // 两个常量单独被引用，别改了一个忘了另一个
        assert!(PROGRAM_ALIASES.contains(&SHORT_PROGRAM_NAME));
        assert!(PROGRAM_ALIASES.contains(&CANONICAL_PROGRAM_NAME));
    }

    #[test]
    fn process_pattern_lists_every_alias() {
        let pattern = process_match_pattern();
        for alias in PROGRAM_ALIASES {
            assert!(
                pattern.contains(alias),
                "进程匹配模式漏了入口名 {}: {}",
                alias,
                pattern
            );
        }
        // 两头必须有锚，否则 `sc` 会把任何含 "sc" 的命令行都算进来
        assert!(pattern.starts_with("(^|/)"), "模式前端没锚: {}", pattern);
        assert!(
            pattern.ends_with("([[:space:]]|$)"),
            "模式后端没锚: {}",
            pattern
        );
    }
}
