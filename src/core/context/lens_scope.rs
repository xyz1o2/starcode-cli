//! StarLens Managed Scope：决定每个文件的认知角色，并强制 secret 安全红线。
//!
//! 角色评估顺序（前者优先，不可被后者覆盖）：
//! 1. secret 红线 → 永远 Exclude（内容会送给 cheap model，这条是硬红线）；
//! 2. 结构性排除（构建产物 / 依赖 / VCS / 参考副本 / 运行态）→ Exclude；
//! 3. 非可解析/文本扩展 → Observe（只存 hash 测漂移，不建语义 entry）；
//! 4. 按 profile 判 Index / Observe。

use super::lens::ScopeRole;

/// 注入 profile：`Core` 只把核心子系统纳入全认知，`Full` 全仓源码皆 Index。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeProfile {
    Core,
    Full,
}

/// 可建语义 entry 的扩展名（镜像 codebase_search 的可索引集合，保持本模块自洽）。
const CANDIDATE_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp", "rb", "php",
    "swift", "kt", "scala", "cs", "toml", "json", "yaml", "yml", "md", "sh", "sql",
];

fn ext_of(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map(|(_, e)| e)
}

fn is_candidate_ext(path: &str) -> bool {
    ext_of(path).is_some_and(|e| CANDIDATE_EXTS.contains(&e))
}

fn is_code_ext(path: &str) -> bool {
    const CODE: &[&str] = &[
        "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp", "rb", "php",
        "swift", "kt", "scala", "cs",
    ];
    ext_of(path).is_some_and(|e| CODE.contains(&e))
}

/// secret 安全红线：永远强制 Exclude，任何 profile 都不能覆盖。
///
/// 原则：对**非源码文件**用关键词启发式（secret/password/credential），
/// 对源码文件只认精确名与敏感扩展——否则像 `token_counter.rs` 这类正常源码会被误杀。
pub fn is_secret_path(path: &str) -> bool {
    let p = path.replace('\\', "/");
    let name = p.rsplit('/').next().unwrap_or(&p);
    let lower = name.to_ascii_lowercase();

    const SECRET_NAMES: &[&str] = &[
        ".env",
        "credentials",
        "credentials.json",
        "id_rsa",
        "id_ed25519",
        ".netrc",
        ".npmrc",
        ".pgpass",
        "secring.gpg",
    ];
    if SECRET_NAMES.contains(&name) || name.starts_with(".env.") {
        return true;
    }

    const SECRET_SUFFIXES: &[&str] = &[
        ".pem",
        ".key",
        ".p12",
        ".pfx",
        ".keystore",
        ".jks",
        ".crt",
        ".secret",
    ];
    if SECRET_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
        return true;
    }

    // 关键词启发式只作用于非源码文件，避免误杀 token_counter.rs 之类。
    if !is_code_ext(&p)
        && (lower.contains("secret") || lower.contains("password") || lower.contains("credential"))
    {
        return true;
    }

    // 运行态凭据
    if p.contains(".star/") && (lower.contains("oauth") || lower.contains("cred")) {
        return true;
    }
    false
}

/// 评估一个相对路径的认知角色。
pub fn evaluate(path: &str, profile: ScopeProfile) -> ScopeRole {
    if is_secret_path(path) {
        return ScopeRole::Exclude;
    }
    let p = path.replace('\\', "/");

    const EXCLUDE_DIRS: &[&str] = &[
        ".git/",
        "target/",
        "node_modules/",
        ".star/",
        "study_or_copy_projects/",
        "dist/",
        "build/",
    ];
    if EXCLUDE_DIRS.iter().any(|d| p.contains(d)) {
        return ScopeRole::Exclude;
    }

    if !is_candidate_ext(&p) {
        return ScopeRole::Observe;
    }

    match profile {
        ScopeProfile::Full => ScopeRole::Index,
        ScopeProfile::Core => {
            const CORE_PREFIXES: &[&str] = &[
                "src/runtime/",
                "src/agent/",
                "src/core/tools/",
                "src/core/context/",
                "src/core/config/",
                "src/ui/state/",
                "src/ui/services/",
            ];
            let name = p.rsplit('/').next().unwrap_or(&p);
            const CORE_FILES: &[&str] = &["Cargo.toml", "CLAUDE.md", "STAR.md", "STARCODE.md"];
            if CORE_PREFIXES.iter().any(|pre| p.starts_with(pre)) || CORE_FILES.contains(&name) {
                ScopeRole::Index
            } else {
                ScopeRole::Observe
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_gate_catches_sensitive_files() {
        assert!(is_secret_path(".env"));
        assert!(is_secret_path(".env.production"));
        assert!(is_secret_path("config/server.pem"));
        assert!(is_secret_path("deploy/id_rsa"));
        assert!(is_secret_path("ops/secrets.yaml"));
        assert!(is_secret_path("infra/db-password.txt"));
        assert!(is_secret_path(".star/oauth_creds.json"));
    }

    #[test]
    fn secret_gate_does_not_nuke_source_code() {
        // 带 token/secret 字样的正常源码不该被误判为 secret。
        assert!(!is_secret_path("src/agent/compact/token_counter.rs"));
        assert!(!is_secret_path("src/core/secret_scanner.rs"));
        assert!(!is_secret_path("src/auth/password_hash.rs"));
    }

    #[test]
    fn secret_gate_overrides_profile() {
        // 即便落在 core 前缀下，secret 也必须 Exclude。
        assert_eq!(
            evaluate("src/core/config/credentials.json", ScopeProfile::Full),
            ScopeRole::Exclude
        );
    }

    #[test]
    fn core_profile_indexes_core_subsystems_only() {
        assert_eq!(
            evaluate("src/runtime/messages.rs", ScopeProfile::Core),
            ScopeRole::Index
        );
        assert_eq!(
            evaluate("src/agent/compact/token_counter.rs", ScopeProfile::Core),
            ScopeRole::Index
        );
        assert_eq!(evaluate("Cargo.toml", ScopeProfile::Core), ScopeRole::Index);
        // 非核心子系统的源码只 Observe。
        assert_eq!(
            evaluate("src/utils/helpers.rs", ScopeProfile::Core),
            ScopeRole::Observe
        );
        assert_eq!(
            evaluate("README.md", ScopeProfile::Core),
            ScopeRole::Observe
        );
    }

    #[test]
    fn full_profile_indexes_all_source() {
        assert_eq!(
            evaluate("src/utils/helpers.rs", ScopeProfile::Full),
            ScopeRole::Index
        );
    }

    #[test]
    fn structural_and_nonsource_exclusions() {
        assert_eq!(
            evaluate("target/debug/build.rs", ScopeProfile::Full),
            ScopeRole::Exclude
        );
        assert_eq!(
            evaluate("node_modules/x/index.js", ScopeProfile::Full),
            ScopeRole::Exclude
        );
        assert_eq!(
            evaluate(
                "study_or_copy_projects/claude-code-main/a.ts",
                ScopeProfile::Full
            ),
            ScopeRole::Exclude
        );
        // 非候选扩展（如图片）→ Observe，只测漂移。
        assert_eq!(
            evaluate("assets/logo.png", ScopeProfile::Full),
            ScopeRole::Observe
        );
    }
}
