//! 外部 CLI 配置兼容层：直接读 Claude Code 与 Codex 的配置，让 starcode
//! 「拿来就能用」，不必把同一套 endpoint / key / 模型再抄一遍到 starcode 里。
//!
//! # 为什么要读文件而不只是读环境变量
//!
//! `provider_resolution` 早就认 `ANTHROPIC_MODEL` / `ANTHROPIC_BASE_URL` /
//! `ANTHROPIC_API_KEY` 三个进程环境变量。问题是绝大多数人**不在环境变量里配**：
//!
//! - Claude Code 把配置写在 `~/.claude/settings.json` 的顶层 `model` 与 `env` 块里，
//!   **启动时**才把 `env` 注入进程。所以同一个人开 Claude Code 一切正常，
//!   开 starcode 却「什么都没配」—— 两个进程的环境本来就不共享。
//! - Codex 更彻底：模型与端点在 `~/.codex/config.toml`，key 在 `~/.codex/auth.json`，
//!   根本不存在对应的环境变量。
//!
//! 读文件把这条断链补上。写进 starcode 自己的 `~/.star/user-settings.json`
//! 当然仍然可行，但那要求用户知道两个工具各叫什么配置名 —— 兼容层就是为了免掉这一步。
//!
//! # 各读什么
//!
//! Claude Code `settings.json`：
//! - 顶层 `model`，其次 `env.ANTHROPIC_MODEL`。**只认顶层会让一部分用户整份配置
//!   都被丢掉** —— `adopted()` 要求有模型才采纳，而把模型写在 `env` 块里很常见；
//!   顶层值可能还是档位别名（`sonnet` / `opus[1m]`），要按 `env.ANTHROPIC_DEFAULT_*_MODEL`
//!   展开成真实模型名；
//! - `env.ANTHROPIC_BASE_URL`；
//! - `env.ANTHROPIC_AUTH_TOKEN`。**注意是 AUTH_TOKEN 不是 API_KEY** —— CC 接第三方
//!   网关时写的就是 `ANTHROPIC_AUTH_TOKEN`，只认 `API_KEY` 的实现读不到，这正是
//!   `api_key` 同时收两个键的原因；
//! - `env.ANTHROPIC_SMALL_FAST_MODEL`，以及 `env.ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`。
//!
//! Codex `config.toml`：
//! - `model` / `model_provider`；
//! - `[model_providers.<id>]` 的 `base_url` / `env_key` / `wire_api`；
//! - `~/.codex/auth.json` 的 `OPENAI_API_KEY`。
//!
//! # 层级合并
//!
//! Claude Code 的 settings 分三层，后者覆盖前者，`env` 块按键合并
//! （与 CC 自身一致）：用户级 → 项目级 `.claude/settings.json` → 本地
//! `.claude/settings.local.json`。用户级目录认 `CLAUDE_CONFIG_DIR`，
//! Codex 的目录认 `CODEX_HOME` —— 两个工具都支持改目录，不认这两个变量就会
//! 在自定义安装的用户身上读到「不存在」。
//!
//! # 为什么不直接 `std::env::set_var` 把它们塞进环境
//!
//! 与 `core::self_tuning` 里那条禁令同源：tokio 是多线程的，工具侧写进程环境
//! 会和其他线程的 `getenv` 竞争。这里只**读**，解析结果通过
//! [`ExternalConfigs`] 显式传下去，见 `provider_resolution`。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::core::config::json_with_comments::parse_json_with_comments;

/// Claude Code 用户级配置目录的环境变量（Claude Code 自己认这个）。
const ENV_CLAUDE_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";
/// Codex 的配置根目录环境变量。
const ENV_CODEX_HOME: &str = "CODEX_HOME";

const CLAUDE_SETTINGS_FILE: &str = "settings.json";
const CLAUDE_LOCAL_SETTINGS_FILE: &str = "settings.local.json";
const CLAUDE_PROJECT_DIR: &str = ".claude";
const CODEX_CONFIG_FILE: &str = "config.toml";
const CODEX_AUTH_FILE: &str = "auth.json";

/// 兼容层合成的 provider id（见 `providers.rs` 的同名内建条目）。
///
/// 它不是「另一个 Anthropic」：端点和密钥都来自 Claude Code 自己的配置，用户
/// 在 starcode 里不必再抄一遍，也不会被写回 `~/.star/user-settings.json`。
pub const CLAUDE_CODE_PROVIDER_ID: &str = "claude-code";

const CLAUDE_ENV_MODEL: &str = "ANTHROPIC_MODEL";
const CLAUDE_ENV_BASE_URL: &str = "ANTHROPIC_BASE_URL";
const CLAUDE_ENV_AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";
const CLAUDE_ENV_API_KEY: &str = "ANTHROPIC_API_KEY";
const CLAUDE_ENV_OPUS: &str = "ANTHROPIC_DEFAULT_OPUS_MODEL";
const CLAUDE_ENV_SONNET: &str = "ANTHROPIC_DEFAULT_SONNET_MODEL";
const CLAUDE_ENV_HAIKU: &str = "ANTHROPIC_DEFAULT_HAIKU_MODEL";
/// CC 用来指定「小而快」副模型（标题生成、分类这类旁路调用）的键。
const CLAUDE_ENV_SMALL_FAST_MODEL: &str = "ANTHROPIC_SMALL_FAST_MODEL";

