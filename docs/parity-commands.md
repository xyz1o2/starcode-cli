# Slash Commands 对标分析

> 基于 `src/commands/system.rs` + `src/commands/parity.rs` (2,400+ 行)

## 状态: ✅ 全部 Pending 命令已实现

`system.rs` 中有测试断言不存在 `category: "Pending"` 的命令。

---

## 已实现的对标的命令

### 诊断 / 系统

| 命令 | 功能 | 对标 CCB |
|------|------|----------|
| `/env` | 环境 / 运行时诊断 | ✅ |
| `/release-notes` | 版本信息 + 提交记录 | ✅ |
| `/statusline` | 状态栏信息 / 切换 | ✅ |
| `/heapdump` | 进程资源快照 | ✅ |
| `/monitor` | 进程监控快照 | ✅ |
| `/debug-tool-call` | 工具调用检查 | ✅ |

### 输入 / 历史

| 命令 | 功能 | 对标 CCB |
|------|------|----------|
| `/history` | 输入历史浏览 / 搜索 | ✅ |
| `/attach` | 文件附件 | ✅ |
| `/tag` | 会话标签 | ✅ |
| `/keybindings` | 快捷键帮助覆盖层 | ✅ |

### 模式 / 行为

| 命令 | 功能 | 对标 CCB |
|------|------|----------|
| `/mode` | 权限模式切换 | ✅ |
| `/output-style` | 输出风格选择 | ✅ |
| `/poor` | 省电模式 | ✅ |
| `/proactive` | 主动建议开关 | ✅ |
| `/advisor` | 顾问模式 | ✅ |
| `/autonomy` | 自动继续面板 | ✅ |
| `/tui` | TUI 增强开关 | ✅ |
| `/network` | 离线/在线切换 | ✅ |

### 插件 / 集成

| 命令 | 功能 | 对标 CCB |
|------|------|----------|
| `/reload-plugins` | 插件重扫描 | ✅ |
| `/issue` | GitHub issue 管理 (via `gh`) | ✅ |
| `/subscribe-pr` | PR 订阅跟踪 | ✅ |
| `/install-github-app` | GitHub 集成状态 | ✅ |
| `/install-slack-app` | Slack 集成状态 | ✅ |

### Swarm / 多 Agent 通信

| 命令 | 功能 | 对标 CCB |
|------|------|----------|
| `/peers` | 对等节点列表 | ✅ |
| `/send` | 对等消息发送 | ✅ |
| `/claim-main` | 主实例声明 | ✅ |
| `/bridge-kick` | 对等关闭请求 | ✅ |
| `/remote-control` | 远程控制路径 | ✅ |
| `/mobile` | 移动端控制 | ✅ |
| `/desktop` | 桌面端控制 | ✅ |

### 任务 / 后台

| 命令 | 功能 | 对标 CCB |
|------|------|----------|
| `/goal` | 目标跟踪 (项目级持久化) | ✅ |
| `/job` | 后台任务队列 | ✅ |
| `/daemon` | 守护循环状态 | ✅ |
| `/coordinator` | 协调器状态 | ✅ |
| `/insights` | 会话使用洞察 | ✅ |

---

## 注意事项

1. **命令声明位置**: `src/commands/system.rs` — `ALL_COMMANDS` 数组
2. **实现位置**: `src/commands/parity.rs`
3. **dispatch 入口**: `src/commands/mod.rs` — `handle_command` match
4. **新增命令四步**: system.rs 声明 → 模块实现 → mod.rs dispatch → 切换 category
5. **category 字符串是 load-bearing**: `format_help()` 只打印硬编码的 category 列表, 未列出的 category 会隐藏命令
