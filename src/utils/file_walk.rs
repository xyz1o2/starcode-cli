//! 项目文件遍历 —— 一份 ignore 口径，所有调用点共用。
//!
//! 之前树里有七套遍历：`ignore::WalkBuilder`（四份配置互不相同）、
//! `walkdir::WalkDir`（完全不认 `.gitignore`）、`glob::glob`（连硬编码
//! 黑名单都没有）、`fs::read_dir`。同一个仓库里的后果是：`Glob` 能翻出
//! `target/` 里两千多个文件、`Grep` 翻不出、`@` 选择器 87% 的名额花在
//! 被 `.gitignore` 忽略的路径上、`ListDir` 又会一头钻进 `target/`。
//!
//! 这里定一份口径，对齐 Claude Code：
//!
//! | 维度 | 口径 |
//! |------|------|
//! | dotfile | **默认可见** —— `.github/workflows`、`.env.example` 都是要读的 |
//! | VCS 目录 | **永远跳过**：`.git .svn .hg .bzr .jj .sl` |
//! | `.gitignore` | 默认尊重，且 `require_git(false)` —— 没有 `.git` 也生效 |
//! | `.ignore` / `.rgignore` | 永远尊重，不给开关（Claude Code 同此）|
//! | `~/.star/ignore`、`.starignore` | 默认尊重，本项目的扩展 |
//!
//! 硬编码黑名单只剩那 6 个 VCS 目录。`node_modules`、`target` 这些本来
//! 就在 `.gitignore` 里，再硬编码一遍只会在"用户就是要搜 target"时挡路。
//!
//! **include 和 exclude 的实现方式不同，这是故意的。** `ignore` 的
//! `Override` 一旦命中就短路返回（`dir.rs::matched` 里 overrides 优先级
//! 最高），所以正向 glob 走 `Override` 会连带把 `.gitignore` 也绕过去。
//! 于是：exclude 用 `Override`（命中即剪枝，正是想要的），include 用
//! `globset` 在产出侧过滤（`.gitignore` 依然生效）。

use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::overrides::{Override, OverrideBuilder};
use ignore::{Walk, WalkBuilder};

/// 版本控制元数据目录 —— 任何遍历都不该进去。
/// 取自 Claude Code 的 `VCS_DIRECTORIES_TO_EXCLUDE`（jj = Jujutsu，sl = Sapling）。
pub const VCS_DIRS: [&str; 6] = [".git", ".svn", ".hg", ".bzr", ".jj", ".sl"];

/// 全局忽略文件 `~/.star/ignore`，不存在则返回 `None`。
pub fn global_ignore_file() -> Option<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".star").join("ignore"))
        .filter(|path| path.exists())
}

/// 项目级忽略文件 `<root>/.starignore`，不存在则返回 `None`。
pub fn project_ignore_file(root: &Path) -> Option<PathBuf> {
    let path = root.join(".starignore");
    path.exists().then_some(path)
}

/// 遍历口径。`Default` 就是上面那张表，绝大多数调用点直接用默认值。
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// dotfile 是否可见。默认 `true` —— 对齐 Claude Code。
    pub include_hidden: bool,
    /// 是否尊重 `.gitignore` / 全局 gitignore / `.git/info/exclude`。
    pub respect_git_ignore: bool,
    /// 是否尊重 `~/.star/ignore` 和 `<root>/.starignore`。
    pub respect_star_ignore: bool,
    /// `None` 表示不限深度。
    pub max_depth: Option<usize>,
    /// 是否跟随符号链接。默认 `false`：跟随会绕开 ignore 规则，也可能成环。
    pub follow_links: bool,
    /// 只保留匹配这些 glob 的文件。空表示不过滤。
    pub includes: Vec<String>,
    /// 剪掉匹配这些 glob 的路径（目录会整棵剪掉）。
    pub excludes: Vec<String>,
    /// glob 匹配是否区分大小写。默认 `true`。
    pub case_sensitive: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            include_hidden: true,
            respect_git_ignore: true,
            respect_star_ignore: true,
            max_depth: None,
            follow_links: false,
            includes: Vec::new(),
            excludes: Vec::new(),
            case_sensitive: true,
        }
    }
}

