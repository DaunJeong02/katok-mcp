use crate::{config::KatokConfig, Error, Result};
#[cfg(feature = "semantic")]
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use sha2::{Digest, Sha256};

pub(crate) trait SemanticEmbedder {
    fn id(&self) -> &'static str;
    fn embed(&mut self, texts: &[String], batch_size: usize) -> Result<Vec<Vec<f32>>>;
    fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        let embeddings = self.embed(&[query.to_string()], 1)?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| Error::Embedding("embedder returned no query vector".to_string()))
    }
}

/// Whether `KATOK_EMBEDDER` selects the deterministic test embedder, which
/// needs no model and so works in every build.
fn test_embedder_requested() -> bool {
    matches!(
        std::env::var("KATOK_EMBEDDER").ok().as_deref(),
        Some("mock" | "local-test")
    )
}

/// Fail before any work when this build cannot embed at all.
///
/// Without the `semantic` feature there is no local model, so `index` and
/// `search semantic` would otherwise get as far as staging a generation before
/// discovering that. The test embedder stays usable, which keeps the synthetic
/// suite runnable against a slim build.
pub fn ensure_embedder_available() -> Result<()> {
    if cfg!(feature = "semantic") || test_embedder_requested() {
        Ok(())
    } else {
        Err(Error::SemanticFeatureDisabled)
    }
}

pub(crate) fn create_embedder(config: &KatokConfig) -> Result<Box<dyn SemanticEmbedder>> {
    if test_embedder_requested() {
        return Ok(Box::new(DeterministicEmbedder::new(usize::from(
            config.vector_dimension,
        ))));
    }
    real_embedder(config)
}

#[cfg(feature = "semantic")]
fn real_embedder(config: &KatokConfig) -> Result<Box<dyn SemanticEmbedder>> {
    Ok(Box::new(FastEmbedder::new(config)?))
}

#[cfg(not(feature = "semantic"))]
fn real_embedder(_config: &KatokConfig) -> Result<Box<dyn SemanticEmbedder>> {
    Err(Error::SemanticFeatureDisabled)
}

/// Where fastembed keeps the model. fastembed's own default is `.fastembed_cache` in the current
/// directory, which re-downloads the model for every working directory (an MCP server or a
/// launchd job starts in an arbitrary one) and drops a model folder into whatever repo that is.
/// An explicit `FASTEMBED_CACHE_DIR` still wins; otherwise use the per-user cache directory.
#[cfg(feature = "semantic")]
fn model_cache_dir(
    env_override: Option<&std::ffi::OsStr>,
    user_cache: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if env_override.is_some_and(|value| !value.is_empty()) {
        return None;
    }
    user_cache.map(|dir| dir.join("katok").join("fastembed"))
}

#[cfg(feature = "semantic")]
struct FastEmbedder {
    inner: TextEmbedding,
}

#[cfg(feature = "semantic")]
impl FastEmbedder {
    fn new(config: &KatokConfig) -> Result<Self> {
        if config.embedder_model != crate::config::DEFAULT_EMBEDDER_MODEL {
            return Err(Error::Embedding(format!(
                "unsupported local embedder model: {}",
                config.embedder_model
            )));
        }
        let mut options = TextInitOptions::new(EmbeddingModel::EmbeddingGemma300MQ4)
            .with_show_download_progress(false);
        if let Some(dir) = model_cache_dir(
            std::env::var_os("FASTEMBED_CACHE_DIR").as_deref(),
            dirs::cache_dir(),
        ) {
            options = options.with_cache_dir(dir);
        }
        let inner = TextEmbedding::try_new(options).map_err(to_embedding_error)?;
        Ok(Self { inner })
    }
}

#[cfg(feature = "semantic")]
impl SemanticEmbedder for FastEmbedder {
    fn id(&self) -> &'static str {
        "embeddinggemma/local"
    }

    fn embed(&mut self, texts: &[String], batch_size: usize) -> Result<Vec<Vec<f32>>> {
        let documents = texts
            .iter()
            .map(|text| format!("passage: {text}"))
            .collect::<Vec<_>>();
        self.inner
            .embed(documents, Some(batch_size.max(1)))
            .map_err(to_embedding_error)
    }

    fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        let embeddings = self
            .inner
            .embed([format!("query: {}", query.trim())], Some(1))
            .map_err(to_embedding_error)?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| Error::Embedding("embedder returned no query vector".to_string()))
    }
}

struct DeterministicEmbedder {
    dimension: usize,
}

impl DeterministicEmbedder {
    const fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

impl SemanticEmbedder for DeterministicEmbedder {
    fn id(&self) -> &'static str {
        "embeddinggemma/local-test"
    }

    fn embed(&mut self, texts: &[String], _batch_size: usize) -> Result<Vec<Vec<f32>>> {
        texts
            .iter()
            .map(|text| deterministic_vector(text, self.dimension))
            .collect()
    }
}

fn deterministic_vector(text: &str, dimension: usize) -> Result<Vec<f32>> {
    if dimension == 0 {
        return Err(Error::Embedding(
            "embedding dimension must be nonzero".to_string(),
        ));
    }
    let mut vector = vec![0.0_f32; dimension];
    for term in text.split_whitespace() {
        let hash = Sha256::digest(term.as_bytes());
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&hash[..8]);
        let dimension =
            u64::try_from(dimension).map_err(|error| Error::Embedding(error.to_string()))?;
        let index = usize::try_from(u64::from_le_bytes(bytes) % dimension)
            .map_err(|error| Error::Embedding(error.to_string()))?;
        vector[index] += 1.0;
    }
    normalize(&mut vector);
    Ok(vector)
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

#[cfg(feature = "semantic")]
fn to_embedding_error(error: impl std::fmt::Display) -> Error {
    Error::Embedding(error.to_string())
}

#[cfg(all(test, feature = "semantic"))]
mod tests {
    use super::model_cache_dir;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn model_cache_defaults_to_the_user_cache_dir_not_the_working_directory() {
        let dir = model_cache_dir(None, Some(PathBuf::from("/cache")));
        assert_eq!(dir, Some(PathBuf::from("/cache/katok/fastembed")));
        assert_eq!(
            model_cache_dir(Some(OsStr::new("")), Some(PathBuf::from("/cache"))),
            Some(PathBuf::from("/cache/katok/fastembed"))
        );
    }

    #[test]
    fn explicit_fastembed_cache_dir_and_missing_user_cache_leave_fastembed_default() {
        assert_eq!(
            model_cache_dir(Some(OsStr::new("/x")), Some(PathBuf::from("/cache"))),
            None
        );
        assert_eq!(model_cache_dir(None, None), None);
    }
}
