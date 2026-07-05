use turbovault_core::VectorSearchConfig;

/// Resolved vector search config, derived from `VectorSearchConfig`.
#[derive(Debug, Clone)]
pub struct VectorConfig {
    pub model: String,
    pub chunk_max_chars: usize,
    pub chunk_overlap_chars: usize,
    pub rrf_k: f64,
    pub bm25_weight: f32,
    pub auto_update: bool,
    pub incremental_granularity: String,
    pub model_cache_dir: Option<std::path::PathBuf>,
    pub index_quantization: String,
    pub search_overfetch_factor: usize,
    pub min_similarity: f32,
    pub rerank_enabled: bool,
    pub rerank_model: String,
}

impl From<&VectorSearchConfig> for VectorConfig {
    fn from(c: &VectorSearchConfig) -> Self {
        Self {
            model: c.model.clone(),
            chunk_max_chars: c.chunk_max_chars,
            chunk_overlap_chars: c.chunk_overlap_chars,
            rrf_k: c.rrf_k,
            bm25_weight: c.bm25_weight,
            auto_update: c.auto_update,
            incremental_granularity: c.incremental_granularity.clone(),
            model_cache_dir: if c.model_cache_dir.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(&c.model_cache_dir))
            },
            index_quantization: c.index_quantization.clone(),
            search_overfetch_factor: c.search_overfetch_factor,
            min_similarity: c.min_similarity,
            rerank_enabled: c.rerank_enabled,
            rerank_model: c.rerank_model.clone(),
        }
    }
}
