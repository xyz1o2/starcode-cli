# StarCode CLI — TUI 设计系统

> 基于 Ratatui + Crossterm 的 Rust 原生 AI 编程助手。对标 Claude Code 设计语言，融合 Rust 生态特色。

## 1. 设计原则

- **温暖而非冰冷** — 赤陶色主色调，拒绝企业蓝
- **内容优先** — 最小化 chrome，最大化代码可见性
- **即时反馈** — 流式渲染，~33ms 帧时间目标
- **可访问性** — 256 色回退，ASCII fallback，屏幕阅读器支持

## 2. 色彩系统

### 语义角色

| 角色 | 色值 | ANSI 256 | 用途 |
|------|------|----------|------|
| Background | 终端默认 | - | 深色背景 |
| Foreground | `#ffffff` | `15` | 默认文本、AI 响应 |
| **Primary** | `#d77757` | `173` | 赤陶色 — 品牌主色 |
| **Secondary** | `#fd5db1` | `206` | 热粉色 — 工具/Shell 边框 |
| **Accent** | `#b1b9f9` | `147` | 薰衣草色 — 权限对话框 |
| Success | `#4eba65` | `71` | 绿色 — 完成 |
| Warning | `#ffc107` | `220` | 琥珀色 — 警告 |
| Error | `#ff6b80` | `204` | 柔红 — 错误 |
| Muted | `#888888` | `245` | 灰色 — 输入边框、非活跃 |
| Surface | `#373737` | `237` | 用户消息背景 |

### 特殊色彩

| 名称 | 色值 | 用途 |
|------|------|------|
| Claude shimmer | `#eb9f7f` | 浅赤陶色 shimmer 动画 |
| Bash border | `#fd5db1` | 热粉色工具执行边框 |
| Permission | `#b1b9f9` | 薰衣草蓝权限对话框 |
| Auto-accept | `#af87ff` | 紫色 YOLO/自动接受模式 |
| Diff added bg | `#225c2b` | 新增行绿色底色 |
| Diff removed bg | `#7a2936` | 删除行红色底色 |

## 3. 布局系统

### 三区域垂直布局

```
┌─────────────────────────────────────────┐
│                                         │
│           Output Region                 │  ← flex-grow
│         (对话转录/工具输出)               │     Paragraph + wrap
│                                         │     自动滚动到底部
├─────────────────────────────────────────┤
│  cwd · provider model · session · ...   │  ← Status Bar (1行)
├─────────────────────────────────────────┤
│  > _                                    │  ← Input Area (2行)
└─────────────────────────────────────────┘
```

### 布局参数

| 参数 | 值 |
|------|-----|
| 最小终端宽度 | 80 |
| 理想终端宽度 | 120 |
| 工具块内边距 | 1 行上下，1 字符左右 |
| 消息间距 | 1 行 + 细分隔线 |
| 缩进级别 | 2 空格 |
| 状态栏高度 | 1 行 |
| 输入区高度 | 2 行（边框 + 文本） |

## 4. 组件规范

### 4.1 输入框（签名设计 — ASCII 虚线）

```
- - - - - - - - - - - - - - - - -
|  > your message here_          |
- - - - - - - - - - - - - - - - -
```

- **必须**使用 ASCII 虚线（`-` 横线，`|` 竖线）
- **禁止**使用 Unicode box-drawing（`┌─┐│└─┘`）
- 边框颜色：Muted 灰色，shimmer 在 `#888` 和 `#A6A6A6` 之间
- 背景：Surface `#373737`
- `>` 前缀

### 4.2 工具调用块（热粉色边框）

```
┌─ Bash ─────────────────────────┐
│ $ cargo test                    │
│                                 │
│ running 42 tests ...            │
│ test result: ok. 42 passed      │
└─────────────────────────────────┘
```

- 边框字符：`┌─┐│└─┘`（单线）
- 边框颜色：热粉色 `#fd5db1`
- 工具名 + 文件路径在边框标题
- 代码内容带语法高亮
- Bash 输出背景：`rgb(65,60,65)`

### 4.3 Diff 视图

```
┌─ Edit: src/app.rs ──────────────┐
│                                 │
│  - let old = get_value();       │
│  + let result = get_new_value();│
│  + log::info!("Updated");       │
│                                 │
└─────────────────────────────────┘
```

