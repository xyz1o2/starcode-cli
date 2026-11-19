/// 钥匙串存储
///
/// 对标claude-code-main的macOsKeychainStorage.ts
use super::types::{SecureStorage, StorageError};

/// `errSecItemNotFound` 的 OSStatus 值。
///
/// `security` 用它表示"钥匙串里没有这一条"。进程退出状态只保留低 8 位，
/// 所以这个值落到退出码上就是 `44`（见 [`ITEM_NOT_FOUND_EXIT_CODES`]）。
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// 明确表示"条目不存在"的退出码。
///
/// `security` 以 `exit(errSecItemNotFound)` 上报"没有这一条"，而进程退出
/// 状态只保留低 8 位，所以落到退出码上就是 `44`
/// （`(-25300 as u8)`）。
///
/// 这是**状态码**判定，不是文本匹配 —— 文本匹配会把
/// "keychain not found" 之类的无关 stderr 也误判成"条目不存在"。
const ITEM_NOT_FOUND_EXIT_CODES: &[i32] = &[ERR_SEC_ITEM_NOT_FOUND as u8 as i32];

/// `security` 子进程一次调用的结果。
///
/// 单独抽出来是为了让"退出码 → 结论"的映射可以在任何平台纯解析测试，
/// 不需要真的去调 `security`（也不需要 macOS 机器）。
#[derive(Debug)]
struct SecurityCall {
    /// 退出码；被信号杀死时为 `None`。
    exit_code: Option<i32>,
    /// 标准输出。
    stdout: String,
    /// 标准错误。
    stderr: String,
}

impl SecurityCall {
    /// 从真实的子进程输出构造。
    fn from_output(output: &std::process::Output) -> Self {
        Self {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        }
    }

    /// 是否成功退出。
    fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// 这次失败是不是明确的"条目不存在"。
///
/// 只有 `errSecItemNotFound` 对应的退出码才算；被信号杀死、其他退出码
/// 一律不算 —— 权限拒绝、钥匙串不可达都落在这里。
fn is_item_not_found(call: &SecurityCall) -> bool {
    match call.exit_code {
        Some(code) => ITEM_NOT_FOUND_EXIT_CODES.contains(&code),
        None => false,
    }
}

/// 失败原因的可读描述：退出状态 + stderr。
///
/// stderr 可能为空（并非所有失败都写 stderr），这时只报退出状态也足够定位。
fn failure_reason(call: &SecurityCall) -> String {
    let status = match call.exit_code {
        Some(code) => format!("exit code {code}"),
        None => "terminated by signal".to_string(),
    };
    let stderr = call.stderr.trim();
    if stderr.is_empty() {
        format!("security failed ({status})")
    } else {
        format!("security failed ({status}): {stderr}")
    }
}

/// 解析 `security find-generic-password` 的结果。
///
/// 只有明确的"条目不存在"状态映射成 `None`；权限拒绝、钥匙串不可达等
/// 其余失败一律保留原因（`KeychainError`），不能一律当成"不存在" ——
/// 否则上层（`/local-vault get`）会把"没权限读"报成"条目不在库里"。
fn interpret_get(call: &SecurityCall) -> Result<Option<String>, StorageError> {
    if call.succeeded() {
        return Ok(Some(call.stdout.trim().to_string()));
    }
    if is_item_not_found(call) {
        return Ok(None);
    }
    Err(StorageError::KeychainError(failure_reason(call)))
}

/// 解析 `security delete-generic-password` 的结果。
///
/// "条目不存在"映射成 `StorageError::NotFound`；其余失败保留原因。
fn interpret_delete(call: &SecurityCall) -> Result<(), StorageError> {
    if call.succeeded() {
        return Ok(());
    }
    if is_item_not_found(call) {
        return Err(StorageError::NotFound);
    }
    Err(StorageError::KeychainError(failure_reason(call)))
}

fn interpret_native_get(result: Result<Vec<u8>, i32>) -> Result<Option<String>, StorageError> {
    match result {
        Ok(value) => String::from_utf8(value).map(Some).map_err(|_| {
            StorageError::KeychainError("Keychain value is not valid UTF-8".to_string())
        }),
        Err(ERR_SEC_ITEM_NOT_FOUND) => Ok(None),
        Err(code) => Err(StorageError::KeychainError(format!(
            "Keychain get failed (OSStatus {code})"
        ))),
    }
}

fn interpret_native_delete(result: Result<(), i32>) -> Result<(), StorageError> {
    match result {
        Ok(()) => Ok(()),
        Err(ERR_SEC_ITEM_NOT_FOUND) => Err(StorageError::NotFound),
        Err(code) => Err(StorageError::KeychainError(format!(
            "Keychain delete failed (OSStatus {code})"
        ))),
    }
}

/// 钥匙串存储
pub struct KeychainStorage {
    /// 服务名称
    service_name: String,
}

impl KeychainStorage {
    /// 创建新的钥匙串存储
    pub fn new() -> Self {
        Self {
            service_name: "starcode".to_string(),
        }
    }
}

impl SecureStorage for KeychainStorage {
    fn store(&self, key: &str, value: &str) -> Result<(), StorageError> {
        #[cfg(target_os = "macos")]
        {
            // 与查询/删除保持相同 service/account；库在条目存在时原位更新。
            security_framework::passwords::set_generic_password(
                &self.service_name,
                key,
                value.as_bytes(),
            )
            .map_err(|error| {
                StorageError::KeychainError(format!(
                    "Keychain store failed (OSStatus {})",
                    error.code()
                ))
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(StorageError::Unsupported)
        }
    }

    fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        #[cfg(target_os = "macos")]
        {
            interpret_native_get(
                security_framework::passwords::get_generic_password(&self.service_name, key)
                    .map_err(|error| error.code()),
            )
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(StorageError::Unsupported)
        }
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        #[cfg(target_os = "macos")]
        {
            interpret_native_delete(
                security_framework::passwords::delete_generic_password(&self.service_name, key)
                    .map_err(|error| error.code()),
            )
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(StorageError::Unsupported)
        }
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        // 列出所有凭证比较复杂，这里返回空列表
        Ok(Vec::new())
    }

    fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let result = self.get(key)?;
        Ok(result.is_some())
    }

