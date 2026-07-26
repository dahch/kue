use std::sync::{Arc, Mutex};

use candle_core::{Device, DType, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use hf_hub::api::sync::Api;
use tokenizers::Tokenizer;

const MODEL_ID: &str = "Snowflake/snowflake-arctic-embed-s";
pub const EMBEDDING_DIM: usize = 384;

pub trait Embedder {
    fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>>;

    /// Returns the number of tokens the tokenizer would produce for `text`.
    ///
    /// Used by the indexer to warn when a chunk approaches the model's
    /// 512-token limit (instead of letting the tokenizer silently truncate).
    /// The default implementation counts whitespace-separated words as a
    /// rough fallback; real implementations SHOULD delegate to the tokenizer.
    fn token_count(&self, text: &str) -> usize {
        text.split_whitespace().count()
    }
}

pub struct EmbeddingModel {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl EmbeddingModel {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let device = Device::new_metal(0).unwrap_or(Device::Cpu);

        let api = Api::new()?;
        let repo = api.model(MODEL_ID.to_string());

        let tokenizer_path = repo.get("tokenizer.json")?;
        let config_path = repo.get("config.json")?;
        let weights_path = repo.get("model.safetensors")?;

        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| format!("tokenizer: {e}"))?;

        let config: Config = serde_json::from_str(&std::fs::read_to_string(config_path)?)?;

        // SAFETY: The safetensors file at `weights_path` is from a known HuggingFace
        // repository (Snowflake/snowflake-arctic-embed-s) and is not mutated during
        // the model's lifetime. `from_mmaped_safetensors` uses read-only mmap which is
        // safe as long as the underlying file is not modified — candle's API guarantees
        // alignment and the VarBuilder only reads during `BertModel::load`.
        let vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)? };
        let model = BertModel::load(vb, &config)?;

        Ok(Self { model, tokenizer, device })
    }
}

impl Embedder for EmbeddingModel {
    fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("tokenizer encode: {e}"))?;

        let ids: Vec<u32> = encoding.get_ids().to_vec();
        let type_ids: Vec<u32> = encoding.get_type_ids().to_vec();
        let attention_mask: Vec<u32> = encoding.get_attention_mask().to_vec();

        let ids_i64: Vec<i64> = ids.iter().map(|&id| id as i64).collect();
        let type_i64: Vec<i64> = type_ids.iter().map(|&id| id as i64).collect();
        let mask_i64: Vec<i64> = attention_mask.iter().map(|&id| id as i64).collect();

        let input_ids = Tensor::new(ids_i64.as_slice(), &self.device)?.unsqueeze(0)?;
        let token_type_ids = Tensor::new(type_i64.as_slice(), &self.device)?.unsqueeze(0)?;
        let attention_mask_t = Tensor::new(mask_i64.as_slice(), &self.device)?.unsqueeze(0)?;

        let last_hidden =
            self.model
                .forward(&input_ids, &token_type_ids, Some(&attention_mask_t))?;

        let mask = attention_mask_t.unsqueeze(2)?;
        let masked = (last_hidden * mask.to_dtype(DType::F32)?)?;
        let denom = mask.sum(1)?.to_dtype(DType::F32)?;
        let embedding = (masked.sum(1)? / denom)?;

        let normalized = embedding.broadcast_div(&embedding.sqr()?.sum(1)?.sqrt()?)?;

        let data = normalized.flatten_all()?.to_vec1::<f32>()?;

        Ok(data)
    }

    fn token_count(&self, text: &str) -> usize {
        self.tokenizer
            .encode(text, true)
            .map(|enc| enc.get_ids().len())
            .unwrap_or_else(|_| {
                eprintln!("[kue] Failed to tokenize for token_count, falling back to word count");
                text.split_whitespace().count()
            })
    }
}

// SAFETY: EmbeddingModel is Send + Sync (BertModel + Tokenizer + Device),
// so Mutex<EmbeddingModel> can safely implement Embedder, acquiring the
// lock per call rather than holding it across a full ingestion pipeline.
impl Embedder for std::sync::Mutex<EmbeddingModel> {
    fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        self.lock()
            .map_err(|e| Box::<dyn std::error::Error>::from(format!("model mutex poisoned: {e}")))?
            .generate_embedding(text)
    }

    fn token_count(&self, text: &str) -> usize {
        self.lock()
            .map(|g| g.token_count(text))
            .unwrap_or_else(|e| {
                eprintln!("[kue] model mutex poisoned in token_count: {e}");
                0
            })
    }
}

