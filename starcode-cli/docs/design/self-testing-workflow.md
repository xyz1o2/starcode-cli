# 自测试工作流

> StarCode CLI 的自我测试、自我验证、自我改进机制。

## 1. 工作流概览

```
┌─────────────────────────────────────────────────┐
│                 用户请求                          │
└──────────────────────┬──────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────┐
│            Agent 执行                            │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐         │
│  │ 收集    │→│ 执行    │→│ 验证    │         │
│  │ 上下文  │  │ 操作    │  │ 结果    │         │
│  └─────────┘  └─────────┘  └─────────┘         │
│       ↑                              │          │
│       └────────────修正──────────────┘          │
└──────────────────────┬──────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────┐
│            三层验证                              │
│  ┌──────────────────────────────────────┐       │
│  │ Layer 1: 语法检查 (PostToolUse)      │       │
│  │ cargo check / clippy / rustfmt       │       │
│  └──────────────────────────────────────┘       │
│  ┌──────────────────────────────────────┐       │
│  │ Layer 2: 意图检查 (Stop Prompt)      │       │
│  │ LLM 判断需求是否完整响应              │       │
│  └──────────────────────────────────────┘       │
│  ┌──────────────────────────────────────┐       │
│  │ Layer 3: 回归检查 (Stop Command)     │       │
│  │ cargo test / cargo build             │       │
│  └──────────────────────────────────────┘       │
└─────────────────────────────────────────────────┘
```

## 2. 测试策略

### 测试金字塔

| 层级 | 占比 | 什么验证 | 运行频率 |
|------|------|---------|---------|
| **单元测试** | 80% | 逻辑、边界、错误处理 | 每次变更 |
| **集成测试** | 15% | 服务边界、数据流、API 契约 | 每次变更 |
| **端到端测试** | 5% | 关键路径全栈验证 | 部署前 |

### 测试配置（CLAUDE.md）

```markdown
## Testing
- 测试框架: cargo test (Rust 内置)
- 测试文件位置: 与源码同目录 #[cfg(test)] mod tests
- 命名约定: test_{function_name}_{scenario}
- 覆盖率最低要求: 80% 行覆盖率
- 每次代码变更后运行: cargo test
- 永远不要删除失败的测试 — 修复实现代码
- 永远不要忽略测试输出
- 断言必须具体 — 禁止 assert!(result.is_some())
```

### 关键规则

| 规则 | 原因 |
|------|------|
| 永远不要删除失败的测试 | Claude 可能把正确测试当作"过时"删除 |
| 断言必须具体 | `assert!(result.is_some())` 什么都没验证 |
| 测试行为而非实现 | 重构不应破坏测试 |
| 测试必须独立 | 任意顺序、并行执行都应通过 |
| Mock 外部 I/O | 不在单元测试中调用真实 API/数据库 |

## 3. TDD 工作流

### 测试先行（推荐）

```
1. 用户编写测试（定义规格）
2. 测试全部失败（红）
3. Agent 实现代码使测试通过（绿）
4. 重构（保持绿色）
```

**为什么测试先行更好**：
- 测试编码了精确规格
- Agent 无法误解模糊需求
- 编写测试时想到的边界情况会被处理
- 回归立即被捕获

### 测试后补（已有代码）

```
1. Agent 分析现有代码
2. 识别公共 API 和边界情况
3. 生成测试文件
4. 运行测试 → 修正 → 循环
5. 检查覆盖率 → 补充缺失
```

**关键提示**：
- 指定已有测试风格："查看 tests/ 中的测试风格，按相同方式编写"
- 使用覆盖率驱动："运行 cargo test --coverage，补充低覆盖率文件的测试"
- 分离测试与实现上下文：用 subagent 写测试，避免测试镜像实现

## 4. 验证循环配置

### 4.1 PostToolUse 语法检查

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
    ]
  }
}
```

**效果**：
- 每次文件写入后自动检查编译错误
- 错误信息注入 agent 下一轮
- Agent 立即修复，不等到最后

### 4.2 Stop 回归检查

```bash
#!/bin/bash
# .starcode/hooks/verify-regression.sh
INPUT=$(cat)

if [ "$(echo "$INPUT" | jq -r '.stop_hook_active')" = "true" ]; then
  exit 0
fi

