use crate::types::StarMessage;

/// Media Recovery配置
#[derive(Debug, Clone)]
pub struct MediaRecoveryConfig {
    /// 是否启用
    pub enabled: bool,
    /// 最大重试次数
    pub max_retries: usize,
    /// 最大图片大小（字节）
    pub max_image_size: usize,
    /// 最大PDF大小（字节）
    pub max_pdf_size: usize,
    /// 图片压缩质量（1-100）
    pub image_quality: u32,
    /// 是否自动调整图片大小
    pub auto_resize: bool,
}

impl Default for MediaRecoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 1,
            max_image_size: 10 * 1024 * 1024, // 10MB
            max_pdf_size: 30 * 1024 * 1024,   // 30MB
            image_quality: 80,
            auto_resize: true,
        }
    }
}

impl MediaRecoveryConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_MEDIA_RECOVERY_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let max_retries = std::env::var("STAR_MEDIA_RECOVERY_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        let max_image_size = std::env::var("STAR_MEDIA_MAX_IMAGE_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10 * 1024 * 1024);

        let max_pdf_size = std::env::var("STAR_MEDIA_MAX_PDF_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30 * 1024 * 1024);

        let image_quality = std::env::var("STAR_MEDIA_IMAGE_QUALITY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80);

        let auto_resize = std::env::var("STAR_MEDIA_AUTO_RESIZE")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        Self {
            enabled,
            max_retries,
            max_image_size,
            max_pdf_size,
            image_quality,
            auto_resize,
        }
    }
}

/// Media错误类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// 图片过大
    ImageTooLarge { size: usize, max_size: usize },
    /// PDF过大
    PdfTooLarge { size: usize, max_size: usize },
    /// 图片格式不支持
    UnsupportedImageFormat { format: String },
    /// PDF损坏
    CorruptedPdf,
    /// 其他媒体错误
    Other(String),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::ImageTooLarge { size, max_size } => {
                write!(
                    f,
                    "Image too large: {} bytes (max: {} bytes)",
                    size, max_size
                )
            }
            MediaError::PdfTooLarge { size, max_size } => {
                write!(f, "PDF too large: {} bytes (max: {} bytes)", size, max_size)
            }
            MediaError::UnsupportedImageFormat { format } => {
                write!(f, "Unsupported image format: {}", format)
            }
            MediaError::CorruptedPdf => write!(f, "Corrupted PDF file"),
            MediaError::Other(msg) => write!(f, "Media error: {}", msg),
        }
    }
}

/// Media Recovery管理器
pub struct MediaRecoveryManager {
    config: MediaRecoveryConfig,
    /// 重试次数
    retry_count: usize,
}

impl MediaRecoveryManager {
    pub fn new() -> Self {
        let config = MediaRecoveryConfig::from_env();
        Self {
            config,
            retry_count: 0,
        }
    }

    /// 检查是否是媒体错误
    pub fn is_media_error(&self, error: &str) -> bool {
        let error_lower = error.to_lowercase();
        error_lower.contains("image_too_large")
            || error_lower.contains("image_size")
            || error_lower.contains("pdf_too_large")
            || error_lower.contains("pdf_size")
            || error_lower.contains("media_size")
            || error_lower.contains("file_too_large")
            || error_lower.contains("image_too_large")
            || error_lower.contains("maximum_image_size")
            || error_lower.contains("maximum_pdf_size")
    }

    /// 解析媒体错误
    pub fn parse_media_error(&self, error: &str) -> MediaError {
        let error_lower = error.to_lowercase();

        if error_lower.contains("image_too_large") || error_lower.contains("image_size") {
            MediaError::ImageTooLarge {
                size: 0, // 实际大小需要从错误消息中解析
                max_size: self.config.max_image_size,
            }
        } else if error_lower.contains("pdf_too_large") || error_lower.contains("pdf_size") {
            MediaError::PdfTooLarge {
                size: 0,
                max_size: self.config.max_pdf_size,
            }
        } else if error_lower.contains("unsupported_format") {
            MediaError::UnsupportedImageFormat {
                format: "unknown".to_string(),
            }
        } else if error_lower.contains("corrupted") {
            MediaError::CorruptedPdf
        } else {
            MediaError::Other(error.to_string())
        }
    }

    /// 尝试恢复
    pub fn try_recovery(&mut self, error: &str) -> MediaRecoveryDecision {
        if !self.config.enabled {
            return MediaRecoveryDecision::NoRecovery {
                reason: "Media recovery disabled".to_string(),
            };
        }

        if self.retry_count >= self.config.max_retries {
            return MediaRecoveryDecision::NoRecovery {
                reason: format!(
                    "Max retries reached ({}/{})",
                    self.retry_count, self.config.max_retries
                ),
            };
        }

        let media_error = self.parse_media_error(error);
        self.retry_count += 1;

        match media_error {
            MediaError::ImageTooLarge { .. } => {
                if self.config.auto_resize {
                    MediaRecoveryDecision::RetryWithResize {
                        target_size: self.config.max_image_size,
                        quality: self.config.image_quality,
                    }
                } else {
                    MediaRecoveryDecision::NoRecovery {
                        reason: "Auto-resize disabled".to_string(),
                    }
                }
            }
            MediaError::PdfTooLarge { .. } => MediaRecoveryDecision::NoRecovery {
                reason: "PDF resize not supported".to_string(),
            },
            _ => MediaRecoveryDecision::NoRecovery {
                reason: format!("Unsupported media error: {}", media_error),
            },
        }
    }

    /// 重置状态
    pub fn reset(&mut self) {
        self.retry_count = 0;
    }

    /// 获取配置
    pub fn config(&self) -> &MediaRecoveryConfig {
        &self.config
    }
}

/// Media Recovery决策
#[derive(Debug, Clone)]
pub enum MediaRecoveryDecision {
    /// 重试并调整大小
    RetryWithResize { target_size: usize, quality: u32 },
    /// 不恢复
    NoRecovery { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_media_error() {
        let manager = MediaRecoveryManager::new();
        assert!(manager.is_media_error("Error: image_too_large"));
        assert!(manager.is_media_error("pdf_size_exceeded"));
        assert!(!manager.is_media_error("invalid_api_key"));
    }

    #[test]
    fn test_parse_media_error() {
        let manager = MediaRecoveryManager::new();

        let error = manager.parse_media_error("Error: image_too_large");
        assert!(matches!(error, MediaError::ImageTooLarge { .. }));

        let error = manager.parse_media_error("Error: pdf_too_large");
        assert!(matches!(error, MediaError::PdfTooLarge { .. }));
    }
}
