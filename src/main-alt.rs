use packs::packs::cli;
#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    cli::run().await
}