impl WalkOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// `include = true` 表示 dotfile 可见。注意和 `WalkBuilder::hidden()` 语义相反。
    pub fn hidden(mut self, include: bool) -> Self {
        self.include_hidden = include;
        self
    }

    pub fn git_ignore(mut self, respect: bool) -> Self {
        self.respect_git_ignore = respect;
        self
    }

    pub fn star_ignore(mut self, respect: bool) -> Self {
        self.respect_star_ignore = respect;
        self
    }

    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    pub fn follow_links(mut self, follow: bool) -> Self {
        self.follow_links = follow;
        self
    }

    pub fn case_sensitive(mut self, sensitive: bool) -> Self {
        self.case_sensitive = sensitive;
        self
    }

    pub fn include<I, S>(mut self, globs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.includes.extend(globs.into_iter().map(Into::into));
        self
    }

    pub fn exclude<I, S>(mut self, globs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.excludes.extend(globs.into_iter().map(Into::into));
        self
    }

    /// 接上 `Config::file_filtering()` 里的 `Option<bool>`：`None` = 用默认值。
    pub fn with_file_filtering(
        mut self,
        respect_git_ignore: Option<bool>,
        respect_star_ignore: Option<bool>,
    ) -> Self {
        if let Some(value) = respect_git_ignore {
            self.respect_git_ignore = value;
        }
        if let Some(value) = respect_star_ignore {
            self.respect_star_ignore = value;
        }
        self
    }
}

/// 按统一口径构造 `WalkBuilder`。需要 `build_parallel()` 或再加定制时用它，
/// 只想拿迭代器就用 [`walk`]。
pub fn walk_builder(root: &Path, opts: &WalkOptions) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        // `hidden(true)` 的意思是"把隐藏文件忽略掉"，和参数名正好相反。
        .hidden(!opts.include_hidden)
        .git_ignore(opts.respect_git_ignore)
        .git_global(opts.respect_git_ignore)
        .git_exclude(opts.respect_git_ignore)
        // `.ignore` / `.rgignore` 永远尊重：那是用户显式写给搜索工具看的，
        // 不该被 respect_git_ignore 一起关掉。Claude Code 的注释也是这个口径。
        .ignore(true)
        // 没有 `.git` 目录也要认 `.gitignore` —— worktree、submodule、
        // 以及"还没 git init 的新项目"都属于这一类。
        .require_git(false)
        .follow_links(opts.follow_links)
        .max_depth(opts.max_depth);

    if opts.respect_star_ignore {
        // 先全局后项目：后加的优先级更高，项目配置盖过全局配置。
        if let Some(global) = global_ignore_file() {
            builder.add_ignore(global);
        }
        if let Some(project) = project_ignore_file(root) {
            builder.add_ignore(project);
        }
    }

    builder.overrides(build_excludes(root, opts));
    builder
}

/// 统一口径的目录迭代器。
pub fn walk(root: &Path, opts: &WalkOptions) -> Walk {
    walk_builder(root, opts).build()
}

/// 把 exclude glob 和 VCS 目录编成 `Override`。
///
/// 之前 `ripgrep.rs` 把 exclude pattern 塞给了 `add_ignore()` —— 那个参数
/// 是"忽略文件的路径"而不是 pattern，所以整段是静默 no-op。
fn build_excludes(root: &Path, opts: &WalkOptions) -> Override {
    let mut builder = OverrideBuilder::new(root);
    if !opts.case_sensitive {
        builder.case_insensitive(true).ok();
    }
    for pattern in &opts.excludes {
        // `OverrideBuilder` 里 `!` 的含义是反的：带 `!` 才是排除。
        let negated = if pattern.starts_with('!') {
            pattern.clone()
        } else {
            format!("!{pattern}")
        };
        builder.add(&negated).ok();
    }
    // VCS 目录最后加：gitignore 语义里后写的规则优先级更高，这样即便
    // 调用方自己传了 exclude 也不会把 `.git` 放回来。
    for dir in VCS_DIRS {
        builder.add(&format!("!{dir}/")).ok();
    }
    builder.build().unwrap_or_else(|_| Override::empty())
}

