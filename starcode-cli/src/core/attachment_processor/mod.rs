/// PDF/图像处理系统
/// 
/// 对标claude-code-main的src/utils/pdf.ts和imageResizer.ts
/// 处理PDF和图像附件

use serde::{Deserialize, Serialize};

/// 附件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttachmentType {
    /// PDF文档
    Pdf,
    /// 图像
    Image,
    /// 文本
    Text,
    /// 其他
    Other,
}

/// 附件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    /// 附件ID
    pub id: String,
    /// 文件名
    pub filename: String,
    /// 文件类型
    pub file_type: AttachmentType,
    /// 文件大小（字节）
    pub size: u64,
    /// MIME类型
    pub mime_type: String,
    /// 文件路径
    pub path: Option<String>,
    /// 内容（如果是内联的）
    pub content: Option<Vec<u8>>,
    /// 提取的文本
    pub extracted_text: Option<String>,
}

/// PDF处理配置
#[derive(Debug, Clone)]
pub struct PdfConfig {
    /// 是否启用
    pub enabled: bool,
    /// 最大文件大小（字节）
    pub max_file_size: u64,
    /// 是否提取文本
    pub extract_text: bool,
    /// 最大页数
    pub max_pages: u32,
}

impl Default for PdfConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_file_size: 10 * 1024 * 1024, // 10MB
            extract_text: true,
            max_pages: 100,
        }
    }
}

/// 图像处理配置
#[derive(Debug, Clone)]
pub struct ImageConfig {
    /// 是否启用
    pub enabled: bool,
    /// 最大文件大小（字节）
    pub max_file_size: u64,
    /// 最大尺寸（宽或高）
    pub max_dimension: u32,
    /// 是否压缩
    pub compress: bool,
    /// 压缩质量（0-100）
    pub quality: u8,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_file_size: 5 * 1024 * 1024, // 5MB
            max_dimension: 2048,
            compress: true,
            quality: 80,
        }
    }
}

/// 附件处理器
pub struct AttachmentProcessor {
    pdf_config: PdfConfig,
    image_config: ImageConfig,
}

impl AttachmentProcessor {
    pub fn new(pdf_config: PdfConfig, image_config: ImageConfig) -> Self {
        Self {
            pdf_config,
            image_config,
        }
    }

    /// 处理附件
    pub fn process_attachment(&self, attachment: &AttachmentInfo) -> Result<ProcessedAttachment, String> {
        match attachment.file_type {
            AttachmentType::Pdf => self.process_pdf(attachment),
            AttachmentType::Image => self.process_image(attachment),
            AttachmentType::Text => self.process_text(attachment),
            AttachmentType::Other => Ok(ProcessedAttachment {
                id: attachment.id.clone(),
                original: attachment.clone(),
                processed_content: None,
                extracted_text: None,
                warnings: vec!["Unsupported file type".to_string()],
            }),
        }
    }

    /// 处理PDF
    fn process_pdf(&self, attachment: &AttachmentInfo) -> Result<ProcessedAttachment, String> {
        if !self.pdf_config.enabled {
            return Err("PDF processing is disabled".to_string());
        }

        if attachment.size > self.pdf_config.max_file_size {
            return Err(format!("PDF file too large: {} bytes (max: {})", attachment.size, self.pdf_config.max_file_size));
        }

        Ok(ProcessedAttachment {
            id: attachment.id.clone(),
            original: attachment.clone(),
            processed_content: None,
            extracted_text: attachment.extracted_text.clone(),
            warnings: Vec::new(),
        })
    }

    /// 处理图像
    fn process_image(&self, attachment: &AttachmentInfo) -> Result<ProcessedAttachment, String> {
        if !self.image_config.enabled {
            return Err("Image processing is disabled".to_string());
        }

        if attachment.size > self.image_config.max_file_size {
            return Err(format!("Image file too large: {} bytes (max: {})", attachment.size, self.image_config.max_file_size));
        }

        Ok(ProcessedAttachment {
            id: attachment.id.clone(),
            original: attachment.clone(),
            processed_content: attachment.content.clone(),
            extracted_text: None,
            warnings: Vec::new(),
        })
    }

    /// 处理文本
    fn process_text(&self, attachment: &AttachmentInfo) -> Result<ProcessedAttachment, String> {
        Ok(ProcessedAttachment {
            id: attachment.id.clone(),
            original: attachment.clone(),
            processed_content: attachment.content.clone(),
            extracted_text: attachment.extracted_text.clone(),
            warnings: Vec::new(),
        })
    }
}

/// 处理后的附件
#[derive(Debug, Clone)]
pub struct ProcessedAttachment {
    pub id: String,
    pub original: AttachmentInfo,
    pub processed_content: Option<Vec<u8>>,
    pub extracted_text: Option<String>,
    pub warnings: Vec<String>,
}
