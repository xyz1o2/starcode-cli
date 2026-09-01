/// SSH认证代理

use super::SSHAuthMethod;

/// SSH认证代理
pub struct SSHAuthProxy;

impl SSHAuthProxy {
    /// 创建新的SSH认证代理
    pub fn new() -> Self {
        Self
    }

    /// 获取认证方式
    pub fn get_auth_method(&self, host: &str) -> Option<SSHAuthMethod> {
        // TODO: 实现认证方式获取
        None
    }

    /// 保存认证信息
    pub fn save_auth(&self, host: &str, auth_method: &SSHAuthMethod) -> Result<(), String> {
        // TODO: 实现认证信息保存
        Ok(())
    }
}