# 测试
cargo test 2>&1 || {
  echo "Tests failing. Fix before completing." >&2
  exit 2
}

# 构建
cargo build 2>&1 || {
  echo "Build failing. Fix before completing." >&2
  exit 2
}

exit 0
```

### 4.3 Stop 意图检查

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "bash .starcode/hooks/verify-regression.sh"
          },
          {
            "type": "prompt",
            "prompt": "Review what was accomplished vs the user's original request. If incomplete, block."
          }
        ]
      }
    ]
  }
}
```

## 5. 覆盖率管理

### 覆盖率指标

| 指标 | 目标 | 说明 |
|------|------|------|
| 行覆盖率 | ≥ 80% | 代码执行覆盖率 |
| 分支覆盖率 | ≥ 70% | 决策路径覆盖 |
| 变异测试分数 | ≥ 60% | 测试捕获实际行为变更的能力 |

### 覆盖率检查命令

```bash
# 生成覆盖率报告
cargo llvm-cov --html

# 检查覆盖率阈值
cargo llvm-cov --fail-under-lines 80
```

### 覆盖率驱动测试

```
1. 运行覆盖率报告
2. 识别低覆盖率文件
3. 按风险排序（业务逻辑 > 基础设施）
4. 用 subagent 并行补充测试
5. 验证覆盖率提升
```

## 6. 并行测试生成

### SubAgent 并行策略

```
主 Agent（协调者）
  ├─ SubAgent 1: 测试 src/agent/router.rs
  ├─ SubAgent 2: 测试 src/agent/tool_executor.rs
  ├─ SubAgent 3: 测试 src/core/context/engine.rs
  ├─ SubAgent 4: 测试 src/tools/search.rs
  └─ ...（最多 8 个并行）
```

**规则**：
- 每个 subagent 只测试一个类/模块
- 主 agent 负责协调、测量覆盖率、选择目标
- subagent 完成后终止，保持上下文清洁
- 最多 8 个并行 subagent

### CLAUDE.md 配置

```markdown
## Parallel Testing
- Always assign individual agents to write tests (one agent per module)
- Do NOT assign multiple modules to one agent
- Start up to 8 agents in parallel
- Each agent should: write tests → run tests → fix failures → report coverage
```

## 7. 回归防护

### 缺陷驱动测试

```
1. 发现 bug
2. 编写失败测试复现 bug
3. 修复代码使测试通过
4. 保留测试永远
```

**CLAUDE.md 规则**：
```markdown
## Bug Fix Protocol
- Every bug fix MUST include a regression test
- Write the failing test FIRST, then fix the code
- Never remove a regression test, even if it seems redundant
- The test proves the bug existed and won't recur
```

### 变异测试

```bash
# 使用 cargo-mutants
cargo mutants --timeout 300

# 检查变异分数
cargo mutants --minimum-coverage 60
```

**目的**：验证测试是否真正捕获行为变更，而非只是执行代码。

## 8. CI/CD 集成

### GitHub Actions 配置

```yaml
name: Verification Loop
on: [push, pull_request]

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Layer 1 - Syntax
        run: cargo check && cargo clippy -- -D warnings

      - name: Layer 3 - Regression
        run: cargo test

      - name: Coverage
        run: cargo llvm-cov --fail-under-lines 80
```

### PR 验证

每个 PR 必须通过：
1. `cargo check` — 编译通过
2. `cargo clippy` — 无警告
3. `cargo test` — 测试全绿
4. `cargo llvm-cov` — 覆盖率 ≥ 80%

## 9. 监控指标

### 遥测事件

| 事件 | 数据 | 用途 |
|------|------|------|
| `hook_executed` | hook 名称、耗时、exit code | Hook 性能监控 |
| `verification_passed` | 层级、耗时 | 验证通过率 |
| `verification_failed` | 层级、错误信息 | 失败分析 |
| `loop_iteration` | 循环次数、修正内容 | 循环效率 |
| `coverage_change` | 变化量、文件 | 覆盖率趋势 |

### 仪表板

```
验证通过率:  92% ████████████████████░░
平均循环次数: 2.3 ████░░░░░░░░░░░░░░░░
Hook 延迟 P95: 3.2s ███░░░░░░░░░░░░░░░░░
覆盖率趋势:   82% █████████████████░░░░
```
