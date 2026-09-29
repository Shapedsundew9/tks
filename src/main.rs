//! CLI application entry point.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tks::run().await
}