- 新增行：`+` 前缀，`#225c2b` 背景
- 删除行：`-` 前缀，`#7a2936` 背景
- 热粉色边框（同工具调用块）

### 4.4 权限对话框（薰衣草边框）

```
┌─ Allow Edit to src/app.rs? ────┐
│                                 │
│  [Y]es  [N]o  [A]lways         │
│                                 │
└─────────────────────────────────┘
```

- 边框颜色：薰衣草色 `#b1b9f9`
- 选项按键加粗高亮

### 4.5 Thinking 动画（签名特性）

```
  ✳ Percolating...
```

- 6 符号循环：`· ✢ ✳ ✶ ✻ ✽` 然后反转
- 120ms/帧
- 赤陶色 shimmer 到 `#eb9f7f`
- 配合随机动词（184 个选项）：
  - "Cogitating...", "Percolating...", "Moonwalking..."
  - "Shenaniganing...", "Ruminating...", "Pondering..."

### 4.6 状态栏

```
  Opus · 12.4K tokens · $0.04 · 3.2s · normal
```

- 固定底部行
- 模型名 · token 数 · 成本 · 耗时 · effort 级别
- Muted 灰色

### 4.7 消息分隔

- 消息间：细分隔线 `#505050`
- 无重型分隔 — 内容自然流动

## 5. 文字排版

### 文本层级

| 层级 | 样式 | 示例 |
|------|------|------|
| H1 | BOLD + Primary（赤陶色） | 会话标题 |
| Body | Foreground（白色） | AI 响应文本 |
| Code | 语法高亮 | 代码块 |
| User input | `>` 前缀 + Surface 背景 | 用户消息 |
| Caption | Muted + dim | token 数、时间戳 |
| Thinking | Primary（赤陶色）+ shimmer | Thinking 动词 |

### 图标系统

| 用途 | 图标 | ASCII 回退 |
|------|------|-----------|
| Success | `✓` | `+` |
| Error | `✗` | `x` |
| Warning | `⚠` | `!` |
| Thinking | `· ✢ ✳ ✶ ✻ ✽` | `*` |
| Prompt | `>` | `>` |
| Running | `▸` | `>` |
| Bullet | `•` | `-` |

## 6. 动画与运动

### Thinking Spinner

```
Frames: · → ✢ → ✳ → ✶ → ✻ → ✽ → ✻ → ✶ → ✳ → ✢ → ·
```

- 120ms/帧
- 赤陶色 + shimmer
- 反向镜像循环（上行再下行）
- 随机动词配对

### 输入框 Shimmer

- 虚线边框在 `#888888` 和 `#A6A6A6` 之间 shimmer
- 细微、柔和的动画

### 流式文本

- 无状态转换动画
- 流式文本按 API 接收顺序逐字符渲染
- 工具块以热粉色边框即时出现

## 7. 差异对比

| 元素 | Claude Code | StarCode CLI |
|------|-------------|--------------|
| 框架 | Ink (TypeScript) | Ratatui (Rust) |
| 渲染 | React 组件树 | 即时模式 Widget |
| 输入框 | ASCII 虚线 | ASCII 虚线 ✓ |
| 工具边框 | 热粉色 | 热粉色 ✓ |
| Thinking | 随机动词 | 随机动词 ✓ |
| 全屏模式 | alternate screen | alternate screen ✓ |
| 鼠标支持 | 可选 | 可选 |
| 内存 | ~常量（只渲染可见） | ~常量（只渲染可见） |

## 8. Do's and Don'ts

### Do

- 赤陶色 `#d77757` 作为主品牌色
- ASCII 虚线输入框 — 非 Unicode box-drawing
- 热粉色工具调用边框 — 视觉突出
- 随机动词 loading 状态 — 人格化
- 响应文本纯白色 — 可读性优先
- 流式渲染 — 即时反馈

### Don't

- 冷/企业蓝作为主色 — StarCode 是温暖的
- Unicode box-drawing 输入框 — 虚线是签名
- 过度边框化 — 大部分内容无边框流动
- 通用 "Loading..." — 随机动词是个性
- 着色 AI 响应正文 — 白色代表可信
- 阻塞渲染 — 流式优先