/// 把模型/用户写的 glob 规范化成"从 root 起算"的形式。
///
/// 不含 `/` 的 pattern 按 gitignore 语义在任意层级匹配 —— `rg -g '*.rs'`
/// 就是这样，模型写 `*.py` 想要的也是"所有 python 文件"，不是"根目录下的"。
pub fn normalize_glob(pattern: &str) -> String {
    let trimmed = pattern.trim().trim_start_matches("./");
    if trimmed.contains('/') {
        trimmed.to_string()
    } else {
        format!("**/{trimmed}")
    }
}

/// 编译一组 glob。列表为空或全部编译失败时返回 `None`，含义是"不过滤"。
///
/// 不含 glob 元字符的 pattern 额外补一条 `<p>/**`：调用方传 `src/tools`
/// 这种纯路径前缀时，按目录前缀理解（gitignore 对目录名也是这个语义），
/// 否则它只能匹配到一个同名文件，实际等于什么都匹配不到。
pub fn compile_globs(patterns: &[String], case_sensitive: bool) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    let mut added = 0usize;
    for pattern in patterns {
        let normalized = normalize_glob(pattern);
        let mut variants = vec![normalized.clone()];
        if !normalized.contains(['*', '?', '[', '{']) {
            variants.push(format!("{}/**", normalized.trim_end_matches('/')));
        }
        for variant in variants {
            let glob = GlobBuilder::new(&variant)
                // `*` 不跨 `/`，否则 `src/*.rs` 会匹配到 `src/a/b.rs`。
                .literal_separator(true)
                .case_insensitive(!case_sensitive)
                .build();
            if let Ok(glob) = glob {
                builder.add(glob);
                added += 1;
            }
        }
    }
    if added == 0 {
        return None;
    }
    builder.build().ok()
}

/// `opts.includes` 的产出侧过滤器。见模块头注释里为什么不走 `Override`。
pub fn include_matcher(opts: &WalkOptions) -> Option<GlobSet> {
    compile_globs(&opts.includes, opts.case_sensitive)
}

/// 用相对 `root` 的路径做 glob 匹配；`path` 不在 `root` 下时退回完整路径。
pub fn glob_matches(set: &GlobSet, root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    set.is_match(relative)
}

