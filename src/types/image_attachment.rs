//! 用户明确附加的图片以内容保存，历史恢复不依赖剪贴板临时文件。

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

/// Base64 编码后最多 5 MiB；限制读取量，避免文件被替换后无界分配。
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024 / 4 * 3;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageAttachment {
    pub media_type: String,
    pub data: String,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for ImageAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageAttachment")
            .field("media_type", &self.media_type)
            .field("encoded_bytes", &self.data.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl ImageAttachment {
    /// 仅由显式附件入口调用，不能用它扫描普通消息中的文件路径。
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take((MAX_IMAGE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err("Image exceeds the 5 MiB encoded size limit".into());
        }
        let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
        let media_type = match format {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::Jpeg => "image/jpeg",
            _ => return Err("Only PNG and JPEG attachments are supported".into()),
        };
        let reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
        let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
        if width == 0 || height == 0 || width > 2000 || height > 2000 {
            return Err("Image dimensions must be between 1 and 2000 pixels".into());
        }
        image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?;
        Ok(Self {
            media_type: media_type.into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            width,
            height,
        })
    }
}

/// 输入的解释资格在提交时冻结；旧持久数据缺少来源时只允许作为正文恢复。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputIntent {
    #[default]
    Literal,
    CommandEligible,
}

/// 普通输入与 steering 共用的数据包，文字分类不会读取图片字节。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInput {
    pub text: String,
    #[serde(default)]
    pub intent: InputIntent,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageAttachment>,
}

impl UserInput {
    /// 仅供明确的本地命令入口使用，反序列化和恢复不能隐式调用。
    pub fn command_eligible(text: String) -> Self {
        Self {
            text,
            intent: InputIntent::CommandEligible,
            images: Vec::new(),
        }
    }
}

impl From<&str> for UserInput {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl From<String> for UserInput {
    fn from(text: String) -> Self {
        Self {
            text,
            intent: InputIntent::Literal,
            images: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_attachment_survives_history_without_source_file() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 3)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let image = ImageAttachment::from_bytes(png.get_ref()).unwrap();
        assert_eq!(image.media_type, "image/png");
        let saved = serde_json::to_string(&image).unwrap();
        let restored: ImageAttachment = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored, image);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(restored.data)
                .unwrap(),
            png.into_inner()
        );
    }

    #[test]
    fn image_attachment_rejects_invalid_or_oversized_content() {
        assert!(ImageAttachment::from_bytes(b"not an image").is_err());
        assert!(ImageAttachment::from_bytes(&vec![0; MAX_IMAGE_BYTES + 1]).is_err());
    }
}
