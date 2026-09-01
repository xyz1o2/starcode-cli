use async_trait::async_trait;

#[async_trait]
pub trait FileSystemService: Send + Sync {
    async fn read_text_file(&self, file_path: &str) -> Result<String, Box<dyn std::error::Error>>;
    async fn write_text_file(
        &self,
        file_path: &str,
        content: &str,
    ) -> Result<(), Box<dyn std::error::Error>>;
}

pub struct StandardFileSystemService;

#[async_trait]
impl FileSystemService for StandardFileSystemService {
    async fn read_text_file(&self, file_path: &str) -> Result<String, Box<dyn std::error::Error>> {
        crate::core::utils::file_utils::read_file_with_encoding_async(std::path::Path::new(file_path))
            .await
            .map_err(|e| e.into())
    }

    async fn write_text_file(
        &self,
        file_path: &str,
        content: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        tokio::fs::write(file_path, content)
            .await
            .map_err(|e| e.into())
    }
}
