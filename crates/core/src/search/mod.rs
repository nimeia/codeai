use crate::{embedding, vectorstore};

pub fn semantic(query: &str, limit: usize) -> Vec<vectorstore::SearchHit> {
    let emb = embedding::embed(query);
    vectorstore::search(&emb, limit)
}