/// 从 Claude Code 读到的、可以直接复用的配置。
///
/// 字段之间是**配套**的：`base_url` 与 `api_key` 必须来自同一个网关才有意义，
/// 所以它们和 `model` 一起从同一份 settings.json 里取，不会一半来自 CC、
/// 一半来自别处。见 [`ExternalConfigs::adopted`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeCodeConfig {
    /// 生效优先级最高的那份 settings.json 的路径（诊断信息用）。
    pub source_path: PathBuf,
    /// 顶层 `model`（档位别名会按 `env.ANTHROPIC_DEFAULT_*_MODEL` 展开），
    /// 其次 `env.ANTHROPIC_MODEL`。
    pub model: Option<String>,
    /// `env.ANTHROPIC_BASE_URL`。
    pub base_url: Option<String>,
    /// `env.ANTHROPIC_AUTH_TOKEN`，其次 `env.ANTHROPIC_API_KEY`。
    pub api_key: Option<String>,
    /// `env.ANTHROPIC_SMALL_FAST_MODEL`，其次 `env.ANTHROPIC_DEFAULT_HAIKU_MODEL`。
    ///
    /// 退到 HAIKU 是因为不少人只设了 `DEFAULT_*` 那一组，而那一组里语义上
    /// 对应「轻量档」的就是 HAIKU。
    pub small_fast_model: Option<String>,
    /// `env.ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL` 三档，全量保留。
    pub tier_models: ClaudeTierModels,
}

/// Claude Code 的 opus / sonnet / haiku 档位模型。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeTierModels {
    pub opus: Option<String>,
    pub sonnet: Option<String>,
    pub haiku: Option<String>,
}

/// 从 Codex 读到的、可以直接复用的配置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexConfig {
    /// `config.toml` 的路径（诊断信息用）。
    pub source_path: PathBuf,
    /// `auth.json` 的路径；文件不存在时为 `None`。
    pub auth_path: Option<PathBuf>,
    /// 顶层 `model`。
    pub model: Option<String>,
    /// 顶层 `model_provider`，指向 `[model_providers.<id>]` 里的哪一条。
    pub provider_id: Option<String>,
    /// `[model_providers.<provider_id>].base_url`。
    pub base_url: Option<String>,
    /// `[model_providers.<provider_id>].env_key` —— Codex 用它指明 key 放在哪个
    /// 环境变量里。我们不读那个环境变量（Codex 未必把它导出来了），保留下来
    /// 是为了能告诉用户「你的 key 该放哪」。
    pub env_key: Option<String>,
    /// `[model_providers.<provider_id>].wire_api`（`responses` / `chat`）。
    pub wire_api: Option<String>,
    /// `auth.json` 的 `OPENAI_API_KEY`。
    pub api_key: Option<String>,
}

/// 外部 CLI 配置的汇总。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalConfigs {
    pub claude: Option<ClaudeCodeConfig>,
    pub codex: Option<CodexConfig>,
}

/// 外部配置来自哪个工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalTool {
    ClaudeCode,
    Codex,
}

impl ExternalTool {
    /// `provider_resolution` 里 SourceRef 的前缀，与该工具的配置名保持一致，
    /// 这样 `/provider doctor` 打印出来的 `Model source: claude_code.model`
    /// 一眼就知道该去翻哪个文件。
    pub fn source_prefix(self) -> &'static str {
        match self {
            ExternalTool::ClaudeCode => "claude_code",
            ExternalTool::Codex => "codex",
        }
    }

    /// 被采纳的配置该落成哪个内建 provider id。
    ///
    /// Claude Code 有对应的内建条目（`providers.rs`），采纳它的配置就该把归属
    /// 记到它头上 —— 否则模型、端点、密钥全是 CC 的，provider 一栏却写着用户
    /// 自己配的另一个 gateway，两边对不上。
    ///
    /// Codex 暂时返回 `None`：它的 `wire_api` 还没映射到本项目的协议推断，
    /// 造一个路由不到正确协议的 id 只会更糟。
    pub fn provider_id(self) -> Option<&'static str> {
        match self {
            ExternalTool::ClaudeCode => Some(CLAUDE_CODE_PROVIDER_ID),
            ExternalTool::Codex => None,
        }
    }
}

