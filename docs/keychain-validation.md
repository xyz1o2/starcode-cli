# macOS Keychain 验证

`KeychainStorage` 在 macOS 上使用 `security-framework` 的系统 API 写入、查询和删除，不把凭据传入进程参数。查询保留字符串首尾空白；只有 `errSecItemNotFound` 映射为缺失，其他 OSStatus 保留在错误中。

## 自动化平台门

`.github/workflows/ci.yml` 的 macOS 测试作业执行 `bash scripts/test-keychain-macos.sh`。脚本仅接受一次性 GitHub 托管 macOS runner：

- 记录当前默认钥匙串与搜索列表。
- 创建临时钥匙串，将默认和搜索列表限定到该库。
- 显式运行默认忽略的 macOS 系统回归。测试自身再次核对默认库和搜索列表。
- 退出时恢复原配置，删除测试库和临时目录。

测试覆盖系统 API 创建、更新、读取、删除、重复删除、查询缺失；旧 `security add-generic-password` 生成条目经系统 API 更新和读取；锁定临时库并禁止交互后，写入和读取返回真实系统错误。命令行里的密码均为公开测试夹具，测试不读取真实凭据。

不要在开发者登录会话中伪造 runner 环境变量运行此脚本，也不要使用登录钥匙串替代临时库。默认库/搜索列表是用户会话状态，不能仅用服务名前缀作为隔离保证。

## 验收边界

Linux 可执行错误码映射、UTF-8/空白保真和禁止写入子进程的回归；这些不替代 macOS 系统测试。只有包含本次变更的 macOS CI 作业实际通过后，才可标记自动化平台验收通过。

本次编写环境只有 Linux Rust targets，尚无本次变更的 macOS CI 运行结果。授权弹窗与已有用户登录库 ACL 的交互仍属于人工平台验收；自动测试仅给临时旧条目授权当前测试程序和 `/usr/bin/security`，不宣称覆盖任意既有登录库权限。
