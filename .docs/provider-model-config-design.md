# provider 模型配置设计：多模型、可选模型、空密钥

> 目标读者：starcode-cli 维护者。本文把"provider 配了模型名之后行为不符合预期、
> 配多个模型名时无法表达、密钥留空时加载表现反直觉"三个问题，落成一套字段语义 +
> 解析规则 + 加载矩阵，并给出落地顺序。
>
> 文中所有代码位置、行号、字段名均已对当前 tree 核实。

## 1. 背景：三个问题，一个根源

用户在 provider 表单里填了 Base URL、可能留空 API Key、可能填一个或多个模型名，
然后在模型面板里选模型。这条路径上有三处不符合预期的地方：

### 1.1 `models` 字段是死的

`ProviderSettings` 里同时存在两个模型相关字段
（`src/core/config/models.rs:25-39`）：

```rust
pub struct ProviderSettings {
    #[serde(alias = "selectedModel", alias = "currentModel")]
    pub selected_model: Option<String>,
    pub models: Option<HashMap<String, ModelConfig>>,
    // ...
}
```

- `selected_model` 是**唯一真正生效**的字段。`provider_resolution.rs:325` 在解析
  "当前该用哪个模型"时读它；`provider_store.rs` 的 `get/set_selected_model`
  也只碰它。
- `models` 全 crate **只有一处**读它：`commands/model.rs:97`，而且只用于
  `/model list` 打印一行 ``- `id` (name)``。

后果：用户（或别的工具）在配置里手写 `models: {...}`，期望它像 Claude Code 的
`providerProfiles` 那样声明"这家能提供这些模型"，结果**面板不显示、切不了、请求层不认**，
只有 `/model list` 会把它打印出来。一个只影响打印、不影响任何行为的配置字段，
比没有这个字段更糟 —— 它让人以为配了。

### 1.2 只能配一个模型

`selected_model: Option<String>` 是单值。要表达"这家可用 N 个模型、其中一个是默认"，
现在只能把 N 个名字塞进同一个字符串 —— 而那是**字面量**，不会被解析成列表。
`/model <name>` 可以临时切，但那是运行时状态，不落配置。

### 1.3 空密钥 = 已配置

`provider_is_configured`（`core/config/providers.rs:491-531`）对**自定义** provider
（不在 `ALL_PROVIDERS` 里的）判定是：

```rust
has_api_key || has_saved_base_url || is_active
```

也就是说，**只要填了 Base URL 就算配好了**，哪怕 API Key 是空的。这个判定同时决定：

- provider 菜��里出不出现（`configured_provider_ids_from`）
- 模型列表扇出时会不会去打它（`agent/model_list.rs` 的 `list_models_with_mode`）

于是：留空 key 的 provider 会正常出现在模型列表里 → 用户选它的模型 → 发请求 →
上游回 401。整条链路上**界面没有任何一处提示"这家没密钥"**，用户只能从一次
网络错误里反推。

## 2. 设计目标

- **多模型**：一个 provider 可以声明 N 个候选模型，其中一个为默认。
- **模型可选**：模型字段整体可以不填，表示"完全交给 API 自动发现"。
- **空密钥可见**：没有凭证是一个**配置时就能看出来**的状态，不是运行时才炸。
- **不迁移**：现有配置文件一行都不用改，旧语义（旧配置）行为不变。

非目标：

- 不做 provider 级别的模型重试 / 故障转移（`a, b, c` 顺序尝试）。
- 不做按模型的独立 `context_window` 之外的 per-model 配置（temperature 等）——
  `ModelConfig` 已有 `name` / `context_window` / `alias` 三个字段，本期只保证它们
  不被丢弃，不新增字段。
- 不改协议层。`ProviderSettings` 的 `r#type` 仍是二选一。

## 3. 字段语义

### 3.1 目标语义

```jsonc
{
  "providers": {
    "yunjunet": {
      "type": "openai-compatible",
      "base_url": "https://deepseek.yunjunet.cn/v1",
      "api_key": "sk-…",
      // 默认项：解析"当前该用哪个模型"时读它，语义与今天完全一致
      "selected_model": "deepseek-v4.1-flash",
      // 候选集：让模型面板知道这家有哪些名字
      "models": {
        "deepseek-v4.1-flash": { "context_window": 128000 },
        "deepseek-v4-pro":     { "alias": ["v4-pro"] },
        "glm-5.3":             {}
      }
    }
  }
}
```