impl ExternalConfigs {
    /// 采纳哪一个工具的「配置套件」。
    ///
    /// 只挑一个，而且要求它**配了模型**：端点和 key 是给模型用的，孤零零一个
    /// `base_url` 说明不了它要配谁。两个工具都配了模型时 Claude Code 优先 ——
    /// 它是本项目明确对标的那个（见 `CLAUDE.md`），配置也更完整。
    ///
    /// 被采纳的工具缺哪个字段，就让 `provider_resolution` 继续往下找，
    /// **不会**拿另一个工具的字段来凑 —— 那会拼出一个两边都不认的组合
    /// （A 工具的模型 + B 工具的网关）。
    pub fn adopted(&self) -> Option<AdoptedExternalProfile<'_>> {
        if let Some(claude) = self.claude.as_ref() {
            if claude.model.is_some() {
                return Some(AdoptedExternalProfile {
                    tool: ExternalTool::ClaudeCode,
                    model: claude.model.as_deref(),
                    base_url: claude.base_url.as_deref(),
                    api_key: claude.api_key.as_deref(),
                    small_fast_model: claude.small_fast_model.as_deref(),
                    source_path: Some(claude.source_path.as_path()),
                });
            }
        }

        if let Some(codex) = self.codex.as_ref() {
            if codex.model.is_some() {
                return Some(AdoptedExternalProfile {
                    tool: ExternalTool::Codex,
                    model: codex.model.as_deref(),
                    base_url: codex.base_url.as_deref(),
                    api_key: codex.api_key.as_deref(),
                    // Codex 的 config.toml 里没有「小而快的副模型」这个概念。
                    small_fast_model: None,
                    source_path: Some(codex.source_path.as_path()),
                });
            }
        }

        None
    }
}

/// 被采纳的那份外部配置。字段为 `None` 表示「这个工具没配」，由调用方继续往下解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdoptedExternalProfile<'a> {
    pub tool: ExternalTool,
    pub model: Option<&'a str>,
    pub base_url: Option<&'a str>,
    pub api_key: Option<&'a str>,
    pub small_fast_model: Option<&'a str>,
    pub source_path: Option<&'a Path>,
}

// ---------------------------------------------------------------------------
// 磁盘格式
// ---------------------------------------------------------------------------

/// Claude Code `settings.json` 里我们关心的部分。
///
/// `env` 的值声明成 `serde_json::Value` 而不是 `String`：单个值类型不对
/// （用户写了数字或对象）不应该让整份文件解析失败、连带把 `model` 也丢掉。
#[derive(Debug, Default, Deserialize)]
struct RawClaudeSettings {
    #[serde(default)]
    model: Option<serde_json::Value>,
    #[serde(default)]
    env: BTreeMap<String, serde_json::Value>,
}

impl RawClaudeSettings {
    /// 字符串字段：只接受字符串。CC 的字段都是字符串，但手改过的文件未必，
    /// 遇到数字/对象/数组一律当作「没配」，而不是 `to_string()` 出一串
    /// 用户从没写过的垃圾进 `SourceRef`。
    fn string_field(value: Option<&serde_json::Value>) -> Option<String> {
        value
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn env_string(&self, key: &str) -> Option<String> {
        Self::string_field(self.env.get(key))
    }

    /// `env` 块整体压过上一层（项目级 / 本地级覆盖用户级）。
    fn overlay(&mut self, higher: RawClaudeSettings) {
        if let Some(model) = higher.model {
            self.model = Some(model);
        }
        self.env.extend(higher.env);
    }
}

/// Codex `config.toml` 里我们关心的部分。
#[derive(Debug, Default, Deserialize)]
struct RawCodexConfig {
    #[serde(default)]
    model: Option<String>,
    #[serde(default, rename = "model_provider")]
    model_provider: Option<String>,
    #[serde(default, rename = "model_providers")]
    model_providers: HashMap<String, RawCodexProvider>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCodexProvider {
    #[serde(default, rename = "base_url")]
    base_url: Option<String>,
    #[serde(default, rename = "env_key")]
    env_key: Option<String>,
    #[serde(default, rename = "wire_api")]
    wire_api: Option<String>,
}

/// Codex `auth.json` 里我们关心的部分。
#[derive(Debug, Default, Deserialize)]
struct RawCodexAuth {
    #[serde(default, rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
}

// ---------------------------------------------------------------------------
// 读取
// ---------------------------------------------------------------------------

/// 进程级缓存：一次运行只需要读一次这几个小文件，而 `provider_resolution`
/// 会被 `main`、`/provider doctor`、子 agent 各调一次。
static CACHED: OnceLock<ExternalConfigs> = OnceLock::new();

/// 读当前进程可见的外部 CLI 配置（结果缓存）。
pub fn load() -> &'static ExternalConfigs {
    CACHED.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        load_with_overrides(
            dirs::home_dir().as_deref(),
            &cwd,
            ConfigDirOverrides::from_env(),
        )
    })
}

/// 按给定的目录读，不走缓存。测试与显式指定根目录时用这个。
pub fn load_from_dirs(home: Option<&Path>, project_dir: &Path) -> ExternalConfigs {
    load_with_overrides(home, project_dir, ConfigDirOverrides::from_env())
}

