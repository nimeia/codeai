pub mod embedding;
pub mod indexer;
pub mod metadata;
pub mod search;
pub mod vectorstore;
pub mod watcher;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub project_root: String,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            project_root: String::from("."),
        }
    }
}
