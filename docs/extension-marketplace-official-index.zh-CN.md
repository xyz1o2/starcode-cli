# `/extension` 面板看不到 exa —— marketplace 只读本地内置清单

- 日期：2026-11-05
- 现象：`/extension` 打开「Extension Marketplace」，**11 项**，没有 exa；
  但 `~/.claude.json` 里 `mcpServers.exa` 已经配好，模型也能通过
  `ToolSearch` 搜到 `mcp__exa__web_search_exa`。
- 结论：**不是 exa 没装，是 `/extension` 面板只显示本地内置清单，从不下载官方市场。**

---

## 1. 面板里那 11 项是什么

`src/core/extensions/marketplace.rs`：

| 来源 | 数量 | 内容 |
|---|---|---|
| `BUILTIN_SKILLS` | 6 | analyzer / editor / search / navigator / auto_fix / verify（内置 skill 模板） |
| `common_mcp` 硬编码 | 5 | filesystem / github / memory / brave-search / puppeteer |

两者都是**编译进二进制的常量**（`generate_default_index()`，
`marketplace.rs:106`）。截图里那 11 项正好对上。

## 2. 官方市场从来没被下载过

```rust
// marketplace.rs:42
const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/anthropics/claude-plugins-official/main/.claude-plugin/marketplace.json";
```

这个常量**零引用** —— 全仓库只有定义那一行。

`load_index()` 的逻辑是：

```rust
pub fn load_index(&self) -> MarketplaceIndex {
    if self.index_path.exists() { /* 读缓存 */ }
    self.generate_default_index()      // ← 永远走这里
}
```

缓存文件 `index_path` 没有**任何写入者**（`save_index` 存在，但没人调），
所以永远是内置清单。

## 3. 官方市场实际有 315 个插件，含 exa

实测拉取 `DEFAULT_INDEX_URL`：

```text
官方市场插件总数: 315
含 exa 的: ['exa']
```

exa 的定义：

```json
{
  "name": "exa",
  "description": "Exa AI web search, deep research, and content extraction...",
  "author": { "name": "Exa" },
  "category": "productivity",
  "source": {
    "source": "url",
    "url": "https://github.com/exa-labs/exa-mcp-server.git",
    "sha": "15ffb50519e719dc791cdc750ce5ed1934c0a1ed"
  },
  "homepage": "https://exa.ai/docs/reference/exa-mcp"
}
```

### 障碍：数据结构对不上

官方 `source` 有 4 种形态，StarCode 的 `MarketplaceEntry.source` 是**单个
String**：

| 官方形态 | 数量 | StarCode 能表达吗 |
|---|---|---|
| `{"source":"git-subdir","url":...,"path":...}` | 99 | ❌ path/ref/sha 装不下 |
| `{"source":"url","url":...,"sha":...}` | 163 | ❌ sha 装不下 |
| `"./plugins/xxx"`（仓库内路径） | 53 | ⚠️ 语义不同（相对官方仓库） |

顶层字段也有出入：官方有 `displayName` / `category` / `keywords` /
`skills` / `lspServers` / `strict` / `version`，StarCode 的
`MarketplaceEntry` 只有 `name` / `version` / `description` / `author`（String）
/ `extension_type` / `source`（String）/ `tags` / `downloads` / `stars` /
`featured`。

所以**直接下载官方索引还不够** —— 得同时改数据模型与安装逻辑（`git-subdir`
要 clone 后取子目录，`url` 要带 sha 校验，仓库内路径要相对官方仓库解析）。

## 4. 你现在的 exa 是怎么工作的

`~/.claude.json` 的 `mcpServers.exa`：

```json
{ "type": "stdio", "command": "npx", "args": ["-y", "exa-mcp-server@latest"],
  "env": { "EXA_API_KEY": "..." } }
```