/// 两个工具各自的自定义配置目录（`CLAUDE_CONFIG_DIR` / `CODEX_HOME`）。
///
/// 单独拎出来不是为了好看：这两个值来自进程环境，而 `std::env::set_var` 在并行
/// 的单测里会跨用例串值——同一进程、同一份环境表。做成参数后，重定向的用例只碰
/// 自己的临时目录，一个进程环境变量都不用动。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConfigDirOverrides {
    claude_config_dir: Option<PathBuf>,
    codex_home: Option<PathBuf>,
}

impl ConfigDirOverrides {
    fn from_env() -> Self {
        Self {
            claude_config_dir: non_blank_dir(std::env::var(ENV_CLAUDE_CONFIG_DIR).ok()),
            codex_home: non_blank_dir(std::env::var(ENV_CODEX_HOME).ok()),
        }
    }
}

/// 空串 / 纯空白视为「没设」。
fn non_blank_dir(value: Option<String>) -> Option<PathBuf> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn load_with_overrides(
    home: Option<&Path>,
    project_dir: &Path,
    overrides: ConfigDirOverrides,
) -> ExternalConfigs {
    ExternalConfigs {
        claude: load_claude_code(home, project_dir, overrides.claude_config_dir.as_deref()),
        codex: load_codex(home, overrides.codex_home.as_deref()),
    }
}

fn load_claude_code(
    home: Option<&Path>,
    project_dir: &Path,
    config_dir_override: Option<&Path>,
) -> Option<ClaudeCodeConfig> {
    let mut merged = RawClaudeSettings::default();
    let mut source_path: Option<PathBuf> = None;

    // 层级顺序 = 覆盖顺序：用户级 → 项目级 → 本地级。
    let mut layers: Vec<PathBuf> = Vec::new();
    if let Some(path) = claude_user_settings_path(home, config_dir_override) {
        layers.push(path);
    }
    for relative in [
        format!("{CLAUDE_PROJECT_DIR}/{CLAUDE_SETTINGS_FILE}"),
        format!("{CLAUDE_PROJECT_DIR}/{CLAUDE_LOCAL_SETTINGS_FILE}"),
    ] {
        if let Some(path) = find_project_settings(project_dir, &relative) {
            layers.push(path);
        }
    }

    for path in layers {
        let Some(raw) = read_claude_settings(&path) else {
            continue;
        };
        merged.overlay(raw);
        // 记最高优先级那份：诊断时用户要照着改的是它。
        source_path = Some(path);
    }

    let source_path = source_path?;
    let tier_models = ClaudeTierModels {
        opus: merged.env_string(CLAUDE_ENV_OPUS),
        sonnet: merged.env_string(CLAUDE_ENV_SONNET),
        haiku: merged.env_string(CLAUDE_ENV_HAIKU),
    };
    Some(ClaudeCodeConfig {
        source_path,
        // 顶层 `model` 可能是档位别名（`sonnet` / `opus[1m]`），CC 自己会按
        // `env.ANTHROPIC_DEFAULT_*_MODEL` 展开；照抄别名会把一个 CC 内部记号
        // 当成模型名发给网关。展开不了就原样保留 —— 用户写的字总比空着强。
        model: RawClaudeSettings::string_field(merged.model.as_ref())
            .map(|model| expand_tier_alias(&model, &tier_models))
            // 模型只写在 `env.ANTHROPIC_MODEL` 里也算数：`adopted()` 要求有模型才
            // 采纳整份配置，漏掉这个键会连 base_url / token 一起被丢掉。
            .or_else(|| merged.env_string(CLAUDE_ENV_MODEL)),
        base_url: merged.env_string(CLAUDE_ENV_BASE_URL),
        // AUTH_TOKEN 在前：接第三方网关时 CC 用的就是它，而同一场景下
        // ANTHROPIC_API_KEY 往往还留着 Anthropic 官方的旧值。
        api_key: merged
            .env_string(CLAUDE_ENV_AUTH_TOKEN)
            .or_else(|| merged.env_string(CLAUDE_ENV_API_KEY)),
        small_fast_model: merged
            .env_string(CLAUDE_ENV_SMALL_FAST_MODEL)
            .or_else(|| merged.env_string(CLAUDE_ENV_HAIKU)),
        tier_models,
    })
}

/// 把 `sonnet` / `opus[1m]` / `haiku` 这类档位别名展开成
/// `env.ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL` 里的真实模型名。
///
/// `[1m]` 是 CC 用来标记「百万上下文变体」的后缀，展开成档位模型时一并去掉 ——
/// 它是 CC 自己的记号，网关不认。
fn expand_tier_alias(model: &str, tiers: &ClaudeTierModels) -> String {
    let lowered = model.trim().to_ascii_lowercase();
    let base = lowered.strip_suffix("[1m]").unwrap_or(&lowered).trim();
    let tier = match base {
        "opus" => tiers.opus.as_deref(),
        "sonnet" => tiers.sonnet.as_deref(),
        "haiku" => tiers.haiku.as_deref(),
        _ => None,
    };
    tier.map(str::trim)
        .filter(|tier| !tier.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| model.trim().to_string())
}

