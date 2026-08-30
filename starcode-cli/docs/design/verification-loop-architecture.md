# 验证循环架构

> StarCode CLI 的自测试、自修正机制。对标 Claude Code 的 Verification Loop 体系。

## 1. 核心理念

**验证是外部的、确定性的。** 测试套件决定是否通过，不是模型自己判断。

```
用户请求 → Agent 执行 → 外部验证 → 通过/修正 → 循环
```

### 原则

| 原则 | 说明 |
|------|------|
| 外部判断 | 验证逻辑在 agent 上下文之外，由命令/脚本决定 |
| 确定性 | 相同输入 → 相同结果，无模型参与决策点 |
| 分层防护 | 语法 → 意图 → 回归，逐层拦截 |
| 防循环 | `stop_hook_active` 机制防止无限验证 |
| 结果优先 | 评估产出，不评估路径 |

## 2. 三层验证体系

### Layer 1: 语法验证（PostToolUse Hook）

**触发时机**：每次 Write/Edit 工具调用后
**执行方式**：确定性命令（lint、type-check）
**反馈方式**：`additionalContext` 注入下一轮

```rust
// PostToolUse Hook 配置
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "cargo check 2>&1 | tail -20"
          }
        ]
      }
    ]
  }
}
```

**检查内容**：
- `cargo check` — 编译错误
- `cargo clippy` — lint 警告
- `rustfmt --check` — 格式问题

**特点**：
- 零 token 消耗
- < 2 秒执行
- 返回 exit 0（不阻止工具完成，只注入反馈）

### Layer 2: 意图验证（Stop Prompt Hook）

**触发时机**：Agent 尝试停止响应时
**执行方式**：LLM 判断（Prompt Hook）
**反馈方式**：exit 2 阻止停止

```rust
// Stop Prompt Hook
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "prompt",
            "prompt": "Review what was accomplished. Check if all requirements from the user's original request were addressed. If incomplete, respond with {\"decision\": \"block\", \"reason\": \"<what remains>\"}. If complete, respond with {\"decision\": \"allow\"}."
          }
        ]
      }
    ]
  }
}
```

**检查内容**：
- 用户原始请求是否被完整响应
- 遗漏的需求项
- 错误的理解方向

**特点**：
- 消耗 token（LLM 判断）
- 适用于复杂/多步骤任务
- 可替换为 Agent Hook（更彻底但更慢）

### Layer 3: 回归验证（Stop Command Hook）

**触发时机**：Agent 尝试停止响应时
**执行方式**：确定性命令（测试套件）
**反馈方式**：exit 2 阻止停止

```bash
#!/bin/bash
INPUT=$(cat)

# 防循环机制 — 必须检查
if [ "$(echo "$INPUT" | jq -r '.stop_hook_active')" = "true" ]; then
  exit 0  # 已在强制继续状态，允许停止
fi

# 运行测试
TEST_OUTPUT=$(cargo test 2>&1)
if [ $? -ne 0 ]; then
  TRIMMED=$(echo "$TEST_OUTPUT" | tail -50)
  echo "Tests failing. Fix before completing:\n$TRIMMED" >&2
  exit 2  # 阻止停止
fi

# 运行构建
BUILD_OUTPUT=$(cargo build 2>&1)
if [ $? -ne 0 ]; then
  TRIMMED=$(echo "$BUILD_OUTPUT" | tail -30)
  echo "Build failing:\n$TRIMMED" >&2
  exit 2
fi

exit 0  # 允许停止
```

**检查内容**：
- `cargo test` — 测试是否通过
- `cargo build` — 构建是否成功
- 自定义验证脚本

**特点**：
- 零 token 消耗
- 最高 ROI 单一 hook
- 捕获最常见失败：agent 说"完成"但测试未通过

### Hook 执行顺序

```
Stop Hooks 按定义顺序执行：
1. Layer 3 (Command Hook) — 快速确定性检查
   ├─ 失败 → exit 2，阻止停止，注入错误信息
   └─ 通过 → 继续
2. Layer 2 (Prompt Hook) — LLM 意图检查
   ├─ 不完整 → exit 2，阻止停止
   └─ 完整 → exit 0，允许停止
```

**先执行 Layer 3**：如果测试失败，无需消耗 token 做意图检查。

## 3. 防循环机制

### stop_hook_active

```bash
# 每个 Stop Hook 必须检查此字段
if [ "$(echo "$INPUT" | jq -r '.stop_hook_active')" = "true" ]; then
  exit 0  # 允许停止
fi
```

**原理**：
- `exit 2` 阻止 agent 停止
- Agent 修复后再次尝试停止
- 如果 hook 再次 `exit 2` → 无限循环
- `stop_hook_active = true` 表示已在强制继续状态
- 此时 `exit 0` 允许停止，打破循环

### 最大连续阻止次数

Claude Code 在 8 次连续 `exit 2` 后强制结束 turn。StarCode CLI 应实现类似机制：