    fn is_available(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            // 检查security命令是否可用
            std::process::Command::new("security")
                .arg("--version")
                .output()
                .is_ok()
        }

        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 新增或更新秘密不得退回子进程参数，纯源码门可在非 macOS 平台运行。
    #[test]
    fn keychain_store_uses_the_native_api_without_a_subprocess() {
        let source = include_str!("keychain.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap();
        let store = implementation
            .split("fn store(")
            .nth(1)
            .unwrap()
            .split("fn get(")
            .next()
            .unwrap();
        assert!(store.contains("security_framework::passwords::set_generic_password"));
        assert!(!store.contains("std::process::Command"));
        assert!(!store.contains("add-generic-password"));
        let read_delete = implementation
            .split("fn get(")
            .nth(1)
            .unwrap()
            .split("fn list(")
            .next()
            .unwrap();
        assert!(read_delete.contains("security_framework::passwords::get_generic_password"));
        assert!(read_delete.contains("security_framework::passwords::delete_generic_password"));
        assert!(!read_delete.contains("std::process::Command"));
    }

    /// 仅由隔离 CI 脚本显式运行，未提供临时钥匙串时直接失败。
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "run scripts/test-keychain-macos.sh on a disposable GitHub-hosted macOS runner"]
    fn native_store_updates_and_remains_readable_by_existing_commands() {
        let keychain = std::env::var("STARCODE_TEST_KEYCHAIN")
            .expect("a disposable test keychain is required");
        assert_eq!(
            std::env::var("RUNNER_ENVIRONMENT").as_deref(),
            Ok("github-hosted")
        );
        let default = std::process::Command::new("security")
            .args(["default-keychain", "-d", "user"])
            .output()
            .expect("read default keychain");
        assert!(default.status.success());
        assert_eq!(
            String::from_utf8_lossy(&default.stdout)
                .trim()
                .trim_matches('"'),
            keychain
        );
        let search = std::process::Command::new("security")
            .args(["list-keychains", "-d", "user"])
            .output()
            .expect("read keychain search list");
        assert!(search.status.success());
        let search = String::from_utf8_lossy(&search.stdout);
        let paths: Vec<_> = search
            .lines()
            .map(|line| line.trim().trim_matches('"'))
            .collect();
        assert_eq!(
            paths,
            vec![keychain.as_str()],
            "only the disposable keychain may be searched"
        );
        let storage = KeychainStorage::new();
        let account = format!("starcode-native-store-test-{}", uuid::Uuid::new_v4());
        // 外层脚本始终删除整个临时库，panic 路径也不访问用户钥匙串。
        storage.store(&account, "local test value").unwrap();
        assert_eq!(
            storage.get(&account).unwrap().as_deref(),
            Some("local test value")
        );
        storage.store(&account, "updated local test value").unwrap();
        assert_eq!(
            storage.get(&account).unwrap().as_deref(),
            Some("updated local test value")
        );
        storage.delete(&account).unwrap();
        assert!(storage.get(&account).unwrap().is_none());
        assert!(matches!(
            storage.delete(&account),
            Err(StorageError::NotFound)
        ));

        // 旧版 security 写入的条目明确授权当前测试程序和 security 读取。
        // 参数里的值为公开测试夹具，不是用户秘密。
        let executable = std::env::current_exe().unwrap();
        let legacy = std::process::Command::new("security")
            .args([
                "add-generic-password",
                "-s",
                &storage.service_name,
                "-a",
                &account,
                "-w",
                "public legacy fixture",
                "-T",
                "/usr/bin/security",
                "-T",
            ])
            .arg(&executable)
            .arg(&keychain)
            .output()
            .unwrap();
        assert!(
            legacy.status.success(),
            "{}",
            String::from_utf8_lossy(&legacy.stderr)
        );
        storage
            .store(&account, "native updated legacy fixture")
            .unwrap();
        assert_eq!(
            storage.get(&account).unwrap().as_deref(),
            Some("native updated legacy fixture")
        );
        storage.delete(&account).unwrap();

        // 锁定临时库并禁止本进程弹窗，验证真实系统拒绝不会被吞成成功。
        storage.store(&account, "public locked fixture").unwrap();
        let interaction =
            security_framework::os::macos::keychain::SecKeychain::disable_user_interaction()
                .unwrap();
        let locked = std::process::Command::new("security")
            .arg("lock-keychain")
            .arg(&keychain)
            .output()
            .unwrap();
        assert!(locked.status.success());
        let failure = storage
            .store(&account, "must not be stored")
            .expect_err("locked keychain must reject writes");
        assert!(reason(&failure).contains("OSStatus"));
        let failure = storage
            .get(&account)
            .expect_err("locked item must not look missing");
        assert!(reason(&failure).contains("OSStatus"));
        let failure = storage
            .delete(&account)
            .expect_err("locked item deletion must report the system error");
        assert!(reason(&failure).contains("OSStatus"));
        // 清理不走用户钥匙串；外层脚本删除整个临时库。
        drop(interaction);
    }