fn claude_user_settings_path(
    home: Option<&Path>,
    config_dir_override: Option<&Path>,
) -> Option<PathBuf> {
    // `CLAUDE_CONFIG_DIR`（经 override 传进来）是 Claude Code 自己认的变量：
    // 自定义安装位置的用户不会把配置放在 `~/.claude` 下。不认它 = 对这部分
    // 用户完全读不到。
    let dir = match config_dir_override {
        Some(dir) => dir.to_path_buf(),
        None => home?.join(CLAUDE_PROJECT_DIR),
    };
    let path = dir.join(CLAUDE_SETTINGS_FILE);
    path.exists().then_some(path)
}

fn read_claude_settings(path: &Path) -> Option<RawClaudeSettings> {
    let content = std::fs::read_to_string(path).ok()?;
    // 走 `parse_json_with_comments`：CC 的 settings 是给人手改的，留注释很常见，
    // 而 starcode 自己的 settings 解析器已经处理过这件事，直接复用。
    parse_json_with_comments::<RawClaudeSettings>(&content).ok()
}

/// 从 `start` 向上找**项目级** settings：到最近的 `.git` 为止，且不看 home 目录本身。
///
/// 这里没有复用 `core::utils::paths::find_project_file_upwards`：它在整条祖先链上
/// 都没有 `.git` 时会一路走到 `$HOME` 再收手，于是 `~/.claude/settings.json` 会被
/// 当成**项目级**配置再读一遍。它和用户级是同一个文件，值上碰巧一样 —— 但那属于
/// 碰巧。层级覆盖顺序一旦依赖它，`CLAUDE_CONFIG_DIR` 指到别处、或 home 下换个布局
/// 时就会静默按错的优先级解析。
fn find_project_settings(start: &Path, relative: &str) -> Option<PathBuf> {
    let home = dirs::home_dir();
    let git_root = start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf);

    for dir in start.ancestors() {
        // home 里的 `.claude/` 是用户级，已经单独读过了。
        if Some(dir) == home.as_deref() {
            return None;
        }
        let path = dir.join(relative);
        if path.exists() {
            return Some(path);
        }
        if git_root.as_deref() == Some(dir) {
            break;
        }
    }

    None
}

fn load_codex(home: Option<&Path>, home_override: Option<&Path>) -> Option<CodexConfig> {
    let dir = match home_override {
        Some(dir) => dir.to_path_buf(),
        None => home?.join(".codex"),
    };
    let source_path = dir.join(CODEX_CONFIG_FILE);
    let raw: RawCodexConfig = std::fs::read_to_string(&source_path)
        .ok()
        .and_then(|content| toml::from_str(&content).ok())?;

    // `model_provider` 没写时 Codex 用内置的 openai。这里不去 provider 表里
    // 找同名的条目：用户完全可能自建过一个叫 openai 的自定义网关，那会拿错端点。
    let provider = raw
        .model_provider
        .as_deref()
        .and_then(|id| raw.model_providers.get(id));

    let auth_path = dir.join(CODEX_AUTH_FILE);
    let auth_exists = auth_path.exists();
    let api_key = if auth_exists {
        read_codex_api_key(&auth_path)
    } else {
        None
    };

    Some(CodexConfig {
        source_path,
        auth_path: auth_exists.then_some(auth_path),
        model: raw.model,
        provider_id: raw.model_provider,
        base_url: provider.and_then(|provider| provider.base_url.clone()),
        env_key: provider.and_then(|provider| provider.env_key.clone()),
        wire_api: provider.and_then(|provider| provider.wire_api.clone()),
        api_key,
    })
}

