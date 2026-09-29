use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::fmt;

/// Expected vector dimensionality for `all-MiniLM-L6-v2` per Decision D-77.
pub const TARGET_VECTOR_DIM: usize = 384;

/// Error types occurring during local vector embedding generation.
#[derive(Debug)]
pub enum LocalEmbeddingError {
    InitializationFailed(String),
    InferenceFailed(String),
    DimensionMismatch { expected: usize, actual: usize },
    EmptyInput,
}

impl fmt::Display for LocalEmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitializationFailed(msg) => {
                write!(f, "Failed to initialize embedding model: {msg}")
            }
            Self::InferenceFailed(msg) => write!(f, "Inference execution failed: {msg}"),
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "Unexpected vector dimension: expected {expected}, got {actual}"
                )
            }
            Self::EmptyInput => write!(f, "Cannot generate embedding for empty input"),
        }
    }
}

impl std::error::Error for LocalEmbeddingError {}

/// Embedded vector generator wrapping `fastembed` for CPU execution.
pub struct LocalEmbeddingGenerator {
    model: TextEmbedding,
    expected_dim: usize,
}

impl LocalEmbeddingGenerator {
    /// Initialize with the default Phase 0 / Phase 1 embedding model: `all-MiniLM-L6-v2` (384-d).
    pub fn new() -> Result<Self, LocalEmbeddingError> {
        Self::with_model(EmbeddingModel::AllMiniLML6V2, TARGET_VECTOR_DIM)
    }

    /// Initialize with a specific model and expected dimensionality.
    pub fn with_model(
        model_name: EmbeddingModel,
        expected_dim: usize,
    ) -> Result<Self, LocalEmbeddingError> {
        let options = InitOptions::new(model_name)
            .with_show_download_progress(false)
            .with_max_length(256);
        let model = TextEmbedding::try_new(options)
            .map_err(|e| LocalEmbeddingError::InitializationFailed(e.to_string()))?;
        Ok(Self {
            model,
            expected_dim,
        })
    }

    /// Expected embedding vector dimensionality (e.g. 384).
    #[allow(dead_code)]
    pub fn vector_dim(&self) -> usize {
        self.expected_dim
    }

    /// Embed a single text chunk, returning a 384-dimensional vector.
    pub fn embed_chunk(&self, text: &str) -> Result<Vec<f32>, LocalEmbeddingError> {
        if text.trim().is_empty() {
            return Err(LocalEmbeddingError::EmptyInput);
        }
        let embeddings = self
            .model
            .embed(vec![text], Some(1))
            .map_err(|e| LocalEmbeddingError::InferenceFailed(e.to_string()))?;

        let first = embeddings.into_iter().next().ok_or_else(|| {
            LocalEmbeddingError::InferenceFailed("Model returned no embeddings".into())
        })?;

        if first.len() != self.expected_dim {
            return Err(LocalEmbeddingError::DimensionMismatch {
                expected: self.expected_dim,
                actual: first.len(),
            });
        }

        Ok(first)
    }

    /// Embed a batch of text chunks, returning a vector of vectors.
    #[allow(dead_code)]
    pub fn embed_batch<S: AsRef<str> + Send + Sync>(
        &self,
        texts: &[S],
        batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>, LocalEmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let text_vec: Vec<&str> = texts.iter().map(|s| s.as_ref()).collect();
        let embeddings = self
            .model
            .embed(text_vec, batch_size)
            .map_err(|e| LocalEmbeddingError::InferenceFailed(e.to_string()))?;

        for emb in &embeddings {
            if emb.len() != self.expected_dim {
                return Err(LocalEmbeddingError::DimensionMismatch {
                    expected: self.expected_dim,
                    actual: emb.len(),
                });
            }
        }

        Ok(embeddings)
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::LocalEmbeddingGenerator;

    #[test]
    fn test_local_embedding_generator_single_and_batch() {
        let generator = LocalEmbeddingGenerator::new().expect("Failed to initialize generator");
        assert_eq!(generator.vector_dim(), 384);

        let text = "All requirement vector embeddings MUST specify fixed dimensionality of 384 dimensions.";
        let emb = generator.embed_chunk(text).expect("Failed to embed chunk");
        assert_eq!(emb.len(), 384);

        // Check non-zero embedding
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            norm > 0.9 && norm < 1.1,
            "Normalized embedding should have norm ~ 1.0, got {norm}"
        );

        // Test empty input error
        assert!(generator.embed_chunk("   ").is_err());

        // Test batch embedding
        let batch = vec![
            "REQ-01: System MUST enforce authentication.",
            "REQ-02: Dense vectors SHALL be indexed with pgvector.",
        ];
        let embeddings = generator
            .embed_batch(&batch, Some(2))
            .expect("Failed batch embedding");
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), 384);
        assert_eq!(embeddings[1].len(), 384);
    }
}