    #[test]
    fn native_results_preserve_values_and_distinguish_errors() {
        let value = "  value with whitespace\n";
        assert_eq!(
            interpret_native_get(Ok(value.as_bytes().to_vec()))
                .unwrap()
                .as_deref(),
            Some(value)
        );
        assert!(interpret_native_get(Err(ERR_SEC_ITEM_NOT_FOUND))
            .unwrap()
            .is_none());
        assert!(matches!(
            interpret_native_delete(Err(ERR_SEC_ITEM_NOT_FOUND)),
            Err(StorageError::NotFound)
        ));
        for code in [-25293, -25308, -25291] {
            assert!(
                reason(&interpret_native_get(Err(code)).unwrap_err()).contains(&code.to_string())
            );
            assert!(reason(&interpret_native_delete(Err(code)).unwrap_err())
                .contains(&code.to_string()));
        }
        assert!(interpret_native_get(Ok(vec![0xff])).is_err());
        assert!(interpret_native_delete(Ok(())).is_ok());
    }

    /// 构造一次 `security` 调用的结果。
    fn call(exit_code: Option<i32>, stdout: &str, stderr: &str) -> SecurityCall {
        SecurityCall {
            exit_code,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    /// 取出 `KeychainError` 里的原因文本。
    fn reason(err: &StorageError) -> String {
        match err {
            StorageError::KeychainError(msg) => msg.clone(),
            other => panic!("期望 KeychainError，实际是 {other:?}"),
        }
    }

    /// 退出码常量必须与 OSStatus 对得上：`errSecItemNotFound`（-25300）落到
    /// 进程退出状态上只剩低 8 位，即 `44`。
    ///
    /// 这条测试锁住常量推导 —— 常量写错的话，"条目不存在"就会退化成
    /// "所有失败都算不存在"，正好是本次要修的缺陷。
    #[test]
    fn the_not_found_exit_codes_match_the_osstatus() {
        assert_eq!(ERR_SEC_ITEM_NOT_FOUND, -25300);
        assert_eq!(ERR_SEC_ITEM_NOT_FOUND as u8 as i32, 44);
        assert_eq!(ITEM_NOT_FOUND_EXIT_CODES, &[44]);
    }

    /// 读成功：返回值就是 stdout（去掉尾部换行）。
    #[test]
    fn a_successful_get_returns_the_value() {
        let out = interpret_get(&call(Some(0), "s3cret\n", "")).expect("成功不应报错");
        assert_eq!(out.as_deref(), Some("s3cret"));
    }

    /// 明确的"条目不存在"才映射成 `None`。
    #[test]
    fn an_explicit_item_not_found_is_none() {
        for code in ITEM_NOT_FOUND_EXIT_CODES.iter().copied() {
            let out = interpret_get(&call(Some(code), "", "security: item not found"))
                .expect("条目不存在不是错误");
            assert!(out.is_none(), "退出码 {code} 表示条目不存在，应映射成 None");
        }
    }

    /// 权限拒绝不能被当成"不存在" —— 原因必须留着。
    #[test]
    fn a_permission_denial_keeps_its_reason() {
        let err = interpret_get(&call(
            Some(1),
            "",
            "security: SecKeychainSearchCopyNext: User interaction is not allowed.",
        ))
        .expect_err("权限拒绝必须报错");
        let msg = reason(&err);
        assert!(
            msg.contains("User interaction is not allowed"),
            "stderr 原因必须保留：{msg}"
        );
        assert!(msg.contains("exit code 1"), "退出状态必须带上：{msg}");
    }

    /// 其他非零退出码同样保留原因。
    #[test]
    fn other_failures_keep_their_reason() {
        let err = interpret_get(&call(Some(51), "", "security: some other failure"))
            .expect_err("其他失败必须报错");
        let msg = reason(&err);
        assert!(
            msg.contains("some other failure"),
            "stderr 原因必须保留：{msg}"
        );
        assert!(msg.contains("exit code 51"), "退出状态必须带上：{msg}");
    }

    /// stderr 为空时也要有可诊断信息，不能只给一个空字符串。
    #[test]
    fn a_failure_without_stderr_still_names_the_exit_code() {
        let err = interpret_get(&call(Some(128), "", "")).expect_err("失败必须报错");
        let msg = reason(&err);
        assert!(msg.contains("exit code 128"), "必须报出退出状态：{msg}");
        assert!(!msg.trim().is_empty(), "错误信息不能为空");
    }

    /// 被信号杀死不是"条目不存在"。
    #[test]
    fn a_signaled_call_is_not_a_missing_item() {
        let err = interpret_get(&call(None, "", "")).expect_err("信号终止必须报错");
        let msg = reason(&err);
        assert!(msg.contains("signal"), "必须说明是被信号终止：{msg}");
    }

    /// 判定只看退出码，不看 stderr 文本 —— "keychain not found" 这类
    /// 文本不得被当成"条目不存在"。
    #[test]
    fn the_stderr_text_never_decides_whether_the_item_is_missing() {
        let err = interpret_get(&call(
            Some(1),
            "",
            "security: keychain not found, or the keychain is locked",
        ))
        .expect_err("只有状态码才算数");
        assert!(
            !reason(&err).contains("item not found"),
            "文本里的 not found 不能改变结论"
        );
    }

    /// 删成功。
    #[test]
    fn a_successful_delete_is_ok() {
        interpret_delete(&call(Some(0), "", "")).expect("删除成功不应报错");
    }

    /// 删"条目不存在" → `NotFound`。
    #[test]
    fn deleting_a_missing_item_is_not_found() {
        for code in ITEM_NOT_FOUND_EXIT_CODES.iter().copied() {
            let err = interpret_delete(&call(Some(code), "", "")).expect_err("条目不存在");
            assert!(
                matches!(err, StorageError::NotFound),
                "退出码 {code} 应映射成 NotFound，实际 {err:?}"
            );
        }
    }

    /// 删失败（权限等）保留原因，不再一律泛化成 `NotFound`。
    #[test]
    fn a_failed_delete_keeps_its_reason() {
        let err = interpret_delete(&call(
            Some(1),
            "",
            "security: SecKeychainItemDelete: User interaction is not allowed.",
        ))
        .expect_err("权限拒绝必须报错");
        assert!(
            matches!(err, StorageError::KeychainError(_)),
            "权限拒绝不能报成 NotFound，实际 {err:?}"
        );
        assert!(reason(&err).contains("User interaction is not allowed"));
    }
}
