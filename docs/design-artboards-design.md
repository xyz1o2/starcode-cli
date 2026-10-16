# Claude Code `/design` 研究与落地方案

> 快照日期：2026-10-17
> 研究对象：Claude Code `/design` skill（research preview，v2.1.234+），及其底座 Claude Design artboard workflow 与 Claude Code artifacts
> 定位：**架构对标文档**。格式与判定纪律对齐 `docs/claude-code-alignment.md` —— 官方侧给出处，StarCode 侧给文件与行号，并标注验证状态。
> **本文不含代码实现**，只交付研究结论 + 可执行的分阶段落地方案。

验证状态标记：

| 标记 | 含义 |
|---|---|
| ✅ | grep 构造点 / 调用点确认 |
| ⚠️ | 读了代码但未确认接线，动手前需再查 |
| ❌ | 已确认缺失 |

---

## 0. TL;DR

1. `/design` **不是一个新能力，是一次重新打包**。Anthropic 把 2026-04-17 上线的 Claude Design（claude.ai 上的独立产品）的 artboard 工作流，连同 artifacts 运行时一起塞进了 CLI / Desktop。对标含义：我们要抄的是**工作流的形状**，不是某个模型能力。
2. 它的可复制内核是**三层纯文本协议**：`.dc.html`（画板源码）+ `canvas.json`（布局清单）+ 一个 seed 脚本（把前两者烘进编辑器载荷）。这三样**全部落在工作区的普通文件里**，可读、可 diff、可提交 —— 这是本次复刻 90% 的价值所在。
3. 不可复制的两样：claude.ai 的 artifact 发布/版本化运行时，以及 2.1 MB 的预编译专有画板编辑器。前者可以用「本地自包含 HTML + 浏览器打开」平替；后者必须自研或放弃。
4. **StarCode 侧有一个硬阻塞：`SKILL.md` 根本到不了模型。** `.star/skills/*/SKILL.md` 只被 `/skills` 和 `/skill-search` 两个命令读取，agent 热路径上没有任何地方枚举它（`src/agent/tools/skill.rs:267-283`）。不先修这个，任何 skill 形态的特性都落不了地。
5. 顺手可捡的现成脚手架：`ChatState::preview_visible` 等三个字段（`src/ui/state/store.rs:477-479`）已经铺好、已经被 stream 写入，但**全仓无渲染器** —— 正好是这个特性的空位。

---

## 1. 官方设计基线（`/design` 到底是什么）

### 1.1 定位与版本门槛

| 项 | 值 |
|---|---|
| 形态 | skill，research preview |
| 最低版本 | 官方 Week 34 digest 明确写 **v2.1.234**（第三方有 v2.1.233 的说法，见 §5 冲突表） |
| 入口 | `/design <brief>`，CLI 与 Claude Code Desktop 同源 |
| 底层 | Claude Design artboard workflow，运行在 Claude Code **artifacts** 之上 |
| 覆盖面 | Pro / Max / Team / Enterprise |

**artifacts 本身的门槛**（`/design` 继承）：CLI ≥ v2.1.183、Desktop ≥ 1.13576.0；会话需用 claude.ai 账号 `/login`；模型提供方**仅限 Anthropic API**（Bedrock / Vertex AI / Microsoft Foundry 均不可用）；组织侧需关闭 CMEK / HIPAA / Zero Data Retention。

