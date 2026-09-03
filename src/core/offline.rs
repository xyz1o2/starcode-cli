use std::sync::atomic::{AtomicBool, Ordering};

/// 全局离线开关：LLM 发送入口与 Web 工具（WebSearch/WebFetch）在 offline 时拒绝
/// 请求。用 AtomicBool 而非 env var —— 进程全局、可测试、与 UI 状态对称。
pub static NETWORK_OFFLINE: AtomicBool = AtomicBool::new(false);

/// 是否处于离线模式
pub fn is_offline() -> bool {
    NETWORK_OFFLINE.load(Ordering::Relaxed)
}

/// 设置离线模式
pub fn set_offline(on: bool) {
    NETWORK_OFFLINE.store(on, Ordering::Relaxed);
}
