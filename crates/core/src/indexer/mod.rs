use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexJob {
    pub full: bool,
}

pub fn run(job: IndexJob) -> Result<()> {
    if job.full {
        tracing::info!("full index requested");
    } else {
        tracing::info!("incremental index requested");
    }
    Ok(())
}
