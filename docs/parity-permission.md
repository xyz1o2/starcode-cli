# Permission UI 对标分析 — 缺失项详细设计

> 参考: `study_or_copy_projects/claude-code-main/` — PermissionRequest 系统
> 当前: `src/ui/components/confirmation_dialog.rs` (1,295 行)

## 现状

已实现统一对话框:
- `RiskLevel` 枚举: Safe / Low / Medium / High / Critical
- 颜色常量: `PERMISSION_COLOR`, `SUGGESTION_COLOR`, `SUCCESS_COLOR`, `ERROR_COLOR`
- 单一对话框处理所有工具确认

## 缺失项

---

### 1. Feedback Input 系统 — P0

**CCB 实现**:
```
Tab: 切换 列表选择 ↔ 文本输入模式
  - Yes 场景: 输入 "下一步做什么" (附加指令)
  - No 场景: 输入 "改什么" (修改建议)
输入框: 单行, 提交后作为 feedback 附在确认响应中
```

- `PermissionDialog` 底部: 选项列表 + "Tab to switch to input" 提示
- 输入模式: 光标在文本框, Enter 提交, Esc 返回选项模式
- 反馈内容: 附加到 `ToolConfirmationResponse` 的 `feedback` 字段

**Starcode 需实现**:
- `ConfirmationDialog` 增加 `input_mode: bool` 状态
- Tab 键切换: 选项模式 ↔ 输入模式
- 输入模式: 单行 `TextArea`, Enter 提交
- 反馈传递: `ConfirmTool` 消息增加 `feedback: Option<String>` 字段
- UI 提示: 底部显示 "Tab to provide feedback"

**涉及文件**: `src/ui/components/confirmation_dialog.rs`, `src/runtime/messages.rs`, `src/ui/events/modal_input.rs`

---

### 2. 按工具分发特化组件 — P1

**CCB 实现**:
```
permissionComponentForTool(toolName):
  "Bash"            → BashPermissionRequest
  "Edit"/"Write"    → FileEditPermissionRequest
  "Read"            → ReadPermissionRequest (简版)
  "WebFetch"        → WebFetchPermissionRequest
  "TodoWrite"       → TodoWritePermissionRequest
  "TaskCreate/..."  → TaskPermissionRequest
  "SendMessageTool" → SendMessagePermissionRequest
  "McpTool"         → McpPermissionRequest
  ... 12+ 专用组件
```

每个组件定制:
- 标题: "Allow bash command?" vs "Edit file?" vs "Create task?"
- 内容区: Bash 显示命令文本, Edit 显示 diff, Task 显示任务详情
- 风险提示: Bash 有 shell 注入警告, Edit 有文件覆盖警告

**Starcode 需实现**:
- 新增 `PermissionComponent` trait 或 enum
- `confirmation_dialog.rs` 改为 dispatcher: 按 tool_name 选择渲染逻辑
- 优先实现: Bash (命令文本), Edit/Write (diff), Read (文件路径)
- 后续: WebFetch, Task, MCP

**涉及文件**: `confirmation_dialog.rs` (重构), 可能拆分为 `permission/` 目录

---

### 3. Shift+Tab 快速批准 — P1

**CCB 实现**:
```
Shift+Tab: 立即批准, 跳过选项列表
  - 等同于选择第一个选项 (Allow) 但更快
  - accept-edits 模式下: 直接批准文件编辑
```

**Starcode 需实现**:
- `modal_input.rs` 中增加 Shift+Tab 处理
- 行为: 立即发送 `ConfirmTool { approved: true }`, 关闭对话框
- 仅在 `ApprovalMode::Default` 下可用 (Yolo 模式不需要)

**涉及文件**: `src/ui/events/modal_input.rs`

---

### 4. 底部模式指示器 — P1

**CCB 实现**:
```
⏵⏵ auto on (shift+tab to cycle)    // 底部状态栏
颜色: auto=orange, bypass=red, plan=planMode 颜色
```
- 始终在底部状态栏显示当前权限模式
- Shift+Tab 在非对话框状态下循环模式

**Starcode 需实现**:
- `status_line.rs` 增加权限模式指示区
- 显示: `⏵⏵ {mode_name}` + 颜色
- 模式映射:
  - `Default` → "default" (白色)
  - `Plan` → "plan" (蓝色)
  - `Yolo` → "yolo" (红色)