/// 路径里是否含 VCS 元数据目录 —— 给暂时还没换到统一 walker 的地方兜底。
pub fn contains_vcs_dir(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        VCS_DIRS.contains(&name.as_ref())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;

    /// 造一棵小树：一个被 gitignore 的目录、一个 dotfile、一个 `.git` 目录。
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join(".gitignore"), "target/\nsecret.txt\n").unwrap();
        fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("secret.txt"), "shh").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join("target/debug/build.rs"), "").unwrap();
        fs::create_dir_all(root.join(".github/workflows")).unwrap();
        fs::write(root.join(".github/workflows/ci.yml"), "").unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        dir
    }

    fn collect(root: &Path, opts: &WalkOptions) -> BTreeSet<String> {
        walk(root, opts)
            .flatten()
            .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
            .filter_map(|entry| {
                entry
                    .path()
                    .strip_prefix(root)
                    .ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
            })
            .collect()
    }

    #[test]
    fn dotfiles_visible_but_vcs_dirs_never_are() {
        let dir = fixture();
        let found = collect(dir.path(), &WalkOptions::new());
        // 关键回归：`.github/workflows/ci.yml` 必须能被看到 —— 之前七个
        // 遍历里有五个把它当"隐藏文件"跳过了。
        assert!(found.contains(".github/workflows/ci.yml"), "{found:?}");
        // `.git` 内部永远不进：即便 hidden 打开，也由 Override 剪掉。
        assert!(!found.contains(".git/HEAD"), "{found:?}");
    }

    #[test]
    fn gitignore_applies_without_a_git_dir() {
        let dir = fixture();
        // fixture 里的 `.git` 只是个空目录，不是仓库；require_git(false)
        // 才让 `.gitignore` 生效。
        let found = collect(dir.path(), &WalkOptions::new());
        assert!(!found.contains("target/debug/build.rs"), "{found:?}");
        assert!(!found.contains("secret.txt"), "{found:?}");
        assert!(found.contains("main.rs"), "{found:?}");

        // 关掉之后被忽略的文件回来，但 `.git` 依然进不去。
        let all = collect(dir.path(), &WalkOptions::new().git_ignore(false));
        assert!(all.contains("target/debug/build.rs"), "{all:?}");
        assert!(!all.contains(".git/HEAD"), "{all:?}");
    }

    #[test]
    fn exclude_globs_prune_instead_of_being_a_noop() {
        let dir = fixture();
        // 以前这个 pattern 被喂给 add_ignore()，静默无效。
        let opts = WalkOptions::new().exclude(["src"]);
        let found = collect(dir.path(), &opts);
        assert!(!found.contains("src/lib.rs"), "{found:?}");
        assert!(found.contains("main.rs"), "{found:?}");
    }

    #[test]
    fn include_globs_do_not_bypass_gitignore() {
        let dir = fixture();
        // `Override` 的正向 glob 一命中就短路返回，会把 .gitignore 一起绕过；
        // 所以 include 走产出侧过滤。这条测的就是"绕不过去"。
        let opts = WalkOptions::new().include(["*.rs"]);
        let matcher = include_matcher(&opts).expect("matcher");
        let found: BTreeSet<String> = collect(dir.path(), &opts)
            .into_iter()
            .filter(|rel| matcher.is_match(rel))
            .collect();
        assert!(found.contains("main.rs"), "{found:?}");
        assert!(found.contains("src/lib.rs"), "{found:?}");
        assert!(!found.contains("target/debug/build.rs"), "{found:?}");
    }

    #[test]
    fn bare_globs_match_at_any_depth() {
        // 模型写 `*.py` 要的是所有 python 文件，rg -g 也是这个语义。
        assert_eq!(normalize_glob("*.py"), "**/*.py");
        assert_eq!(normalize_glob("./src/**/*.rs"), "src/**/*.rs");
        assert_eq!(normalize_glob("src/*.rs"), "src/*.rs");

        let set = compile_globs(&["*.py".to_string()], true).unwrap();
        assert!(set.is_match("a/b/c.py"));
        assert!(set.is_match("c.py"));
        assert!(!set.is_match("c.pyc"));

        // `*` 不跨 `/`：src/*.rs 不该匹配 src/a/b.rs。
        let set = compile_globs(&["src/*.rs".to_string()], true).unwrap();
        assert!(set.is_match("src/lib.rs"));
        assert!(!set.is_match("src/a/b.rs"));
    }

    #[test]
    fn case_insensitive_globs() {
        let set = compile_globs(&["*.RS".to_string()], false).unwrap();
        assert!(set.is_match("main.rs"));
        assert!(compile_globs(&[], true).is_none());
    }

    #[test]
    fn plain_path_prefix_matches_subtree() {
        // Grep 的 include_pattern 常被写成纯路径前缀。之前的实现是子串比较，
        // 换成 glob 后 `src/tools` 必须还能匹配它下面的文件。
        let set = compile_globs(&["src/tools".to_string()], true).unwrap();
        assert!(set.is_match("src/tools"));
        assert!(set.is_match("src/tools/search.rs"));
        assert!(set.is_match("src/tools/a/b.rs"));
        assert!(!set.is_match("src/core/tools.rs"));

        // 带元字符的照旧，不额外补 `/**`。
        let set = compile_globs(&["*.rs".to_string()], true).unwrap();
        assert!(!set.is_match("main.rs/inner"));
    }

    #[test]
    fn max_depth_is_honoured() {
        let dir = fixture();
        let found = collect(dir.path(), &WalkOptions::new().max_depth(1));
        assert!(found.contains("main.rs"), "{found:?}");
        assert!(!found.contains("src/lib.rs"), "{found:?}");
    }
}