fn read_codex_api_key(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let from_struct = serde_json::from_str::<RawCodexAuth>(&content)
        .ok()
        .and_then(|auth| auth.openai_api_key);
    if let Some(key) = from_struct.filter(|key| !key.trim().is_empty()) {
        return Some(key.trim().to_string());
    }
    // 回退：直接按键取。auth.json 里除了 key 还有一坨 token 结构，那部分一旦
    // 变形，反序列化整体就失败了，但 `OPENAI_API_KEY` 往往还是好的。
    serde_json::from_str::<serde_json::Value>(&content)
        .ok()
        .and_then(|value| {
            value
                .get("OPENAI_API_KEY")
                .and_then(|key| key.as_str())
                .map(|key| key.trim().to_string())
        })
        .filter(|key| !key.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 所有用例都走 `load_with_overrides`，因此**一个进程环境变量都不用动** ——
    /// `std::env::set_var` 在并行单测里会跨用例串值，而这些用例恰好都在改
    /// `CLAUDE_CONFIG_DIR` / `CODEX_HOME`。
    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, content).expect("write");
    }

    /// 空覆盖 = 「两个工具都装在默认位置」。
    fn load_default(home: &Path, project_dir: &Path) -> ExternalConfigs {
        load_with_overrides(Some(home), project_dir, ConfigDirOverrides::default())
    }

    #[test]
    fn reads_model_base_url_and_token_from_claude_settings() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".claude").join("settings.json"),
            r#"{
              "model": "claude-sonnet-4-5",
              "env": {
                "ANTHROPIC_BASE_URL": "https://gateway.example.com",
                "ANTHROPIC_AUTH_TOKEN": "cc-token",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "claude-haiku-4-5"
              }
            }"#,
        );

        let configs = load_default(home, home);
        let claude = configs.claude.expect("claude config");

        // 用户在 Claude Code 里配了什么，starcode 就该拿到什么 —— 这是整个
        // 兼容层存在的理由。
        assert_eq!(claude.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(
            claude.base_url.as_deref(),
            Some("https://gateway.example.com")
        );
        // AUTH_TOKEN 才是 CC 接第三方网关时用的键，只认 API_KEY 的实现读不到。
        assert_eq!(claude.api_key.as_deref(), Some("cc-token"));
        // SMALL_FAST_MODEL 没写时退到 DEFAULT_HAIKU。
        assert_eq!(claude.small_fast_model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(
            claude.tier_models.haiku.as_deref(),
            Some("claude-haiku-4-5")
        );
    }

    #[test]
    fn a_model_written_only_in_the_env_block_is_still_adopted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        // 只把模型写在 `env` 里是很常见的配法。`adopted()` 要求有模型才采纳整份
        // 配置，只认顶层 `model` 会把 base_url / AUTH_TOKEN 一起丢掉 —— 用户明明
        // 在 CC 里配好了，starcode 却「什么都没配」。
        write(
            &home.join(".claude").join("settings.json"),
            r#"{
              "env": {
                "ANTHROPIC_MODEL": "group/auto-deepseek-v4-1-flash-free",
                "ANTHROPIC_BASE_URL": "http://127.0.0.1:3425",
                "ANTHROPIC_AUTH_TOKEN": "gateway-token"
              }
            }"#,
        );

        let configs = load_default(home, home);
        let claude = configs.claude.as_ref().expect("claude config");
        assert_eq!(
            claude.model.as_deref(),
            Some("group/auto-deepseek-v4-1-flash-free")
        );

        let adopted = configs.adopted().expect("整份配置都该被采纳");
        assert_eq!(adopted.tool, ExternalTool::ClaudeCode);
        assert_eq!(adopted.base_url, Some("http://127.0.0.1:3425"));
        assert_eq!(adopted.api_key, Some("gateway-token"));
    }

    #[test]
    fn a_tier_alias_is_expanded_to_the_configured_model() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        // `sonnet[1m]` 是 CC 自己的档位记号，不是模型名；网关不认。
        write(
            &home.join(".claude").join("settings.json"),
            r#"{
              "model": "sonnet[1m]",
              "env": {
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-sonnet-4-5-20250929"
              }
            }"#,
        );

        let configs = load_default(home, home);
        let claude = configs.claude.expect("claude config");
        assert_eq!(claude.model.as_deref(), Some("claude-sonnet-4-5-20250929"));
    }

    #[test]
    fn an_unexpandable_alias_is_kept_verbatim() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".claude").join("settings.json"),
            r#"{ "model": "opus" }"#,
        );

        let configs = load_default(home, home);
        let claude = configs.claude.expect("claude config");
        // 没有对应档位变量时别把它换成空串：用户写的字总比空着强。
        assert_eq!(claude.model.as_deref(), Some("opus"));
    }

    #[test]
    fn small_fast_model_beats_the_haiku_tier() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".claude").join("settings.json"),
            r#"{
              "env": {
                "ANTHROPIC_SMALL_FAST_MODEL": "claude-haiku-4-5-fast",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "claude-haiku-4-5"
              }
            }"#,
        );

        let configs = load_default(home, home);
        let claude = configs.claude.expect("claude config");
        assert_eq!(
            claude.small_fast_model.as_deref(),
            Some("claude-haiku-4-5-fast")
        );
    }

    #[test]
    fn auth_token_wins_over_a_stale_api_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".claude").join("settings.json"),
            r#"{
              "env": {
                "ANTHROPIC_AUTH_TOKEN": "gateway-token",
                "ANTHROPIC_API_KEY": "stale-official-key"
              }
            }"#,
        );

        let configs = load_default(home, home);
        let claude = configs.claude.expect("claude config");
        assert_eq!(claude.api_key.as_deref(), Some("gateway-token"));
    }

    #[test]
    fn project_settings_override_user_settings_and_env_merges_by_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        let project = temp.path().join("repo");
        fs::create_dir_all(&project).expect("mkdir");

        write(
            &home.join(".claude").join("settings.json"),
            r#"{
              "model": "claude-sonnet-4-5",
              "env": {
                "ANTHROPIC_BASE_URL": "https://user.example.com",
                "ANTHROPIC_AUTH_TOKEN": "user-token"
              }
            }"#,
        );
        write(
            &project.join(".claude").join("settings.json"),
            r#"{
              "model": "claude-opus-4-5",
              "env": { "ANTHROPIC_BASE_URL": "https://project.example.com" }
            }"#,
        );

        let configs = load_default(home, &project);
        let claude = configs.claude.expect("claude config");

        // 标量字段：项目级整条覆盖用户级。
        assert_eq!(claude.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(
            claude.base_url.as_deref(),
            Some("https://project.example.com")
        );
        // env 块按键覆盖而非整块替换 —— 否则项目级只想改个模型，
        // 顺手把用户的 token 也抹掉了。
        assert_eq!(claude.api_key.as_deref(), Some("user-token"));
    }

    #[test]
    fn settings_local_wins_over_the_shared_project_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        let project = temp.path().join("repo");
        write(
            &project.join(".claude").join("settings.json"),
            r#"{ "model": "shared" }"#,
        );
        write(
            &project.join(".claude").join("settings.local.json"),
            r#"{ "model": "local" }"#,
        );

        let configs = load_default(home, &project);
        let claude = configs.claude.expect("claude config");
        assert_eq!(claude.model.as_deref(), Some("local"));
    }

    #[test]
    fn a_non_string_env_value_does_not_take_the_model_down_with_it() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        // 手改过的文件里出现数字值很常见。不因为它把整个文件丢掉 ——
        // 否则顶层 model 也一起没了，比丢掉那一个 env 键糟得多。
        write(
            &home.join(".claude").join("settings.json"),
            r#"{
              "model": "claude-sonnet-4-5",
              "env": { "ANTHROPIC_BASE_URL": 12345 }
            }"#,
        );

        let configs = load_default(home, home);
        let claude = configs.claude.expect("claude config");
        assert_eq!(claude.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(claude.base_url, None);
    }

    #[test]
    fn comments_in_claude_settings_are_tolerated() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".claude").join("settings.json"),
            r#"{
              // 自定义网关
              "model": "claude-sonnet-4-5",
              "env": { "ANTHROPIC_BASE_URL": "https://gateway.example.com" }
            }"#,
        );

        let configs = load_default(home, home);
        let claude = configs.claude.expect("claude config");
        assert_eq!(claude.model.as_deref(), Some("claude-sonnet-4-5"));
    }

    #[test]
    fn reads_model_provider_and_key_from_codex() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".codex").join("config.toml"),
            r#"
