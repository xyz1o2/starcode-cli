//! Plugin marketplaces (Claude Code style).
//!
//! A marketplace is a git repo or local directory containing
//! `.claude-plugin/marketplace.json` that lists installable plugins.
//! The added-marketplace list is stored per-project at
//! `.star/extensions/plugin-marketplaces.json`; git sources are cloned into
//! `.star/extensions/marketplaces/<name>/`.

use super::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 官方默认 marketplace（对标 Claude Code 的 claude-plugins-official）。
pub const DEFAULT_MARKETPLACE_NAME: &str = "claude-plugins-official";
pub const DEFAULT_MARKETPLACE_SOURCE: &str =
    "https://github.com/anthropics/claude-plugins-official";

/// GitHub 加速镜像（可选）：设置 `STARCODE_GITHUB_MIRROR` 环境变量后，
/// 对 github.com 的 https clone URL 加镜像前缀（如 `https://gh-proxy.com/`）。
/// 仅在用户显式配置时启用，不做静默的第三方降级（插件代码供应链安全）。
fn mirror_github_url(url: &str) -> String {
    let prefix = std::env::var("STARCODE_GITHUB_MIRROR")
        .ok()
        .map(|p| p.trim().trim_end_matches('/').to_string());
    match prefix {
        Some(p)
            if !p.is_empty()
                && (url.starts_with("https://github.com/")
                    || url.starts_with("http://github.com/")) =>
        {
            format!("{}/{}", p, url)
        }
        _ => url.to_string(),
    }
}

/// Windows 友好的强制删除：git 的 `.git/objects` 文件常带只读位，
/// 直接 `remove_dir_all` 会 Permission denied；先递归清只读位再删，
/// 并带短暂重试（杀毒软件/索引器可能短暂占用句柄）。
async fn force_remove_dir_all(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut last_err = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            clear_readonly_recursive(&p);
            match std::fs::remove_dir_all(&p) {
                Ok(()) => return Ok(()),
                Err(e) => last_err = e.to_string(),
            }
        }
        Err(format!("failed to remove {}: {}", p.display(), last_err))
    })
    .await
    .map_err(|e| format!("task join failed: {}", e))?
}

fn clear_readonly_recursive(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            clear_readonly_recursive(&p);
        } else if let Ok(meta) = std::fs::metadata(&p) {
            let mut perms = meta.permissions();
            if perms.readonly() {
                perms.set_readonly(false);
                let _ = std::fs::set_permissions(&p, perms);
            }
        }
    }
}

/// 把 clone 好的临时目录落位到正式目录：强制清空目标 → rename（带重试）→
/// 递归复制兜底。Windows 上目标残留、git 只读文件或句柄被占用都会让
/// rename 报 `Permission denied (os error 13)`，此流程逐一化解。
async fn move_dir_into_place(src: &Path, dst: &Path) -> Result<(), String> {
    force_remove_dir_all(dst).await?;
    if tokio::fs::rename(src, dst).await.is_ok() {
        return Ok(());
    }
    for _ in 0..3 {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        if tokio::fs::rename(src, dst).await.is_ok() {
            return Ok(());
        }
    }
    copy_dir_recursive(src, dst).await?;
    let _ = force_remove_dir_all(src).await;
    Ok(())
}

async fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    tokio::fs::create_dir_all(dst)
        .await
        .map_err(|e| format!("failed to create {}: {}", dst.display(), e))?;
    let mut rd = tokio::fs::read_dir(src)
        .await
        .map_err(|e| format!("failed to read {}: {}", src.display(), e))?;
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| format!("failed to read {}: {}", src.display(), e))?
    {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry
            .file_type()
            .await
            .map_err(|e| format!("failed to stat {}: {}", from.display(), e))?
            .is_dir()
        {
            Box::pin(copy_dir_recursive(&from, &to)).await?;
        } else {
            tokio::fs::copy(&from, &to)
                .await
                .map_err(|e| format!("failed to copy {}: {}", from.display(), e))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMarketplace {
    pub name: String,
    pub source: String,
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplacePlugin {
    pub name: String,
    /// 安装源：
    /// - 字符串 → marketplace 克隆内（或本地文件系统）的相对/绝对路径
    /// - 对象 `{source:'url'|'git-subdir', url, path?, ref?}` → 外部 git 仓库
    ///   （归一化后 url 存本字段，子目录/ref 存 source_path/source_ref）
    pub source: String,
    /// git 仓库内插件子目录（git-subdir 形态）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    /// git ref（branch/tag）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// 作者（字符串或 `{name}` 对象，对标 marketplace.json entry.author）
    #[serde(default)]
    pub author: String,
    /// 插件主页
    #[serde(default)]
    pub homepage: String,
}

fn is_remote_source(s: &str) -> bool {
    s.contains("://") || s.starts_with("git@")
}

/// marketplace.json 的 author 字段兼容字符串 / `{name}` 对象两种形态
fn author_label(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Object(o)) => o
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// 从 marketplace.json 的 source 字段归一化出 (url_or_path, path, ref)。
fn normalize_plugin_source(
    v: &serde_json::Value,
) -> Option<(String, Option<String>, Option<String>)> {
    match v {
        serde_json::Value::String(s) => Some((s.clone(), None, None)),
        serde_json::Value::Object(m) => {
            let url = m
                .get("url")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                return None;
            }
            Some((
                url,
                m.get("path")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                m.get("ref").and_then(|x| x.as_str()).map(|s| s.to_string()),
            ))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MarketplaceNameFile {
    #[serde(default)]
    name: Option<String>,
}

pub fn marketplaces_config_path(project_root: &Path) -> PathBuf {
    storage(project_root)
        .extensions_dir()
        .join("plugin-marketplaces.json")
}

pub fn marketplaces_dir(project_root: &Path) -> PathBuf {
    storage(project_root).extensions_dir().join("marketplaces")
}

pub fn marketplace_root(project_root: &Path, name: &str) -> PathBuf {
    marketplaces_dir(project_root).join(sanitize_marketplace_name(name))
}

fn sanitize_marketplace_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub async fn load_marketplaces(project_root: &Path) -> Result<Vec<PluginMarketplace>, String> {
    let path = marketplaces_config_path(project_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    #[derive(Deserialize, Default)]
    struct File {
        #[serde(default)]
        marketplaces: Vec<PluginMarketplace>,
    }
    serde_json::from_str::<File>(&text)
        .map(|f| f.marketplaces)
        .map_err(|e| format!("failed to parse {}: {}", path.display(), e))
}

async fn save_marketplaces(project_root: &Path, list: &[PluginMarketplace]) -> Result<(), String> {
    let path = marketplaces_config_path(project_root);
    ensure_parent(&path).await?;
    let text = serde_json::to_string_pretty(&serde_json::json!({ "marketplaces": list }))
        .map_err(|e| e.to_string())?;
    tokio::fs::write(&path, text)
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

/// 判断输入是否是本地路径（非 URL / git@ / owner-repo 缩写）
fn is_local_source(source: &str) -> bool {
    !source.contains("://") && !source.starts_with("git@") && !is_github_shorthand(source)
}

fn is_github_shorthand(source: &str) -> bool {
    // owner/repo 形式
    let parts: Vec<&str> = source.split('/').collect();
    parts.len() == 2
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && !source.contains(' ')
        && !source.contains('\\')
        && !source.starts_with('/')
        && !source.starts_with('.')
}

/// 添加 marketplace：git clone（或引用本地路径），并校验 marketplace.json。
pub async fn add_marketplace(
    project_root: &Path,
    source: &str,
) -> Result<PluginMarketplace, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("empty marketplace source".to_string());
    }

    let local_dir: PathBuf = if is_local_source(source) {
        let p = PathBuf::from(source);
        if !p.exists() {
            return Err(format!("path does not exist: {}", p.display()));
        }
        p.canonicalize().unwrap_or(p)
    } else {
        // git 源：先克隆到临时名（marketplace.json 的 name 确定最终名）
        let tmp_name = format!(
            "tmp-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );
        let dst = marketplaces_dir(project_root).join(&tmp_name);
        ensure_parent(&dst).await?;
        let _ = force_remove_dir_all(&dst).await;

        let clone = tokio::time::timeout(
            std::time::Duration::from_secs(180),
            tokio::process::Command::new("git")
                .arg("clone")
                .arg("--depth")
                .arg("1")
                .arg(mirror_github_url(source))
                .arg(&dst)
                .output(),
        )
        .await
        .map_err(|_| format!("git clone timed out after 180s: {}", source))?
        .map_err(|e| format!("failed to run `git clone`: {}", e))?;
        if !clone.status.success() {
            let _ = force_remove_dir_all(&dst).await;
            return Err(format!(
                "git clone failed: {}",
                summarize_process_output(&clone)
            ));
        }
        dst
    };

    // 解析 marketplace.json
    let mf_path = local_dir.join(".claude-plugin").join("marketplace.json");
    if !mf_path.exists() {
        if !is_local_source(source) {
            let _ = force_remove_dir_all(&local_dir).await;
        }
        return Err(format!(
            "not a plugin marketplace: missing {}",
            mf_path.display()
        ));
    }
    let text = tokio::fs::read_to_string(&mf_path)
        .await
        .map_err(|e| format!("failed to read marketplace.json: {}", e))?;
    // 只取 name（plugins 的 source 形态多样，在 list 阶段再解析）
    let name = serde_json::from_str::<MarketplaceNameFile>(&text)
        .ok()
        .and_then(|mf| mf.name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            local_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "marketplace".to_string())
        });

    // git 源：挪到正式目录名
    let final_root = if is_local_source(source) {
        local_dir.clone()
    } else {
        let final_root = marketplace_root(project_root, &name);
        ensure_parent(&final_root).await?;
        move_dir_into_place(&local_dir, &final_root).await?;
        final_root
    };
    let _ = final_root;

    let mut list = load_marketplaces(project_root).await?;
    list.retain(|m| m.name != name);
    let entry = PluginMarketplace {
        name,
        source: source.to_string(),
        added_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    };
    list.push(entry.clone());
    save_marketplaces(project_root, &list).await?;
    Ok(entry)
}

/// 移除 marketplace（同时删除其克隆目录，若在管理目录内）。
pub async fn remove_marketplace(project_root: &Path, name: &str) -> Result<bool, String> {
    let mut list = load_marketplaces(project_root).await?;
    let before = list.len();
    list.retain(|m| m.name != name);
    if list.len() == before {
        return Ok(false);
    }
    save_marketplaces(project_root, &list).await?;

    let cloned = marketplace_root(project_root, name);
    if cloned.exists() && cloned.starts_with(marketplaces_dir(project_root)) {
        let _ = force_remove_dir_all(&cloned).await;
    }
    Ok(true)
}

/// 读取某个 marketplace 的可安装插件列表。
///
/// 兼容官方 marketplace 的三种 source 形态：
/// 1. 字符串相对路径 `./plugins/x` → 解析为 marketplace 根下绝对路径（本地安装）
/// 2. `{source:'git-subdir', url, path, ref}` → url + 子目录 + ref
/// 3. `{source:'url', url, sha}` → url（整仓库即插件）
pub async fn list_marketplace_plugins(
    project_root: &Path,
    marketplace: &PluginMarketplace,
) -> Result<Vec<MarketplacePlugin>, String> {
    let root = if is_local_source(&marketplace.source) {
        PathBuf::from(&marketplace.source)
    } else {
        marketplace_root(project_root, &marketplace.name)
    };
    let mf_path = root.join(".claude-plugin").join("marketplace.json");
    if !mf_path.exists() {
        return Err(format!(
            "marketplace '{}' is missing {}",
            marketplace.name,
            mf_path.display()
        ));
    }
    let text = tokio::fs::read_to_string(&mf_path)
        .await
        .map_err(|e| format!("failed to read marketplace.json: {}", e))?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse marketplace.json: {}", e))?;

    let mut out = Vec::new();
    if let Some(items) = v.get("plugins").and_then(|p| p.as_array()) {
        for pv in items {
            let name = pv
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if name.trim().is_empty() {
                continue;
            }
            let Some((source, path, git_ref)) = pv.get("source").and_then(normalize_plugin_source)
            else {
                continue;
            };
            // 字符串形态的本地路径 → marketplace 根下绝对路径
            let source = if path.is_none() && !is_remote_source(&source) {
                let p = PathBuf::from(&source);
                if p.is_absolute() {
                    source
                } else {
                    root.join(p).to_string_lossy().to_string()
                }
            } else {
                source
            };
            out.push(MarketplacePlugin {
                name,
                source,
                source_path: path,
                source_ref: git_ref,
                description: pv
                    .get("description")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                version: pv
                    .get("version")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                author: author_label(pv.get("author")),
                homepage: pv
                    .get("homepage")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Ok(out)
}

/// 统一安装入口：按归一化后的 source 形态分流到 local / git / git+子目录。
/// `scope`: "project"（默认）或 "user"（对标 Claude Code 安装范围）。
pub async fn install_marketplace_plugin(
    project_root: &Path,
    plugin: &MarketplacePlugin,
    scope: &str,
) -> Result<PluginEntry, String> {
    match (&plugin.source_path, is_remote_source(&plugin.source)) {
        // 1. 本地路径
        (None, false) => {
            super::install_plugin_local(
                project_root,
                Path::new(&plugin.source),
                &plugin.name,
                scope,
            )
            .await
        }
        // 2. 整 git 仓库
        (None, true) => {
            super::install_plugin_git(
                project_root,
                &plugin.source,
                &plugin.name,
                plugin.source_ref.as_deref(),
                scope,
            )
            .await
        }
        // 3. git 仓库子目录（或本地目录内子目录）：clone 到临时目录后 local 安装
        (Some(sub), remote) => {
            let tmp = if remote {
                let tmp = marketplaces_dir(project_root).join(format!(
                    "tmp-plugin-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                ));
                let _ = force_remove_dir_all(&tmp).await;
                ensure_parent(&tmp).await?;

                let mut cmd = tokio::process::Command::new("git");
                cmd.args(["clone", "--quiet", "--depth", "1"]);
                if let Some(r) = plugin
                    .source_ref
                    .as_deref()
                    .filter(|r| !r.trim().is_empty())
                {
                    cmd.args(["--branch", r]);
                }
                cmd.arg(mirror_github_url(&plugin.source)).arg(&tmp);
                let out = tokio::time::timeout(std::time::Duration::from_secs(180), cmd.output())
                    .await
                    .map_err(|_| format!("git clone timed out after 180s: {}", plugin.source))?
                    .map_err(|e| format!("failed to run `git clone`: {}", e))?;
                if !out.status.success() {
                    let _ = force_remove_dir_all(&tmp).await;
                    return Err(format!(
                        "git clone failed ({}): {}",
                        plugin.source,
                        summarize_process_output(&out)
                    ));
                }
                Some(tmp)
            } else {
                None
            };

            let base = match &tmp {
                Some(t) => t.clone(),
                None => PathBuf::from(&plugin.source),
            };
            let sub_src = base.join(sub.trim_start_matches("./"));
            let result =
                super::install_plugin_local(project_root, &sub_src, &plugin.name, scope).await;
            if let Some(t) = tmp {
                let _ = force_remove_dir_all(&t).await;
            }
            result
        }
    }
}

/// 对标 Claude Code 的启动检查（officialMarketplaceStartupCheck）：
/// 若默认官方 marketplace 尚未注册，则自动添加（git clone）。
///
/// - 调用方（UI 后台任务）仅在缺失时触发，因此每次打开弹窗都会重试，
///   不会因单次网络失败被永久跳过
/// - `STARCODE_DISABLE_DEFAULT_MARKETPLACE_AUTOINSTALL=1` 可关闭
/// - 返回 `Ok(Some(name))` 表示本次注册成功
pub async fn ensure_default_marketplace(project_root: &Path) -> Result<Option<String>, String> {
    if let Ok(v) = std::env::var("STARCODE_DISABLE_DEFAULT_MARKETPLACE_AUTOINSTALL") {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            return Ok(None);
        }
    }

    let list = load_marketplaces(project_root).await?;
    if list.iter().any(|m| m.name == DEFAULT_MARKETPLACE_NAME) {
        return Ok(None);
    }

    // 对标 Claude Code：GCS 镜像优先（~3.5MB zip，不依赖 git/GitHub），
    // 失败才回落 git clone
    match fetch_official_marketplace_from_gcs(project_root).await {
        Ok(sha_opt) => {
            register_marketplace_config(
                project_root,
                DEFAULT_MARKETPLACE_NAME,
                DEFAULT_MARKETPLACE_SOURCE,
            )
            .await?;
            return Ok(Some(match sha_opt {
                Some(sha) => format!("{}@{}", DEFAULT_MARKETPLACE_NAME, &sha[..7.min(sha.len())]),
                None => DEFAULT_MARKETPLACE_NAME.to_string(),
            }));
        }
        Err(e) => {
            tracing_debug(&format!("GCS fetch failed, falling back to git: {}", e));
        }
    }

    let added = add_marketplace(project_root, DEFAULT_MARKETPLACE_SOURCE).await?;
    Ok(Some(added.name))
}

fn tracing_debug(msg: &str) {
    crate::utils::logging::append_debug_log_line(&format!("[marketplace] {}", msg));
}

pub const GCS_BASE: &str =
    "https://downloads.claude.ai/claude-code-releases/plugins/claude-plugins-official";
/// zip 内条目的种子目录前缀（对标 ARC_PREFIX）
pub const GCS_ARC_PREFIX: &str = "marketplaces/claude-plugins-official/";

/// GCS 快速通道（精准对标 officialMarketplaceGcs.ts）：
/// 1. GET `{base}/latest` → SHA 指针（约 40 字节，可每次调用）
/// 2. 安装目录 `.gcs-sha` 哨兵与 SHA 相同 → 已是最新，跳过下载
/// 3. 下载 `{sha}.zip` → 解压到 staging（剥离种子目录前缀 + zip-slip 防护）
///    → 原子落位
///
/// 返回 `Ok(Some(sha))` 表示下载并更新了内容；`Ok(None)` 表示已是最新。
pub async fn fetch_official_marketplace_from_gcs(
    project_root: &Path,
) -> Result<Option<String>, String> {
    use std::io::Read;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("http client: {}", e))?;

    // 1. latest 指针
    let sha = client
        .get(format!("{}/latest", GCS_BASE))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("latest pointer: {}", e))?
        .error_for_status()
        .map_err(|e| format!("latest pointer: {}", e))?
        .text()
        .await
        .map_err(|e| format!("latest body: {}", e))?;
    let sha = sha.trim().to_string();
    if sha.is_empty() {
        return Err("latest pointer returned empty body".to_string());
    }

    let cache_root = marketplaces_dir(project_root);
    let install_root = marketplace_root(project_root, DEFAULT_MARKETPLACE_NAME);
    // 纵深防御：安装位置必须在 marketplaces 缓存目录内
    if !install_root.starts_with(&cache_root) {
        return Err(format!(
            "refusing install location outside cache dir: {}",
            install_root.display()
        ));
    }

    // 2. 哨兵检查
    if let Ok(cur) = tokio::fs::read_to_string(install_root.join(".gcs-sha")).await {
        if cur.trim() == sha {
            return Ok(None);
        }
    }

    // 3. 下载 + 解压到 staging
    let resp = client
        .get(format!("{}/{}.zip", GCS_BASE, sha))
        .send()
        .await
        .map_err(|e| format!("zip download: {}", e))?
        .error_for_status()
        .map_err(|e| format!("zip download: {}", e))?;
    let bytes = resp.bytes().await.map_err(|e| format!("zip read: {}", e))?;

    let staging = cache_root.join(format!("{}.staging", DEFAULT_MARKETPLACE_NAME));
    force_remove_dir_all(&staging).await?;

    // 解压是纯同步 CPU/FS 工作（zip 的 Read 非 Send），放 blocking 线程
    let staging2 = staging.clone();
    let bytes2 = bytes.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes2[..]))
            .map_err(|e| format!("zip parse: {}", e))?;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("zip entry {}: {}", i, e))?;
            let name = entry.name().to_string();
            if !name.starts_with(GCS_ARC_PREFIX) {
                continue;
            }
            let rel = name[GCS_ARC_PREFIX.len()..].to_string();
            if rel.is_empty() || rel.ends_with('/') {
                continue;
            }
            let dest = staging2.join(&rel);
            // zip-slip 防护：解析后的目标必须仍在 staging 内
            if !dest.starts_with(&staging2) {
                return Err(format!("zip-slip entry rejected: {}", name));
            }
            if entry.is_dir() {
                std::fs::create_dir_all(&dest)
                    .map_err(|e| format!("mkdir {}: {}", dest.display(), e))?;
                continue;
            }
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("mkdir {}: {}", p.display(), e))?;
            }
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("read {}: {}", name, e))?;
            std::fs::write(&dest, buf).map_err(|e| format!("write {}: {}", dest.display(), e))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("extract task: {}", e))??;

    tokio::fs::write(staging.join(".gcs-sha"), &sha)
        .await
        .map_err(|e| format!("write sentinel: {}", e))?;

    // 原子落位（staging → install_root，含只读清理与复制兜底）
    move_dir_into_place(&staging, &install_root).await?;
    Ok(Some(sha))
}