- 非对话框状态下 Shift+Tab 循环模式

**涉及文件**: `src/ui/components/status_line.rs`, `src/ui/events/input.rs`

---

### 5. AI 风险解释 (Ctrl+E) — P2

**CCB 实现**:
```
Ctrl+E: 请求 AI 解释当前操作的风险
  - lazy-load: 按需调用 LLM
  - shimmer 加载态: 文字渐变动画
  - 结果: 显示在对话框下方, markdown 格式
```

**Starcode 需实现**:
- Ctrl+E 触发: 发送 `AgentRequest::GenerateNote` 请求风险分析
- prompt: "Explain the risks of: {tool_name} with {args}"
- 加载态: shimmer 或 spinner
- 结果: 追加到对话框下方, 支持 markdown 渲染

**涉及文件**: `confirmation_dialog.rs`, `modal_input.rs`, `src/runtime/messages.rs`

---

### 6. 分类器自动审批 — P2

**CCB 实现**:
```
classifier: 基于工具名 + 参数判断是否安全
  - 安全: 选项 disabled + 绿色 "Auto-approved" 副标题
  - 不安全: 正常审批流程
  - 分类器: ML 模型 或 规则引擎
```

**Starcode 需实现**:
- `PolicyEngine::check` 增加自动审批规则 (当前 `load_permission_rules` 未接线)
- 或: 简单规则引擎 — 匹配工具名 + 参数模式
- UI: 自动审批时显示绿色 "Auto-approved" 标记
- 需要先接线 `PolicyEngine` (见 `parity-agent.md` §12)

**涉及文件**: `src/core/policy/policy_engine.rs`, `confirmation_dialog.rs`

---

### 7. WorkerBadge (@agentName) — P2

**CCB 实现**:
```
WorkerPendingPermission:
  - spinner + "Waiting for @agentName..."
  - WorkerBadge: 彩色标签 @agentName
  - 用于多 agent 场景: 子 agent 请求权限时显示来源
```

**Starcode 需实现**:
- `ToolConfirmationRequest` 增加 `agent_name: Option<String>`
- 对话框标题区: 如果有 agent_name, 显示 `@{agent_name}` 标签
- 颜色: 基于 agent name hash 生成一致颜色

**涉及文件**: `confirmation_dialog.rs`, `src/runtime/messages.rs`

---

### 8. TrustDialog (首次信任扫描) — P2

**CCB 实现**:
```
首次打开项目时:
  1. 扫描: MCP servers / Hooks / env 变量 / .claude/settings.json
  2. 检测: 可执行文件路径、shell hooks、环境变量中的敏感 key
  3. 显示: TrustDialog 列出发现的风险项
  4. 选项: Trust this project / Don't trust / Review details
  5. 持久化: 信任状态保存到 ~/.claude/trusted_projects.json
```

**Starcode 需实现**:
- 扫描逻辑: 检查 `.star/settings.json`, MCP 配置, `STAR_*` 环境变量
- 风险检测: 可执行路径、shell hooks、API key 明文
- `TrustDialog` 组件: 风险列表 + 操作按钮
- 持久化: `.star/trusted_projects.json`

**涉及文件**: 新建 `src/ui/components/trust_dialog.rs`, `src/core/config/`

---

## 实施建议

| 序号 | 任务 | 优先级 | 工作量 | 依赖 |
|------|------|--------|--------|------|
| 1 | Feedback Input (Tab 切换) | P0 | 中 | messages.rs 增加字段 |
| 2 | Shift+Tab 快速批准 | P1 | 小 | 无 |
| 3 | 底部模式指示器 | P1 | 小 | status_line.rs |
| 4 | 按工具分发组件 | P1 | 大 | 重构 confirmation_dialog |
| 5 | AI 风险解释 Ctrl+E | P2 | 中 | LLM 调用 |
| 6 | 分类器自动审批 | P2 | 中 | PolicyEngine 接线 |
| 7 | WorkerBadge | P2 | 小 | messages.rs 增加字段 |
| 8 | TrustDialog | P2 | 大 | 扫描逻辑 + 新组件 |