model = "gpt-5-codex"
model_provider = "my-gateway"

[model_providers.my-gateway]
name = "My Gateway"
base_url = "https://codex.example.com/v1"
env_key = "MY_GATEWAY_KEY"
wire_api = "responses"
"#,
        );
        write(
            &home.join(".codex").join("auth.json"),
            r#"{ "OPENAI_API_KEY": "codex-key", "last_refresh": "2026-01-01" }"#,
        );

        let configs = load_default(home, home);
        let codex = configs.codex.expect("codex config");

        assert_eq!(codex.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(codex.provider_id.as_deref(), Some("my-gateway"));
        assert_eq!(
            codex.base_url.as_deref(),
            Some("https://codex.example.com/v1")
        );
        assert_eq!(codex.env_key.as_deref(), Some("MY_GATEWAY_KEY"));
        assert_eq!(codex.wire_api.as_deref(), Some("responses"));
        assert_eq!(codex.api_key.as_deref(), Some("codex-key"));
    }

    #[test]
    fn codex_key_survives_a_broken_sibling_field() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".codex").join("config.toml"),
            "model = \"gpt-5-codex\"\n",
        );
        // `tokens` 的结构跟反序列化里写的不一样 —— 那是 Codex 自己的字段，
        // 它变一次我们不该连带把 OPENAI_API_KEY 弄丢。
        write(
            &home.join(".codex").join("auth.json"),
            r#"{ "OPENAI_API_KEY": "codex-key", "tokens": "unexpected-shape" }"#,
        );

        let configs = load_default(home, home);
        let codex = configs.codex.expect("codex config");
        assert_eq!(codex.api_key.as_deref(), Some("codex-key"));
    }

    #[test]
    fn missing_config_files_are_not_an_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();

        let configs = load_default(home, home);
        assert_eq!(configs, ExternalConfigs::default());
        assert_eq!(configs.adopted(), None);
    }

    #[test]
    fn claude_profile_is_adopted_when_both_tools_configure_a_model() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".claude").join("settings.json"),
            r#"{ "model": "claude-sonnet-4-5",
                 "env": { "ANTHROPIC_BASE_URL": "https://cc.example.com",
                          "ANTHROPIC_AUTH_TOKEN": "cc-token" } }"#,
        );
        write(
            &home.join(".codex").join("config.toml"),
            "model = \"gpt-5-codex\"\n\n[model_providers.openai]\nbase_url = \"https://codex.example.com\"\n",
        );

        let configs = load_default(home, home);
        let adopted = configs.adopted().expect("adopted profile");

        // 端点与 key 必须跟着模型来自同一个工具。
        assert_eq!(adopted.tool, ExternalTool::ClaudeCode);
        assert_eq!(adopted.model, Some("claude-sonnet-4-5"));
        assert_eq!(adopted.base_url, Some("https://cc.example.com"));
        assert_eq!(adopted.api_key, Some("cc-token"));
    }

    #[test]
    fn a_config_without_a_model_is_not_adopted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        // 只有端点、没有模型 —— 采纳它等于把模型和网关的对应关系交给运气。
        write(
            &home.join(".claude").join("settings.json"),
            r#"{ "env": { "ANTHROPIC_BASE_URL": "https://cc.example.com" } }"#,
        );

        let configs = load_default(home, home);
        assert_eq!(configs.adopted(), None);
        // 文件本身仍然要读得到，`/provider doctor` 要报它存在。
        assert!(configs.claude.is_some());
    }

    #[test]
    fn claude_profile_never_borrows_the_codex_gateway() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".claude").join("settings.json"),
            r#"{ "model": "claude-sonnet-4-5" }"#,
        );
        write(
            &home.join(".codex").join("config.toml"),
            "model = \"gpt-5-codex\"\n\n[model_providers.openai]\nbase_url = \"https://codex.example.com\"\n",
        );

        let configs = load_default(home, home);
        let adopted = configs.adopted().expect("adopted profile");
        assert_eq!(adopted.tool, ExternalTool::ClaudeCode);
        assert_eq!(adopted.model, Some("claude-sonnet-4-5"));
        // Claude 那份没配端点，就该留空让 `provider_resolution` 往下找，
        // 而不是拿 Codex 的网关拼出一个两边都不认的组合。
        assert_eq!(adopted.base_url, None);
        assert_eq!(adopted.api_key, None);
    }

    #[test]
    fn codex_profile_is_adopted_when_claude_configures_no_model() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path();
        write(
            &home.join(".codex").join("config.toml"),
            "model = \"gpt-5-codex\"\n\n[model_providers.my-gateway]\nbase_url = \"https://codex.example.com/v1\"\n",
        );
        write(
            &home.join(".codex").join("auth.json"),
            r#"{ "OPENAI_API_KEY": "codex-key" }"#,
        );

        let configs = load_default(home, home);
        let adopted = configs.adopted().expect("adopted profile");
        assert_eq!(adopted.tool, ExternalTool::Codex);
        assert_eq!(adopted.model, Some("gpt-5-codex"));
        assert_eq!(adopted.api_key, Some("codex-key"));
        assert_eq!(adopted.small_fast_model, None);
    }

    #[test]
    fn claude_config_dir_override_redirects_the_user_level_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let custom = temp.path().join("custom-claude");
        write(
            &custom.join("settings.json"),
            r#"{ "model": "from-custom-dir" }"#,
        );

        // home 里放一份会被误读的，确认自定义目录真的优先。
        let decoy_home = temp.path().join("home");
        write(
            &decoy_home.join(".claude").join("settings.json"),
            r#"{ "model": "from-home" }"#,
        );

        let configs = load_with_overrides(
            Some(&decoy_home),
            temp.path(),
            ConfigDirOverrides {
                claude_config_dir: Some(custom),
                codex_home: None,
            },
        );
        let claude = configs.claude.expect("claude config");
        assert_eq!(claude.model.as_deref(), Some("from-custom-dir"));
    }

    #[test]
    fn codex_home_override_redirects_the_config_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let custom = temp.path().join("custom-codex");
        write(&custom.join("config.toml"), "model = \"from-custom-dir\"\n");

        let decoy_home = temp.path().join("home");
        write(
            &decoy_home.join(".codex").join("config.toml"),
            "model = \"from-home\"\n",
        );

        let configs = load_with_overrides(
            Some(&decoy_home),
            temp.path(),
            ConfigDirOverrides {
                claude_config_dir: None,
                codex_home: Some(custom),
            },
        );
        let codex = configs.codex.expect("codex config");
        assert_eq!(codex.model.as_deref(), Some("from-custom-dir"));
    }

    /// `CLAUDE_CONFIG_DIR` / `CODEX_HOME` 空串常见于 CI 里
    /// `CLAUDE_CONFIG_DIR=` 这种写法。若把它当真，配置根目录就变成 `""`
    ///（即当前目录），于是读到一份凭空捏造的位置，还带着「配置存在」的假象。
    #[test]
    fn a_blank_dir_override_is_treated_as_unset() {
        assert_eq!(non_blank_dir(None), None);
        assert_eq!(non_blank_dir(Some(String::new())), None);
        assert_eq!(non_blank_dir(Some("   \t ".to_string())), None);
        assert_eq!(
            non_blank_dir(Some("  /custom/dir  ".to_string())),
            Some(PathBuf::from("/custom/dir"))
        );
    }
}