/// 仅写入 marketplace 注册信息到配置（不下载内容）。GCS 快速通道用：
/// 内容已就位，只差配置条目。
async fn register_marketplace_config(
    project_root: &Path,
    name: &str,
    source: &str,
) -> Result<(), String> {
    let mut list = load_marketplaces(project_root).await?;
    if list.iter().any(|m| m.name == name) {
        return Ok(());
    }
    list.push(PluginMarketplace {
        name: name.to_string(),
        source: source.to_string(),
        added_at: Utc::now().timestamp(),
    });
    save_marketplaces(project_root, &list).await
}

/// 更新（刷新）已注册 marketplace 的内容（对标 ManageMarketplaces 的
/// update 动作）：
/// - 官方源：走 GCS 重新比对 SHA 指针，有新版本才下载
/// - 其他源：删除克隆目录并重新 shallow clone
pub async fn update_marketplace(project_root: &Path, name: &str) -> Result<String, String> {
    let list = load_marketplaces(project_root).await?;
    let Some(m) = list.iter().find(|m| m.name == name) else {
        return Err(format!("marketplace not found: {}", name));
    };

    if name == DEFAULT_MARKETPLACE_NAME {
        match fetch_official_marketplace_from_gcs(project_root).await {
            Ok(Some(sha)) => return Ok(format!("Updated {} ({})", name, &sha[..7.min(sha.len())])),
            Ok(None) => return Ok(format!("{} already up to date", name)),
            Err(e) => {
                tracing_debug(&format!("GCS update failed, falling back to git: {}", e));
            }
        }
    }

    let cloned = marketplace_root(project_root, name);
    if cloned.starts_with(marketplaces_dir(project_root)) {
        force_remove_dir_all(&cloned).await?;
    }
    // 复用 add_marketplace 的克隆 + 校验逻辑；配置条目已存在会按 name 去重
    add_marketplace(project_root, &m.source).await?;
    Ok(format!("Updated {}", name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 复现 Windows 上的 `Permission denied (os error 13)`：
    /// 目标目录残留（含只读文件）时 move 仍应成功。
    #[tokio::test]
    async fn test_move_dir_into_place_with_readonly_leftover() {
        let base = std::env::temp_dir().join(format!(
            "star_mp_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let src = base.join("src");
        let dst = base.join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(dst.join(".git")).unwrap();
        let ro = dst.join(".git").join("packed.obj");
        std::fs::write(&ro, b"x").unwrap();
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&ro, perms).unwrap();
        // 残留的 dst 存在且含只读文件：直接 rename 会 error 13
        std::fs::write(src.join("marketplace.json"), b"{}").unwrap();

        move_dir_into_place(&src, &dst)
            .await
            .expect("move should succeed");

        assert!(dst.join("marketplace.json").exists());
        assert!(!src.exists(), "src should be moved away");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 强制删除能清掉只读文件（普通 remove_dir_all 在 Windows 上会失败）。
    #[tokio::test]
    async fn test_force_remove_dir_all_clears_readonly() {
        let dir = std::env::temp_dir().join(format!(
            "star_mp_ro_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let ro = dir.join("a.obj");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&ro, b"x").unwrap();
        let mut perms = std::fs::metadata(&ro).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&ro, perms).unwrap();

        force_remove_dir_all(&dir)
            .await
            .expect("remove should succeed");
        assert!(!dir.exists());
    }
}
