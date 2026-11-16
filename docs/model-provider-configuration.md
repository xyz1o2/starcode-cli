# 模型配置与 Claude Code 配置复用

本文说明当前实现支持的配置方式。模型 ID 应使用所连接服务实际提供的 ID。

## 主会话模型

在 TUI 输入框中执行：

```text
/model list
/model <model-id>
/model <known-provider-id>/<model-id>
/provider doctor
```

`/model` 不带参数打开模型选择器。带 provider 前缀时，该前缀必须是已知的内建或自定义 provider；不认识的前缀会作为完整模型 ID 保留。`/provider doctor` 可检查模型、地址、凭据的配置来源，凭据不会完整显示。

启动参数 `--model` 直接使用模型字符串，不经过 TUI `/model` 的 provider 前缀拆分。

## 不同团队成员使用不同模型

先确认当前 provider 可用的模型 ID，再创建自定义代理：

```text
/agents create scout --description "代码检索" --model "<fast-model-id>"
/agents create reviewer --description "代码审查" --model "<strong-model-id>"
/agents team run --agents scout,reviewer --parallelism 2 分析项目
```

修改已有定义或恢复模型继承：

```text
/agents edit reviewer --model "<model-id>"
/agents edit reviewer --clear-model
```

自定义代理的 `model` 是实际模型 ID，连接地址、凭据和 provider 继承其父级 client。该字段不会解析 provider 前缀或 `sonnet`、`haiku`、`inherit` 等特殊含义；如需继承，省略该字段或使用 `--clear-model`。

内置 Agent 工具的 Explore 当前没有独立模型配置入口，使用 runner 创建时保存的 client。配置自定义团队成员不会替换内置 Explore；`ANTHROPIC_DEFAULT_HAIKU_MODEL` 也不会自动把 Explore 分配给 Haiku。主模型运行时切换后，既存 runner 是否同步更新没有保证。

## 复用 Claude Code settings

启动时会读取并合并以下配置，后者优先；`env` 按键覆盖：

1. `~/.claude/settings.json`；设置 `CLAUDE_CONFIG_DIR` 时改读该目录下的 `settings.json`。
2. 项目 `.claude/settings.json`。
3. 项目 `.claude/settings.local.json`。

项目配置向上查找到最近的 Git 根目录。外部配置首次加载后缓存，修改后需要重启 StarCode。

例如，现有 Claude Code 配置可以采用以下结构；尖括号内容均需替换：

```json
{
  "model": "sonnet",
  "env": {
    "ANTHROPIC_BASE_URL": "https://<gateway-host>",
    "ANTHROPIC_AUTH_TOKEN": "<token>",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "<main-model-id>",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "<strong-model-id>",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "<fast-model-id>"
  }
}
```

当前兼容层读取模型和连接字段，不会将 Claude settings 的整个 `env` 注入进程。第三方服务须提供 Anthropic 兼容接口；变量名本身不会将 OpenAI Chat 接口转换为 Anthropic 接口。只有 Claude 配置能解析出模型时，才会采纳该连接配置；仅配置地址或凭据不够。

Claude settings 顶层 `model` 优先于该文件的 `env.ANTHROPIC_MODEL`。顶层 `sonnet`、`opus`、`haiku` 及其 `[1m]` 后缀可通过 `ANTHROPIC_DEFAULT_*_MODEL` 展开为实际模型 ID。`env.ANTHROPIC_MODEL`、CLI `--model`、TUI `/model` 和自定义代理的 `model` 不走这条档位展开路径，应填写实际 ID。

## 复用 shell 环境变量

现有进程环境中的这些变量会被读取：

- `ANTHROPIC_MODEL`
- `ANTHROPIC_BASE_URL`
- `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY`
- `ANTHROPIC_SMALL_FAST_MODEL`

仅在 shell 中导出 `ANTHROPIC_DEFAULT_OPUS_MODEL`、`ANTHROPIC_DEFAULT_SONNET_MODEL`、`ANTHROPIC_DEFAULT_HAIKU_MODEL` 不会启用上述文件配置中的档位展开。可直接将 `ANTHROPIC_MODEL` 设为实际模型 ID。

主模型解析时，session / CLI / `STAR_MODEL` / `ANTHROPIC_MODEL` / provider store 都优先于外部 Claude 配置。地址也会优先采用 CLI、`STAR_BASE_URL`、`ANTHROPIC_BASE_URL` 和选中 provider 的配置。凭据存在 provider 专属变量优先分支，不能笼统认为 `STAR_API_KEY` 总优先；Claude provider 的环境凭据优先取 `ANTHROPIC_AUTH_TOKEN`，再取 `ANTHROPIC_API_KEY`。

如果复用后结果与预期不同，先执行 `/provider doctor` 核对实际来源；不要仅根据某份 settings 文件判断最终配置。