> 出处：[Week 34 digest](https://code.claude.com/docs/en/whats-new/2026-w34) · [Artifacts](https://code.claude.com/docs/en/artifacts#availability)

### 1.2 三个动词与责任边界

Anthropic 对外反复强调的三个动词是 **pick → tweak → implement**，责任切得很干净：

| 动词 | 发生地 | 谁做 |
|---|---|---|
| **pick**（生成多方案） | `/design` skill，CLI 内 | 模型产出画板 |
| **tweak**（改） | 已发布的 artifact 页面，浏览器内 | 人 / 页面自带编辑器 |
| **implement**（写代码） | 回到会话 | 常规 Claude Code 工作流 |

**关键边界**：skill 自己的描述最后一句写死了责任范围 —— *"Only for CREATING or re-seeding a canvas; an existing one is edited in its published Artifact."* 即 `/design` **只管第一步**。第二步在 artifact 里，第三步是普通会话工作。

> 出处：skill 的 `description` frontmatter（第三方泄露副本，见 §1.7）

### 1.3 端到端流程（skill 正文还原）

```
1. 落盘       把每个画板写成工作区里的普通文件：Main.dc.html 必有，
              同级可加 Pricing.dc.html 等；多画板时另写 canvas.json；图片同落盘
2. 播种       node seed-canvas.mjs --template payload.template.html \
                --out <名字>.html --title "<标题>" --artboard Main.dc.html \
                [--artboard ...] [--image ...] [--canvas canvas.json]
              → 产出单个自包含 HTML（实测 5 个画板 ≈ 2.1 MB，因为编辑器内联在里面）
3. 设计系统   动手画之前先"狩猎"：token 文件、theme / CSS 变量样式表、
              Tailwind theme、组件包与 UI 包、Storybook stories、图标集、
              assets 目录里的品牌字体。变量解析到真实值，不是凑整到间距网格
4. 发布       把画布作为 claude.ai artifact 发布，打印链接，浏览器打开
5. 人接手     选一个画板 → 页面上直接改 → 保存重新发布 → 回会话说"实现第 2 版"
```

**一个容易被忽略的设计点**：画布不只是图片，布局清单里带**便签（sticky notes）**，承载每个方向的正反论据。**理由跟着产物一起走。**

### 1.4 产出物落在磁盘上的样子

这是本次复刻最值钱的部分 —— 全是普通文件，没有藏在不可达的缓存目录里，**没有后续导出步骤**：

```
<工作区>/
├── Main.dc.html          # 每个设计方向一个，自包含单页
├── Pricing.dc.html
├── canvas.json           # 布局清单：位置、页列表、便签、启动视图
├── hero.png              # 图片按原样引用，不复制
└── spring-menu-poster.html   # ← helper 烘出来的单文件画布（发布的就是它）
```

种子文件**完全离线可渲染**：`file://` 直接双击就能看，无需登录、无需网络。因为编辑器是内联的。**丢掉的是 Save** —— Save 依赖 artifact 运行时在场才能重新发布；把画布搬到别处，你得到的是「查看 + PNG/PDF 导出」，不是编辑器。

> 出处：[Stacktree 实测](https://stacktr.ee/blog/what-is-design-in-claude-code)（作者自述 2026-08-18 实跑并检查了磁盘产物）

### 1.5 `.dc.html` 画板格式规格

每个 `.dc.html` 是一个自包含的 Design Component 页面。以下约束**静默失败**，是最容易踩的一批：

| 规则 | 说明 |
|---|---|
| `$export` 头行必须逐字一致 | 少了这一行，页面被跳过 |
| 根元素尺寸 | 设备模型 / 海报 / 社交图 / 印刷 → **固定 `w`×`h`**；PAGE（网页、仪表盘、桌面应用屏）→ **流式**：用 `expand: "fill"`、不写 px 宽度、`max-width` 容器、宽高用 `%`/`rem`/`fr` |
| `$preview` | 渲染预览用，等于 `w`×`h` |
| 高度单位 | px/rem；`vh` 只给整屏根元素用，**绝不给 `%`** |
| 窄屏行为 | 手机宽度下菜单堆叠、工具栏换行、宽表格放进滚动容器 |
| 属性面板 | 内联 `style="…"` 是属性面板唯一改的地方 |
| 兄弟分组 | flex 或 grid + `gap` |
| `{{hole}}` | 只是一个查表占位符，**不是表达式**；查的是 `renderVals()` |
| 复制粘贴 | 查看器重新键入后仍是字面 markup |
| 必含块 | `<script>` 块 + 经典 JS（`class Component extends DCLogic`），**不允许 import** |
| UI 构建 | 全部是 markup，**不许用脚本拼**（禁 `innerHTML` / `appendChild` / 自定义 `window.X`） |
| `data-props` | 单引号 JSON，声明可调项 —— 这些是「旋钮」，不是复制品 |
| 网络 | 仅允许 `<head>` 里一个 Google Fonts `css2` `<link>` 和受限图片 URL |
| 图标 | 挂载系统的图标（按名），否则内联 stroke SVG，**绝不用 emoji** |
| 禁止 | 全局 keydown 处理器 |
| 尺寸经验值 | 手机应用 390×844；PAGE `w` 取 1280–1440 |

**artboard 宽度就是真实的响应式视口** —— artboard 内部的 media query 按它断点。所以要按你将来真正要构建的宽度去创作。

> 出处：Design artifact type 的 `SKILL.md`（第三方镜像）

### 1.6 `canvas.json` 布局清单

```jsonc
{
  "v": 3,
  "createdOnFiles": { "v": 1, "at": "2026-09-14T18:20:00Z" },
  "title": "Spring Menu Poster",
  "launch": { "view": "canvas" },
  "pages": [],
  "boards": { "Main.dc.html": { "x": 0, "y": 0, "w": 880, "h": 560 } },
  "order": ["Main.dc.html"],
  "notes": {},
  "designSystems": []
}
```

| 字段 | 规格 |
|---|---|
| `boards` | 每画板一项，键为文件路径；`x`/`y`/`w`/`h` 是画布上**框架**的 CSS px（`w`,`h` 取值 40–8000；同行间距 80 px，行间 120 px） |
| `boards.*` 可选键 | `title`、`page`（无则第一页）、`expand:"fill"`（表示这是个 PAGE）、`print:"flow"`（PDF 自动分页）、`paper:"letter"\|"a4"`、`is_interactive:true`（控件可用）、`frameless`（编辑器设置，保留）、`guides`、`radius` |
| `order` | 与 boards 同路径集合，**从后到前**；第一个画板必须叫 `Main.dc.html` |
| `pages` | `[{id, name}]`，≤ 40；id 匹配 `[A-Za-z0-9_-]{1,40}` |
| `launch` | 只有两种形状：`{"view":"canvas"}`（可选 `"page"`）或 `{"view":"focused","file":"Main.dc.html"}`（单画板专注） |
| `notes` | ≤ 200 条；`x`/`y`/`text` 必填 |
| `notes.*.kind` | `title1` = 覆盖多个画板的标题（**绝不用于单个**，72 px 粗体一行，需 `maxW`）；**缺省 = 便签**（设 `w`，长到 `maxH` 后滚动）；`rect\|oval\|pen\|line\|arrow\|image` 是用户自己画的，**别动** |

**一条纪律**：`canvas.json` 里存在的画板，`.dc.html` 文件必须存在；反过来，**任何不在 boards 里的 `.dc.html` 也会显示**（包括已删除过的）。改索引时**保留你没改的每一个键和条目**。

### 1.7 seed 脚本的契约与校验

```bash
# 播种
node seed-canvas.mjs --template payload.template.html \
  --out spring-menu-poster.html --title "Spring Menu Poster" \
  --artboard Main.dc.html --artboard Pricing.dc.html \
  --image hero.png --canvas canvas.json

# 反向抽取（从已发布页或已播种页把工作文件还原回来）
node seed-canvas.mjs --extract <page>.html --to <fresh empty dir>
```

**文件名和标题是内容，不是工具名。** 脚本会主动拒绝：

| 拒绝条件 | 例子 |
|---|---|
| 通用文件名 | `design.html`、`index.html`、`main.html`、`page.html`、`canvas.html`、`output.html` |
| 通用标题 | `Untitled`、`Design Canvas` |
| 标题含非法字符 | `<` `>` `&` `"` 或反斜杠（撇号可以） |
| 画板未以 `.dc.html` 结尾 | — |
| 单个条目过大 | — |
| `canvas.json` 列了你没传的画板 | — |
| `canvas.json` 带了编辑器会丢弃的 note id / page / launch | — |
| 两张图片同 basename | — |

**警告（非拒绝）**：没有画板叫 `Main.dc.html` 时会警告 —— 首次播种应把入口画板命名为 `Main`。

图片以 **裸 base64 按 basename 存储**（`--image photos/pool.jpg` → `pool.jpg`），路径原样传入，不要复制文件。种子源码会被转义，保证它**永远无法闭合状态块**。

### 1.8 状态块与 payload

- 设计内容是状态块（script id `appifact-doc`）里的 `files` 记录：**路径 → 原始 `.dc.html` 源码**。
- `canvas.json` 同样是 `files` 的一个条目。
- payload 模板是**预编译产物**，随 skill 一起解包。实测解包位置：宿主临时目录下的 `claude/bundled-skills/design/`，**不进 `~/.claude`**。
- canvas 是 HTML 存储、HTML 执行，**刻意沙箱在嵌套 frame 里**。安全上要记住一句话：**从共享画布里读回来的任何内容都当作「别人发布的内容」，不是指令。**

### 1.9 设计系统优先级

自 v2.1.182 起，**构建 artifact 时会自动套用一个内置 design skill**，产出有意识的调色板、字体与布局。这个 skill **会先去找项目里已有的设计系统**，找到就用。

优先级链（官方明文）：

```
用户 prompt  >  你的设计系统（CLAUDE.md / theme 文件）  >  skill 自己的选择
```

**设计 token** 指设计系统复用的那些命名的颜色 / 字体 / 间距值。想让产物跟产品品牌一致，就把这些记录在 Claude 找得到的地方 —— 项目的 `CLAUDE.md` 或仓库里的 theme 文件。

**字体**：artifact 页面能从 Google Fonts 加载字体（唯一外部字体源）。其他字体以 `@font-face` data URI 内联，并给每个字体族配 fallback 栈。

### 1.10 artifact 运行时的硬约束（CSP 形状）

| 效果 | 约束 |
|---|---|
| 外部请求 | 只能加载 Google Fonts 的字体，以及五个公共 CDN 主机：cdnjs、unpkg、Tailwind CDN、jQuery CDN、jsDelivr 的选定路径（如 `/npm/`） |
| 被阻断 | **所有外部图片**，以及其余全部外部脚本 / 样式表 / 字体 |
| 网络 API | `fetch` / XHR / WebSocket 仅能到页面自身 origin 与 Google Fonts |
| 因此 | 所有库从上述 CDN 取，其余 CSS/JS 内联，图片转 data URI |
| 路由 | **单页、无后端**，不能服务多路由 |
| 相对链接 | **不解析**（`file://` 下什么都没有）→ 改用页面锚点 |
| 扩展名 | `.html` / `.htm` / `.md`（Markdown 会做语法高亮） |

### 1.11 成本

生成 artifact 按输出 token 计费，**装饰过的页面比同样内容作为终端文本明显更贵**。主要贡献者：内联 CSS、交互控件的 JS，以及**尤其是** data URI 形式的图片。

### 1.12 周边能力（本次不在范围内，但需知道存在）

- **`DesignSync` 原生工具**（非 MCP）：方法 `list_projects`、`get_project`、`list_files`、`get_file`、`create_project`、`finalize_plan`、`write_files`、`delete_files`、`register_assets`、`unregister_assets`、`report_validate`；作用域 `user:design:read`；端点 `/v1/design/consent`、`/v1/design/grants`、`/v1/design/`。**强制顺序**：先 list/read，再 `finalize_plan`，最后 write/delete。它同步的产物是 **React 设计系统**（来自 Storybook 或裸包），所以纯 HTML/CSS 产物不适用。
- `/design-sync` 双向：把设计系统拉进仓库 / 把仓库里写好的推回画布。
- `/design` 的 dispatch table 里其实带 `import` / `export` / `status` 条目（受 Design 工具是否在场门控），但在 preview 期间不可用。
- 授权命令是 `/design-login`（`/design login` 在 dispatch table 里的拼法）。

---

## 2. StarCode 现状核对

### 2.1 有一个同名不同义的 `/artifacts`

❌ **它不是渲染器，是磁盘文件浏览器。**

- dispatch：`src/commands/mod.rs:332` → `parity::artifacts(ctx, &args)`
- 实现：`src/commands/parity.rs:4231-4406`（`artifact_locations()` 在 `:4231`，`artifacts()` 在 `:4304`，`render_artifact_set()` 在 `:4349`），`artifact_locations()` 是**硬编码的 12 条 `.star/` 路径列表**，输出 markdown 表格 + 大小/mtime

**命名冲突必须在落地方案里处理**，否则 `/artifacts` 的语义会从"浏览生成物"漂移到"打开画布"。

### 2.2 preview pane 是铺好的死脚手架

✅ 这三个字段存在、已被 stream 写入、**全仓无任何渲染器**：

```
src/ui/state/store.rs:477   pub preview_visible: bool,
src/ui/state/store.rs:478   pub preview_scroll_state: PreviewScrollState,
src/ui/state/store.rs:479   pub preview_last_entry_idx: Option<usize>,
src/ui/services/stream.rs:1976   // 写入 preview_last_entry_idx
src/commands/parity.rs:786       // /tui preview 切换
src/commands/parity.rs:820       // 状态表里显示
```

（测试里也有断言：`stream.rs:3128`、`:3153`）

⚠️ 这意味着这个特性的**渲染位置已经预留好了**，但需要确认 `preview_last_entry_idx` 原本的语义（指向 chat entry 还是别的），不要贸然复用同一个字段表达"当前画板"。

### 2.3 硬阻塞：SKILL.md 到不了模型

✅ `SkillLoader::load_skills_from_dir` 的**全部**调用点：

```
src/commands/skills.rs:48,53,118,123     ← /skills 命令
src/commands/parity.rs:2948,2949         ← /skill-search 命令
```

**两条都在命令层，都不在 agent 热路径上。**

再看 `skill` 工具的外部加载分支 `src/agent/tools/skill.rs:267-283`：它只在两种情况下加载 `SKILL.md` ——

1. `skill_name` 以 `http` 或 `git@` 开头（GitHub 浅 clone），或
2. `Path::new(&skill_name).exists()`（**字面路径**）

而 `manager.agent_ids()`（`skill.rs:50`）收集的是**内建 sub-agent + `.star/agents/*.md` 自定义 sub-agent**，**不是 `.star/skills/`**。

**结论**：把设计能力写成 `.star/skills/design/SKILL.md`，模型用 `skill(name: "design")` 调用时：
- 不匹配任何 sub-agent
- 不以 http 开头
- `Path::new("design").exists()` 为假（除非 CWD 下真有个叫 `design` 的文件）
- → 落到 `skill.rs:305`，返回 `"No suitable agent or external skill found for: design"`

同时 ✅ 全仓 `grep available_skills` 只命中两处**字符串字面量**：`src/core/prompts/system-prompts/tool-description-skill.md:15`（"Check `<available_skills>` for built-in options"）和 `src/commands/skills.rs:290`（一个 markdown 标题）。**系统提示里没有任何地方真的注入技能清单。**

**这是 P0，且它是所有 skill 形态特性的共同前置。**

### 2.4 无 artifact / canvas / 浏览器能力

✅ 逐项 grep 确认：

| 能力 | 结论 |
|---|---|
| `iframe` / `webview` / `WebView` | **0 命中** |
| `canvas` | **0 命中** |
| artifact 作为渲染概念 | 44 处 `artifact` 命中，**没有一处是渲染语义** |
| `src/ui/` 里的 `html` | 7 处，全是无关的语法高亮 / 标签剥离 |
| 终端图形协议（kitty/sixel/iTerm） | 无 |
| `.claude/skills/` 回落 | ❌ 无。`project_context.rs` 只读 `CLAUDE.md` 与 `settings*.json` |
| `.star/commands/` | ❌ `storage.rs:72-74,156-158` 定义了但**零调用点** |
| `STAR_DIR` 可覆盖 | ❌ 写死 `const STAR_DIR: &str = ".star"`（`storage.rs:8`） |

⚠️ `src/core/chrome/mod.rs` 是**桩**：`mod.rs:175-176` 返回 `ChromeResult::ScreenshotData { base64: String::new() }`；唯一调用者 `/chrome` 的文档注释（`parity.rs:4163`）自己写着"只有页面列举是真的；导航/点击/截图在本 build 里未实现"，并由 `chrome_limits()`（`parity.rs:4219`）在输出里重复一遍。**不要把它当作可用能力写进方案。**

### 2.5 现成的落点（✅ 已核实）

| 落点 | 位置 | 用途 |
|---|---|---|
| 斜杠命令注册 | `src/commands/system.rs:33-2194` `ALL_COMMANDS` + `HELP_CATEGORIES:17-30`（未列 category = `/help` 里隐形） | 声明 `/design` |
| 斜杠命令 dispatch | `src/commands/mod.rs:66-412` | 加 match 臂，**必须排在 `is_declared_pending` 兜底之前** |
| 命令实现惯例 | `src/commands/parity.rs` | 对标命令的指定归属地 |
| skill 解析器 | `src/agent/skills/loader.rs:165-275` `parse_frontmatter` | 手写 YAML 子集，支持 `allowed-tools` / `paths` / `argument-hint` / `model` / `effort` / `disabled` 等 |
| skills 目录 | `src/core/config/storage.rs:160-162` `.star/skills/`、`storage.rs:76-78` `~/.star/skills/` | 落点 |
| 半静态系统提示注入位 | `src/agent/prompt_builder.rs:395` `semi_static_parts` 起，`### Project Instructions` 在 `:398` | 注入 `<available_skills>` |
| 非缓存的轮次级上下文 | `prompt_builder.rs:565` `build_turn_context` | P0-b 的**安全落点**（不参与缓存前缀） |
| 提示缓存不变式 | `prompt_builder.rs:347-368`（"缓存前缀必须逐字节稳定"）；git 快照的 TTL 稳态范例见 `:47` / `:96` / `:519-522` | **缓存前缀必须逐字节稳定**，加块要守这条 |
| 工具描述由 md 驱动 | `src/core/prompts/system-prompts/tool-description-<key>.md`（68 个）+ `tool_descriptions.rs:26-110` 手维护映射，`tool_executor.rs:186` `resolve_tool_description` 应用 | 新工具描述 |
| Modal 体系 | `src/ui/state/modal.rs:22-59`（14 变体）+ `src/ui/app/mod.rs` 的穷尽 `match top_modal()` + `src/ui/events/modal_input.rs:30` `handle_modal_key`（分派在 `:35`，palette 在 `:97`） | 画板选择器 |
| 渲染权威 | `src/ui/app/mod.rs:42` `render_page`，**8 段纯纵向 Layout**（`:86-98`），无横向分栏 | 分栏是**大改**，见 §3.6 |
| 持久化惯例 | `src/core/config/storage.rs` 加 accessor；再往 `artifact_locations()`（`parity.rs:4231`）加一条 key，`/artifacts` 自动捡起 | `.star/artboards/` |

---

## 3. 落地方案

### 3.0 范围声明 —— 明确不做什么

先把"不可复制"钉死，避免方案里出现假承诺：

| 官方能力 | 处置 | 理由 |
|---|---|---|
| claude.ai artifact 发布 / 版本化 / 私有-公开权限 | **平替**：写单个自包含 HTML 到工作区，浏览器打开 | 无 claude.ai，无账号体系。且原版在这点上本身有缺陷（见 §5） |
| 2.1 MB 预编译画板编辑器（WYSIWYG） | **不自研**（除非单独立项） | 专有预编译产物，无法复制。tweak 环节降级为"在源码上改 → 重新播种" |
| `DesignSync` / `/design-sync` | **不做** | 它同步的是 React 设计系统，本方案产出纯 HTML/CSS，天然不适用 |
| `/design import/export/status` | **不做** | preview 期间官方自己也没开 |

**一句话概括本方案**：
> 复刻**协议与工作流**（`.dc.html` + `canvas.json` + seed 脚本 + 设计系统采集 + handoff），放弃**托管发布与专有编辑器**。产出一个可提交、可 diff、可离线打开的多画板画布。

### 3.1 P0 —— 打通 skill 可达性（前置阻塞，最小改动）

**为什么必须先做**：§2.3 已证明 `.star/skills/*/SKILL.md` 对模型不可见。不修这个，后续全是空谈。

两档，按投入排序：

**P0-a（最小，~30 行）** —— 让 `skill` 工具能按名字解析本地 skill。
在 `src/agent/tools/skill.rs:275-283` 的 `else` 分支里，`Path::new(&skill_name).exists()` 失败后，再查一次 `Storage::project_skills_dir()` / `Storage::user_skills_dir()` 下的 `<name>/SKILL.md`。同时把 `available_skill_names`（`:50`）的来源从"仅 sub-agent"扩成"sub-agent + 本地 skill 目录"。

**P0-b（更正确，~80 行 + 一个 md 模板）** —— 按官方语义注入 `<available_skills>`。
在 `src/agent/prompt_builder.rs:397-406` 旁加一个半静态 section，渲染 skill 目录的 `name` + `description`（只放这两项，正对应官方的"渐进式披露第一级"）。

**纪律**：skill 列表会随文件增删变化，而 `prompt_builder.rs:347-368` 要求缓存前缀逐字节稳定。两条出路 —— 放 `build_turn_context`（`prompt_builder.rs:565`，轮次级、非缓存），或仿照 git 快照的 TTL 稳态做法（`:47` / `:96` / `:519-522`）。**这是本阶段最容易翻车的地方。**

**验收**：`grep` 确认 `<available_skills>` 真的出现在组装后的系统提示里（不要信"读代码看着能跑"，`#![allow(dead_code)]` 开着）。

### 3.2 P1 —— Skill 层：`.star/skills/design/`

对齐 Claude 官方的 skill 目录约定：

```
.star/skills/design/
├── SKILL.md                      # 主指令，控制在 500 行以内
├── references/
│   ├── artboard-format.md        # §1.5 的完整规则（含"静默失败"清单）
│   ├── canvas-manifest.md        # §1.6 的 canvas.json 全字段规格
│   └── design-system.md          # §1.9 的 token 采集清单
├── assets/
│   ├── Main.dc.html              # 画板骨架模板
│   ├── canvas.json               # 布局清单模板
│   └── payload.template.html     # 自研的极简画布查看器
└── scripts/
    ├── seed-canvas.mjs           # §1.7 的等价实现
    └── extract-canvas.mjs        # 反向抽取
```

**SKILL.md 的 frontmatter 设计**：

| 字段 | 值 | 理由 |
|---|---|---|
| `name` | `design` | 目录名一致 |
| `description` | 三人称、含"做什么 + 何时用"，< 1024 字符，无 XML 尖括号 | 官方硬性要求 |
| `argument-hint` | `[brief]` | 自补，对齐官方 |
| `disable-model-invocation` | `true` | **默认关**。官方描述里 `/design` 是显式触发的；且生成画板很贵（§1.11），不该让模型在无关对话里自动烧 token |
| `allowed-tools` | `Read, Glob, Grep, Bash(node:*)` | 预授权 node 跑 seed 脚本 |

**SKILL.md 正文必须承载的关键内容**（否则容易退回"通用 Claude 审美"）：

1. **先问一个问题**：静态模型图还是要可点的原型？这是官方明确要求先澄清的分叉。
2. **先采集设计系统再画**（§1.9），优先级链原样写进去。
3. **画板源码的硬规则**（§1.5），尤其是那批"静默失败"的。
4. **播种契约**（§1.7），包含**文件名和标题是内容不是工具名**这条。
5. **handoff 段**：告诉模型用户会回来做 pick / tweak / implement，以及**怎么把画板还原成项目代码**。

### 3.3 P2 —— seed 脚本等价实现

`seed-canvas.mjs` 的职责边界（**不做编辑器**）：

| 做 | 不做 |
|---|---|
| 校验画板命名 / 标题 / 尺寸（§1.7 的拒绝表**逐条照抄**） | 不注入任何编辑器运行时 |
| 把 `.dc.html` 源码 + `canvas.json` + 图片 base64 烘进 `payload.template.html` | 不做点击选中 / 属性面板 / 撤销重做 |
| 转义种子源码，保证无法闭合状态块 | 不做 Save |
| `--extract` 反向抽取，拒绝覆盖 | 不做在线协同 |

**`payload.template.html` 的目标形态**：一个 ~30–50 KB 的**零依赖单文件画布查看器** —— 平移/缩放画布、按 `boards` 布局渲染每个 `.dc.html`（`sandbox` frame）、显示便签、PNG/PDF 导出按钮。

> **PDF 导出的现实提醒**：`print: "flow"` 让 PDF 自动分页，靠的是浏览器打印。在无头场景下这条路要另写；但对本地画布来说，"打开 → 浏览器打印 → 存 PDF"是可接受的降级，**不必在 P2 解决**。

**为什么值得做**：种子文件完全离线可渲染，`file://` 双击即开，无网络无登录。这正好补上原版"Save 依赖 artifact 运行时"的结构性缺口。

### 3.4 P3 —— handoff 回实现

这是 `/design` 真正的价值兑现点，也是最容易被做漏的一段。方案要求 skill 正文包含一节 **"从选定画板实现"**：

- 读 `canvas.json` 的 `order` + 选中项 → 定位 `.dc.html`
- 按 §1.9 采集到的 token 反推该用什么设计系统 / 组件
- 遵守项目自身的 UI 约定（这一点对 StarCode 尤其重要：本仓已有 ratatui + i18n + 快照测试三套硬约束，画布不该让模型忘掉它们）
- 明确标注这是 **tweak 后的最终版**（因为我们的 tweak 是改源码重新播种，不存在原版那种"页面改完自动同步"）

### 3.5 P4 —— `/design` 斜杠命令（薄壳）

让命令而非模型来触发，成本极低：

1. `src/commands/system.rs` 的 `ALL_COMMANDS` 加一项，category 必须选 `HELP_CATEGORIES`（`:17-30`）里已有的值，否则 `/help` 里看不见。
2. `src/commands/parity.rs` 加 `pub async fn design(ctx, args)`，把 `args` 拼成 brief。
3. `src/commands/mod.rs:66-412` 加 match 臂，**排在 `is_declared_pending` 兜底之前**（CLAUDE.md:128-137 明确点名的坑）。

**同时要处理 §2.1 的命名冲突**：`/artifacts` 现在是磁盘浏览器。选项 —— (a) 把新画布浏览并入 `/artifacts` 作为一类来源；(b) 新开 `/artboards`；(c) `/artifacts` 改名。**建议 (b)**，语义最清晰且不动既有行为。

### 3.6 P5（可选，后续评估）—— Rust 侧原生画板

只有 P0–P4 跑通、且确实被日常使用后才值得做。成本从低到高：

| 档 | 内容 | 成本 |
|---|---|---|
| 低 | `Modal::Artboards` 选择器 + `PaletteMode::Artboards`，列出 `.star/artboards/` 下的画布，回车打开浏览器 | 中。需改 `modal.rs:22-59` + `app/mod.rs` 的穷尽 match + `modal_input.rs:30` `handle_modal_key` 的穷尽分派（`:35`） |
| 高 | `render_page`（`app/mod.rs:86-98`）从纯纵向 8 段改成横向分栏，做真正的内嵌预览 | **高**。按 CLAUDE.md:83-90，分栏几何属于「布局记忆」，要连带处理 `last_chat_area` 的消费方（滚动、鼠标）和「聊天区宽度恒少 1 列」的不变式（CLAUDE.md:211） |

**不要走的路**：❌ 不要试图在 TUI 里渲染 HTML。已确认无浏览器、无 webview、无终端图形协议，`src/core/chrome/mod.rs` 是返回空 base64 的桩。**唯一可行的呈现方式是把画布落到 HTML 文件交给浏览器。**

### 3.7 与官方的能力差异（需在 UI 上如实声明）

| 维度 | Claude Code | 本方案 |
|---|---|---|
| 画板生成 | ✅ | ✅ 同规格 |
| 画布查看 / 平移缩放 / PNG/PDF | ✅ | ✅ P2 覆盖 |
| WYSIWYG 编辑（点击选中 / 属性面板 / 内联改字 / 撤销重做） | ✅ | ❌ **降级为改源码重新播种** |
| 保存 = 发布新版本给所有人 | ✅ | ❌ 无托管发布 |
| 权限分级（私有 / 公开链接） | ✅ 但只有两档，中间档缺失 | ❌ 无（纯本地文件，需分享请自行托管静态副本） |
| 设计系统采集 | ✅ | ✅ |
| handoff 回实现 | ✅ | ✅ |

---

## 4. 风险与红线

| 风险 | 说明 | 处置 |
|---|---|---|
| **提示缓存被打破** | P0-b 往半静态块注入会变化的 skill 列表，违反 `prompt_builder.rs:347-368` 的逐字节稳定不变式 | 放 `build_turn_context`（`:565`）或走 git 快照式 TTL 失效；跑一轮长会话验证前缀没变 |
| **token 成本** | 官方侧最集中的抱怨就是烧 token（§1.11），多画板 × 内联 CSS × data URI 图片是主因 | `disable-model-invocation: true`；SKILL.md 里写明"小屏先别开多方案"；画板数量上限写进指令 |
| **统一审美** | 第三方实测早期产物偏向"青色渐变 + 衬线字 + 堆叠药丸卡片" | 设计系统优先级链必须写进 SKILL.md；本仓可提供 ratatui 主题色作为默认 token 样例 |
| **自研查看器被当编辑器用** | 用户会期待在页面里改完就同步 | SKILL.md 与命令输出**都**要明说：改完重新播种 |
| **`/artifacts` 语义漂移** | §2.1 | 走 §3.5 的 (b) 方案，不动既有语义 |
| **误把 chrome 桩当能力** | §2.4 | 方案里显式排除 |
| **静默失败** | §1.5 那批规则（缺 `$export` 头行、错误的高度单位、`<script>` 拼 UI）全部不报错 | seed 脚本加 lint：检查每个 `.dc.html` 的 `$export` 头行、根元素尺寸声明、禁 `innerHTML` |
| **`{{hole}}` 误当表达式** | 它只是查表占位符 | 写进 `references/artboard-format.md`，并让 lint 拒绝带表达式的写法 |

---

## 5. 信源冲突（必须如实记录，不要抹平）

| 争点 | 说法 A | 说法 B | 本文档立场 |
|---|---|---|---|
| 最低版本 | 官方 digest：**v2.1.234** | 第三方实测：**v2.1.233** | 采信官方 234；233 记为 changelog 口径差异 |
| 是否自动匹配现有代码风格 | Stacktree（实跑并检查磁盘产物）：**会**，明确说它捞出了本仓精确的调色板与圆角 | explainx.ai：**不会**，列为 known limitation | 折中：**会采集 token，但组件复用不保证**。落地时按"必须人工核对 token"来设计 |
| canvas.json 版本 | `{"v":3, "createdOnFiles":{"v":1,...}}` | 无对立说法 | 两个字段都保留原样，交给脚本校验 |
| Save 能力 | 官方描述：账户支持保存则给 WYSIWYG | 第三方：**未确认** preview 内是否自动继承 | 不依赖它。本方案本就无托管发布 |

**其他值得记的实测缺陷**（官方未在文档承认）：

- artifact **无法给查看者做鉴权** —— 没有密码、没有邮箱域白名单、没有过期。状态只有「仅自己」和「拿到 URL 的人都能开」。客户端项目真正需要的中间档不存在。
- **编辑器在发布时被冻结**，之后不更新 —— 老画布永远是老编辑器。
- 多画板 canvas 每个画板挂一个 `about:srcdoc` frame，frame 身份里**不含**它来自哪个 `.dc.html`，无法映射。

---

## 6. 待查清单

| # | 待查 | 怎么查 |
|---|---|---|
| 1 | `preview_last_entry_idx` 原语义（指向 chat entry 还是别的） | 读 `src/ui/services/stream.rs:1950-1985` 的写入上下文 |
| 2 | P0-b 的 `<available_skills>` 放缓存段还是非缓存段 | 读 `prompt_builder.rs:395-406` 的实际分段实现、`:565` 的 `build_turn_context` 边界，以及 git 快照 TTL 机制（`:47` / `:519-522`） |
| 3 | `ToolName` 枚举新增变体的 3 处改动点 | `src/core/tools/constants.rs:12-101 / :105-194 / :203-275` |
| 4 | ratatui 主题色能否抽成默认 design token 样例 | 读 `src/ui` 主题定义 + `i18n/` |
| 5 | `/artifacts` 重命名的连带影响 | grep `artifact_locations()` 的其他调用方 |

---

## 7. 出处

**官方（一手）**
- Week 34 digest（`/design` 首发公告）：https://code.claude.com/docs/en/whats-new/2026-w34
- Artifacts（含 CSP 约束、可用性门槛、成本）：https://code.claude.com/docs/en/artifacts
- Skills（frontmatter 全字段）：https://code.claude.com/docs/en/skills

**第三方（实测与逆向，可信度分档）**
- Stacktree 实测（2026-08-18，检查了磁盘产物）：https://stacktr.ee/blog/what-is-design-in-claude-code
- Blakecrosley（首发梳理，明确标注"文档到此为止"）：https://blakecrosley.com/blog/claude-code-design-skill
- explainx.ai（含 token 成本与设计系统匹配的争议说法）：https://explainx.ai/blog/claude-code-design-command-artboards-research-preview-2026
- Mikegingerich（艺术板工作流综述）：https://www.mikegingerich.com/blog/anthropic-adds-design-artboards-to-claude-code/
- pasqualepillitteri.it（西班牙语，含技术与可用性门槛梳理）：https://pasqualepillitteri.it/en/news/11689/
- Design artifact type `SKILL.md` 第三方镜像（画板格式规格的主要来源）：https://github.com/asgeirtj/system_prompts_leaks
- ultimate-frontend-skills 的 `claude-design.md`（对 `DesignSync` / `/design-sync` / CSP 字体的独立核验）：https://github.com/ridelink0/ultimate-frontend-skills

**StarCode 内部**
- 架构对标总纲（本文格式与判定纪律的来源）：`docs/claude-code-alignment.md`
- 项目约定（命令四触点、TEA、注释语言、i18n）：`CLAUDE.md`
