#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    starcode_cli::entry::run().await
}