`load_effective_mcp_config()` 读这个文件（`config.rs:169`），
`initialize_mcp_servers()` 注册，`refresh_mcp_tools()` 把工具塞进 registry。
所以 exa **是装好的、能调的** —— 日志里能看到
`[MCP-stderr] npm notice run 'exa-mcp-server'`。

**`/extension` 面板与 MCP 配置是两套东西**：面板管的是「可安装的扩展」，
MCP 配置管的是「已连上的 server」。面板没 exa 不影响 exa 能用。

## 5. 你截图里那个「用不了」是什么

模型 `ToolSearch(max_results=5, query='exa web search')` 搜到了 5 个工具，
然后**没有调 exa**，而是去调 `mcp__chrome-devtools__navigate_page`，
而且参数编错（`pageId` 传成 `undefined`）。

这一段发生在 **Claude Code 里**（报错文案 `Input validation error` 与
`Unrecognized key` 不在 StarCode 源码里），是 Claude Code 的模型行为，
不是 StarCode 的问题。

不过 StarCode 侧确实有一个**同类缺陷**，本轮已修：插件的 MCP server 从来
没接进运行时（`discover_plugin_mcp_servers()` 与
`initialize_plugin_mcp_servers()` 都零调用者），且插件工具名带冒号
（`mcp__plugin:exa:exa__web_search_exa`，function name 不允许冒号）。
详见 commit `ecf28da` / `2149c59`。

## 6. 想在面板里看到 exa，该做什么

### 现状能做到的（不改代码）

MCP server 走**配置文件**，不走 marketplace：

```jsonc
// .star/mcp.json（项目级）或 ~/.star/mcp.json（全局级）
{
  "mcpServers": {
    "exa": {
      "command": "npx",
      "args": ["-y", "exa-mcp-server@latest"],
      "env": { "EXA_API_KEY": "你的 key" }
    }
  }
}
```

改完**重启会话**（MCP server 在启动时连接）。`/mcp` 里能看到连接状态。

### 要让面板列出官方 315 个插件（需要改代码）

1. **数据模型**：`MarketplaceEntry.source` 从 `String` 改成
   `MarketplaceSource` 枚举（`Builtin` / `Npm` / `GitUrl` / `GitSubdir` /
   `RepoPath`），并补 `displayName` / `category` / `keywords` 等字段。
2. **拉取**：`load_index()` 在本地缓存缺失时下载 `DEFAULT_INDEX_URL`，
   写进 `index_path`（`save_index` 已存在，只是没人调）；失败时回落到内置清单。
3. **安装**：`install_plugin` 支持 `git-subdir`（clone 后取子目录）与
   `url`（带 sha 校验）。
4. **插件的 MCP server**：装完后读它的 `.claude-plugin/plugin.json` 的
   `mcpServers` —— 这条链路本轮已接通（`ecf28da`），但**前提是插件按
   StarCode 的目录约定安装**（`.star-plugin/plugin.json`），而官方插件用的是
   `.claude-plugin/plugin.json`。这里还要加一条候选路径。

## 7. 验证

```bash
# 官方索引确实有 exa
curl -s https://raw.githubusercontent.com/anthropics/claude-plugins-official/main/.claude-plugin/marketplace.json \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(len(d['plugins']), [p['name'] for p in d['plugins'] if 'exa' in p['name']])"
```

输出：`315 ['exa']`

## 8. 本轮附带修掉的（已提交）

| 提交 | 内容 |
|---|---|
| `ecf28da` | 插件的 MCP server 接进运行时（`discover_plugin_mcp_servers` / `initialize_plugin_mcp_servers` 此前都零调用者） |
| `2149c59` | 插件工具名去掉冒号（`mcp__plugin_exa_exa__tool`），并加逆映射保证拆名能还原 |
| — | `/help` 浮层 ESC 关不掉（`KeyContext::Help` 与「吞掉所有键」的那批上下文混在一起，ESC 永远走不到关闭逻辑） |
