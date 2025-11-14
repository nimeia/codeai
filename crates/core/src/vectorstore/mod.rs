use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchHit {
    pub file: String,
    pub line: u32,
    pub score: f32,
}

pub fn search(_query: &[f32], limit: usize) -> Vec<SearchHit> {
    (0..limit)
        .map(|i| SearchHit {
            file: format!("file_{i}.rs"),
            line: i as u32,
            score: 0.0,
        })
        .collect()
}
