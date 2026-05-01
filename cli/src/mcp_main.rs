#[tokio::main]
async fn main() -> anyhow::Result<()> {
    yourmemory_mcp::run().await
}