关键点：**`selected_model` 的语义没有改变**。它一直都是"默认模型"，
`provider_resolution.rs:325` 读它的行为一字不动。改的只是 `models` 从"死字段"
变成"候选集"。

### 3.2 三种配置组合

| `selected_model` | `models` | 含义 | 面板表现 |
|---|---|---|---|
| 空 | 空 | 完全交给 API 自动发现 | 打开面板触发 `ListModels{force:false}`，显示"从 API 获取" |
| 空 | 有 | 用 `models` 的键，第一个为默认 | 面板直接列出，无需网络 |
| 有 | 空 | 只有一个模型，仍走自动发现（保留该项） | 同第一行 |
| 有 | 有 | `selected_model` 为默认，`models` 为候选集 | 面板列出 `selected_model` 打头 |

> "有 `selected_model` 但 `models` 为空"**不作为特例处理**：它等价于第一行。
> 这样避免了"配了一个模型名但仍然去打 API"这种看似智能实则难解释的行为。

### 3.3 顺序必须确定

`models` 是 `HashMap<String, ModelConfig>`，**迭代顺序不确定**。拿它当有序列表用会让
模型面板每次刷新换一次顺序 —— 这与 `provider_store::configured_provider_ids_from`
里已经修过的那个问题同源（`providers` 同样是 `HashMap`）。

规则（`ProviderSettings::candidate_models`，新增）：

1. 若 `selected_model` 去空白后非空 → 作为第 0 项。
2. `models` 的全部键按 id **升序**（`sort()`，字节序），跳过已在第 0 项的。
3. 两者都空 → 返回空列表，调用方按"自动发现"处理。

不引入 `Vec` 字段、不改 schema —— 顺序由"默认项优先 + 键名排序"这条规则现算，
既有配置文件（哪怕 `models` 写得毫无顺序）都能得到稳定结果。

## 4. 空密钥的加载矩阵

### 4.1 凭证状态的判定

`ProviderSettings` 的凭证来自三处，需合并判断：`api_key` 字段、环境变量
（`resolve_api_key_from_env`，见 `providers.rs:541`）、以及"是否内建且不要求 key"
（`metadata.requires_api_key`）。判定函数：

```
fn has_usable_credentials(provider_id, settings, env) -> bool
```

- 内建 provider 且 `!requires_api_key`（Ollama / LM Studio 等本地免鉴权）→ **恒为 true**。
  这条必须排在最前，否则本地 provider 会被误伤成"未配置密钥"。
- `requires_api_key` 的内建 → 有 key 才 true。
- 自定义 provider → 有 key 才 true。**注意这与 `provider_is_configured` 不同**：
  那个函数用"有 URL 就算配好"，因为它回答的是"这个 provider 该不该出现在菜单里"；
  这个函数回答的是"能不能发请求"。两者语义不同，不能合并。

### 4.2 加载行为

| 状态 | 参与模型扇出 | Provider 菜单 | 选中其模型时 |
|---|---|---|---|
| 有凭证 + 有 `models` | 是（且不必打 API） | 正常 | 正常切换 |
| 有凭证 + 无 `models` | 是 | 正常 | 正常切换 |
| **无凭证 + 有 URL** | **否** | 列出，标 `未配置密钥` | **本地立刻**报 `Provider '<id>' has no API key configured.` |
| 无凭证 + 无 URL | 否 | 不列出 | — |
| 免鉴权内建 | 是 | 正常 | 正常切换 |

设计取舍说明：

- **无凭证为什么排除出扇出**：`list_models_with_mode` 对每个 provider 发带 3s 超时的
  并发请求并 `join_all` 等最慢的。留空 key 的 provider 打过去必然 401/403，等多久
  都是失败，而且会把整次扇出的耗时拉满。排除掉，扇出只在"能成功的集合"上并发。
- **为什么不直接从 UI 隐藏**：用户明明配了 Base URL，只是还没填 key。隐藏会让
  面板凭空少一个分组，用户以为自己没配好。标出来 + 选中时立刻报错，是"能看见
  且不浪费"的最优解。
