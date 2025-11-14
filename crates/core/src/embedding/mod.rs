pub fn embed(text: &str) -> Vec<f32> {
    tracing::trace!(len = text.len(), "embedding placeholder");
    vec![0.0; text.len().min(4)]
}
