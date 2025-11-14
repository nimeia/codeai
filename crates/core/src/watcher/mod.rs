use anyhow::Result;

pub fn start() -> Result<()> {
    tracing::debug!("watcher initialized");
    Ok(())
}
