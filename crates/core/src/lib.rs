pub mod indexer;
pub mod watcher;
pub mod metadata;
pub mod embedding;
pub mod vectorstore;
pub mod search;

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