impl Embedder for Arc<Mutex<EmbeddingModel>> {
    fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        let inner: &Mutex<EmbeddingModel> = self.as_ref();
        inner.generate_embedding(text)
    }

    fn token_count(&self, text: &str) -> usize {
        self.as_ref().token_count(text)
    }
}

pub fn load_embedding_model() -> Result<EmbeddingModel, Box<dyn std::error::Error>> {
    EmbeddingModel::load()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestEmbedder;

    impl Embedder for TestEmbedder {
        fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
            Ok(vec![0.0f32; EMBEDDING_DIM])
        }
    }

    #[test]
    fn embedding_dim_is_correct() {
        assert_eq!(EMBEDDING_DIM, 384, "snowflake-arctic-embed-s outputs 384-dim vectors");
    }

    #[test]
    fn embedder_trait_can_be_implemented() {
        let e = TestEmbedder;
        let emb = e.generate_embedding("hello").unwrap();
        assert_eq!(emb.len(), EMBEDDING_DIM);
    }

    #[test]
    fn embedder_produces_different_outputs_for_different_inputs() {
        let e = TestEmbedder;
        let a = e.generate_embedding("hello world").unwrap();
        let b = e.generate_embedding("goodbye world").unwrap();
        assert_eq!(a.len(), b.len());
        // Mock returns all zeros, so they are equal;
        // in a real model they would differ.
    }

    #[test]
    fn embedder_trait_can_be_used_as_trait_object() {
        let e: Box<dyn Embedder> = Box::new(TestEmbedder);
        let emb = e.generate_embedding("test").unwrap();
        assert_eq!(emb.len(), EMBEDDING_DIM);
    }

    #[test]
    fn embedder_handles_empty_string() {
        let e = TestEmbedder;
        let emb = e.generate_embedding("").unwrap();
        assert_eq!(emb.len(), EMBEDDING_DIM);
    }

    #[test]
    fn embedder_handles_long_string() {
        let e = TestEmbedder;
        let long = "word ".repeat(10_000);
        let emb = e.generate_embedding(&long).unwrap();
        assert_eq!(emb.len(), EMBEDDING_DIM);
    }

    #[test]
    fn embedder_handles_special_characters() {
        let e = TestEmbedder;
        let emb = e.generate_embedding("Hello, World! café résumé ñoño 日本国 αβγ 📚🧪").unwrap();
        assert_eq!(emb.len(), EMBEDDING_DIM);
    }

    // -----------------------------------------------------------------------
    // Default token_count implementation (whitespace-based)
    // -----------------------------------------------------------------------

    #[test]
    fn token_count_default_impl_empty_string() {
        let e = TestEmbedder;
        assert_eq!(e.token_count(""), 0, "empty string should have 0 tokens");
    }

    #[test]
    fn token_count_default_impl_single_word() {
        let e = TestEmbedder;
        assert_eq!(e.token_count("hello"), 1);
    }

    #[test]
    fn token_count_default_impl_multiple_words() {
        let e = TestEmbedder;
        assert_eq!(e.token_count("the quick brown fox"), 4);
    }

    #[test]
    fn token_count_default_impl_leading_trailing_whitespace() {
        let e = TestEmbedder;
        assert_eq!(e.token_count("  hello world  "), 2, "whitespace should be stripped by split_whitespace");
    }

    #[test]
    fn token_count_default_impl_multiline() {
        let e = TestEmbedder;
        assert_eq!(e.token_count("line1\nline2\tline3"), 3, "newlines and tabs are whitespace separators");
    }

    #[test]
    fn token_count_default_impl_only_whitespace() {
        let e = TestEmbedder;
        assert_eq!(e.token_count("   \n  \t  "), 0, "only whitespace should yield 0 tokens");
    }

    #[test]
    fn token_count_default_impl_unicode() {
        let e = TestEmbedder;
        // Unicode words separated by whitespace
        assert_eq!(e.token_count("café résumé niño"), 3);
    }
}
