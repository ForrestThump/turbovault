use crate::error::VectorError;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "local")]
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

/// Common interface for embedding models.
#[async_trait::async_trait]
pub trait EmbeddingEngine: Send + Sync {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError>;
    fn dimensions(&self) -> usize;
    fn model_name(&self) -> &str;
}

/// Local fastembed-backed embedding engine.
pub struct FastembedEngine {
    #[cfg(feature = "local")]
    model: Arc<std::sync::Mutex<TextEmbedding>>,
    dims: usize,
    model_name_str: String,
}

impl FastembedEngine {
    pub fn new(model_name: &str, cache_dir: Option<PathBuf>) -> Result<Self, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = (model_name, cache_dir);
            return Err(VectorError::Embedding(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let (embedding_model, dims) = match model_name {
                "bge-base-en-v1.5" => (EmbeddingModel::BGEBaseENV15, 768usize),
                "bge-small-en-v1.5" => (EmbeddingModel::BGESmallENV15, 384usize),
                "all-MiniLM-L6-v2" => (EmbeddingModel::AllMiniLML6V2, 384usize),
                other => {
                    return Err(VectorError::Embedding(format!(
                        "unknown model name: {other}"
                    )));
                }
            };

            let mut opts = TextInitOptions::new(embedding_model).with_show_download_progress(true);
            if let Some(dir) = cache_dir {
                opts = opts.with_cache_dir(dir);
            }

            let text_embedding =
                TextEmbedding::try_new(opts).map_err(|e| VectorError::Embedding(e.to_string()))?;

            Ok(Self {
                model: Arc::new(std::sync::Mutex::new(text_embedding)),
                dims,
                model_name_str: model_name.to_string(),
            })
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingEngine for FastembedEngine {
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = texts;
            return Err(VectorError::Embedding(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let model = self.model.clone();
            let texts_owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
            let result = tokio::task::spawn_blocking(move || {
                let mut guard = model.lock().unwrap();
                guard.embed(texts_owned, Some(32))
            })
            .await
            .map_err(|e| VectorError::Embedding(e.to_string()))?
            .map_err(|e| VectorError::Embedding(e.to_string()))?;
            Ok(result)
        }
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn model_name(&self) -> &str {
        &self.model_name_str
    }
}
