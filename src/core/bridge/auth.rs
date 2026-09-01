/// Bridge JWT认证

use serde::{Deserialize, Serialize};

/// JWT令牌
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtToken {
    /// 令牌ID
    pub jti: String,
    /// 主题
    pub sub: String,
    /// 签发时间
    pub iat: i64,
    /// 过期时间
    pub exp: i64,
    /// 发行者
    pub iss: String,
    /// 自定义声明
    pub claims: std::collections::HashMap<String, serde_json::Value>,
}

/// JWT认证
pub struct JwtAuth {
    /// 密钥
    secret: String,
    /// 发行者
    issuer: String,
    /// 有效期（秒）
    validity_secs: i64,
}

impl JwtAuth {
    /// 创建新的JWT认证
    pub fn new(secret: String) -> Self {
        Self {
            secret,
            issuer: "starcode-bridge".to_string(),
            validity_secs: 3600, // 1小时
        }
    }

    /// 生成令牌
    pub fn generate_token(&self, subject: &str) -> Result<String, JwtError> {
        let now = chrono::Utc::now().timestamp();
        let token = JwtToken {
            jti: uuid::Uuid::new_v4().to_string(),
            sub: subject.to_string(),
            iat: now,
            exp: now + self.validity_secs,
            iss: self.issuer.clone(),
            claims: std::collections::HashMap::new(),
        };

        // 简化的JWT实现
        // 实际应该使用jsonwebtoken crate
        let payload = serde_json::to_string(&token)
            .map_err(|e| JwtError::EncodeError(e.to_string()))?;

        Ok(format!("{}.{}.{}", 
            base64::encode(&self.issuer),
            base64::encode(&payload),
            base64::encode("signature") // 简化的签名
        ))
    }

    /// 验证令牌
    pub fn verify_token(&self, token: &str) -> bool {
        // 简化的验证
        // 实际应该使用jsonwebtoken crate验证签名和过期时间
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return false;
        }

        // 解码payload
        if let Ok(payload) = base64::decode(parts[1]) {
            if let Ok(token_data) = serde_json::from_slice::<JwtToken>(&payload) {
                let now = chrono::Utc::now().timestamp();
                return token_data.exp > now && token_data.iss == self.issuer;
            }
        }

        false
    }

    /// 解析令牌
    pub fn parse_token(&self, token: &str) -> Result<JwtToken, JwtError> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(JwtError::InvalidFormat);
        }

        let payload = base64::decode(parts[1])
            .map_err(|e| JwtError::DecodeError(e.to_string()))?;

        let token_data: JwtToken = serde_json::from_slice(&payload)
            .map_err(|e| JwtError::DecodeError(e.to_string()))?;

        let now = chrono::Utc::now().timestamp();
        if token_data.exp <= now {
            return Err(JwtError::Expired);
        }

        Ok(token_data)
    }
}

/// JWT错误
#[derive(Debug)]
pub enum JwtError {
    /// 编码错误
    EncodeError(String),
    /// 解码错误
    DecodeError(String),
    /// 无效格式
    InvalidFormat,
    /// 过期
    Expired,
    /// 无效签名
    InvalidSignature,
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JwtError::EncodeError(e) => write!(f, "JWT encode error: {}", e),
            JwtError::DecodeError(e) => write!(f, "JWT decode error: {}", e),
            JwtError::InvalidFormat => write!(f, "Invalid JWT format"),
            JwtError::Expired => write!(f, "JWT token expired"),
            JwtError::InvalidSignature => write!(f, "Invalid JWT signature"),
        }
    }
}

impl std::error::Error for JwtError {}