- **为什么在本地校验而不是等 401**：`apply_model_selection`
  （`agent/workflows/star_agent.rs`）已经有一层 provider 存在性校验。把凭证检查
  加在同一层，失败原因能和 provider 不存在走同一条路径，日志里也统一。

## 5. 表单改造

`ProviderFormState`（`ui/state/palette.rs:135-148`）五个字段里，`Model` 改成
`Models`，其余不动：

- 字段定义（`PROVIDER_FORM_FIELDS`，`ui/state/palette.rs:244`）的提示行改为
  `"逗号 / 换行分隔 · 第一个为默认 · 留空 = 自动获取"`。
- 提交时（`submit_provider_form`，`ui/events/input.rs:516`）按 `,` `\n` 分隔，
  逐项 `trim()`，去空，去重（保持首次出现顺序）。
  - 0 项 → `selected_model = None`，`models = None`（纯自动发现）。
  - 1 项 → `selected_model = Some(项)`，`models = None`（**不为了"看起来完整"而
    造一个单键 map**；单值场景保持与今天一致的配置形状）。
  - ≥2 项 → `selected_model = Some(首项)`，`models = Some(map)`，并把
    `ModelConfig { name, context_window, alias }` 原样带过去（编辑模式下已有值
    要保留，不要用默认覆盖）。

## 6. 顺带修掉：`provider/model` 解析的斜杠歧义

`commands/model.rs:119-129`：

```rust
let parts: Vec<&str> = model_id.splitn(2, '/').collect();
let (provider_id, model_name) = if parts.len() == 2 {
    (parts[0].to_string(), parts[1].to_string())
} // ...
```

无条件按第一个 `/` 拆。但 `opencode-zen/space-bunny-free` 这种**带命名空间的模型 id**
（OpenCode Zen / OpenRouter 风格）会被拆成 provider=`opencode-zen` +
model=`space-bunny-free`。`opencode-zen` 并非 `ALL_PROVIDERS` 里的 id，于是要么
报错，要么落回"当前 provider"，行为不可预期。

规则改为：**只有左边那一段确实是已知的 provider id（`provider_matches_id`
或 `canonical_provider_id` 命中）时才按 `provider/model` 拆；否则整串当模型名。**

这与 `resolve_provider_for_model`（`ui/events/input.rs:291`）已有的
"回退到当前 provider"是同一套归属思路，不新增概念。

## 7. 落地顺序

| 步骤 | 内容 | 风险 | 依赖 |
|---|---|---|---|
| 1 | `candidate_models()` 读侧复活 `models` + 确定顺序 | 低（纯新增函数） | — |
| 2 | 表单 `Model` → `Models` 多值 + 提示行 | 低 | 1 |
| 3 | 空凭证：不参与扇出 + 菜单标记 + 切换前本地校验 | 中（动 `provider_is_configured` 邻近逻辑，须确认免鉴权内建不被误伤） | 1 |
| 4 | `use_model` 斜杠歧义 | 低（一处解析规则） | — |

每步独立可验证、可单独提交。第 1 步做完就能让已经手写了 `models` 的配置开始生效，
不需要用户改任何配置。

## 8. 测试计划

| 测试 | 断言 |
|---|---|
| `candidate_models_puts_the_default_first` | `selected_model` 非空时排第 0 |
| `candidate_models_are_deterministically_ordered` | 同一份 `models`（HashMap）重复调用，顺序完全一致 |
| `candidate_models_skip_the_default_in_the_sorted_tail` | 尾部升序里不重复出现默认项 |
| `candidate_models_is_empty_when_nothing_is_configured` | 两者皆空 → 空列表（触发自动发现） |
| `a_single_configured_model_writes_no_models_map` | 只配一个模型时不造 `models` map |
| `a_keyless_provider_is_skipped_by_the_model_fan_out` | 无凭证的 provider 不被打到 |
| `a_keyless_local_provider_is_not_treated_as_keyless` | 免鉴权内建（Ollama）仍参与扇出 |
| `selecting_a_model_of_a_keyless_provider_fails_before_sending` | 本地报错，日志里没有网络往返痕迹 |
| `namespaced_model_ids_are_not_split_as_provider_model` | `opencode-zen/space-bunny-free` 不被误拆 |
| `an_actual_provider_prefix_is_still_split` | `<已知 provider>/model` 仍正常拆 |