```rust
const MAX_CONSECUTIVE_BLOCKS: u32 = 8;

// 在 agent loop 中跟踪
if consecutive_blocks >= MAX_CONSECUTIVE_BLOCKS {
    // 强制结束，注入警告
    warn!("Verification loop exceeded max iterations. Forcing completion.");
}
```

## 4. 验证循环的四种形态

### 4.1 独立调用（Standalone）

```
用户手动触发 → /verify → 执行检查 → 返回结果
```

**适用场景**：
- 跨切面检查（安全扫描、PR 审计）
- 不常用但重要的验证
- 跨多工作流可用

### 4.2 内嵌（Embedded）

```
Skill A → 步骤1 → 步骤2 → 验证步骤 → 完成
```

**适用场景**：
- 特定工作流的自动检查
- 验证属于单一工作流
- 生产 skill 文件可编辑

**实现**：在 skill 末尾附加检查步骤

```markdown
## Verification
Run `cargo test` and confirm all tests pass.
If any test fails, fix the implementation before reporting done.
```

### 4.3 链式（Chained）

```
/code-review → /simplify → /verify → /design
```

**适用场景**：
- 端到端开发流水线
- 多个 skill 串联
- 自动化完整开发周期

**实现**：

```markdown
# .claude/skills/full-dev-cycle/SKILL.md
Run /code-review on the current diff first.
When /code-review finishes, invoke /simplify.
When /simplify finishes, invoke /verify.
If the change touched UI, invoke /design.
```

### 4.4 PR 级（On Every PR）

```
GitHub Actions → Claude + Verification Skill → 通过/失败
```

**适用场景**：
- 团队基础设施
- 每个 PR 自动验证
- 不依赖作者自觉

## 5. 实现计划

### Phase 1: 基础 Hook 框架

| 任务 | 文件 | 状态 |
|------|------|------|
| Hook 配置解析 | `src/core/hooks/config.rs` | 已有 |
| Hook 执行器 | `src/core/hooks/runner.rs` | 已有 |
| PostToolUse 事件 | `src/core/hooks/events.rs` | 需补全 |
| Stop Hook 事件 | `src/core/hooks/events.rs` | 需补全 |
| stop_hook_active 字段 | `src/agent/agent_loop.rs` | 需新增 |

### Phase 2: 三层验证实现

| 任务 | 文件 | 状态 |
|------|------|------|
| Layer 1: PostToolUse 语法检查 | `.starcode/hooks/verify-syntax.sh` | 需新建 |
| Layer 2: Stop Prompt Hook | 配置文件 | 需新建 |
| Layer 3: Stop Command Hook | `.starcode/hooks/verify-regression.sh` | 需新建 |
| 防循环计数器 | `src/agent/loop_engineering.rs` | 需新增 |

### Phase 3: Skill 集成

| 任务 | 文件 | 状态 |
|------|------|------|
| /verify skill | `.starcode/skills/verify/SKILL.md` | 需新建 |
| /code-review skill | `.starcode/skills/code-review/SKILL.md` | 需新建 |
| 链式 skill 支持 | `src/agent/skills/loader.rs` | 需补全 |

### Phase 4: 遥测与监控

| 任务 | 文件 | 状态 |
|------|------|------|
| 验证事件记录 | `src/core/events/telemetry.rs` | 需新增 |
| Hook 执行统计 | `src/core/hooks/stats.rs` | 需新增 |
| 回归基线管理 | `src/core/verification/baseline.rs` | 需新增 |

## 6. 默认配置

### .starcode/settings.json

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "cargo check 2>&1 | tail -20"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bash .starcode/hooks/verify-regression.sh"
          },
          {
            "type": "prompt",
            "prompt": "Review what was accomplished. Check if all requirements from the user's original request were addressed. If incomplete, respond with {\"decision\": \"block\", \"reason\": \"<what remains>\"}. If complete, respond with {\"decision\": \"allow\"}."
          }
        ]
      }
    ]
  }
}
```

### .starcode/hooks/verify-regression.sh

```bash
#!/bin/bash
INPUT=$(cat)

# 防循环机制
if [ "$(echo "$INPUT" | jq -r '.stop_hook_active')" = "true" ]; then
  exit 0
fi

# 运行测试
TEST_OUTPUT=$(cargo test 2>&1)
if [ $? -ne 0 ]; then
  TRIMMED=$(echo "$TEST_OUTPUT" | tail -50)
  echo "Tests failing. Fix before completing:\n$TRIMMED" >&2
  exit 2
fi

exit 0
```

## 7. 验证指标

| 指标 | 目标 | 说明 |
|------|------|------|
| Hook 执行延迟 | < 5s | 单次 hook 执行时间 |
| 验证通过率 | > 90% | 首次验证通过的比例 |
| 循环次数 | < 3 | 平均修正循环次数 |
| 假阳性率 | < 5% | 验证失败但实际正确的比例 |
| Token 节省 | > 30% | 通过 PostToolUse 早期拦截节省的 token |
