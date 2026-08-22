//! CPU admission adapter for the pinned Qwen3 reranker artifact.

use std::{fs, path::Path};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3::{Config, ModelForCausalLM};
use tokenizers::Tokenizer;

pub const QWEN_REPOSITORY: &str = "Qwen/Qwen3-Reranker-0.6B";
pub const QWEN_REVISION: &str = "e61197ed45024b0ed8a2d74b80b4d909f1255473";
pub const QWEN_MAX_TOKENS: usize = 8192;
pub const QWEN_YES_TOKEN: u32 = 9693;
pub const QWEN_NO_TOKEN: u32 = 2152;
pub const QWEN_INSTRUCTION: &str =
    "Given a web search query, retrieve relevant passages that answer the query";

const PREFIX: &str = "<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be \"yes\" or \"no\".<|im_end|>\n<|im_start|>user\n";
const SUFFIX: &str = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n";

pub struct QwenReranker {
    tokenizer: Tokenizer,
    model: ModelForCausalLM,
    device: Device,
    prefix_tokens: Vec<u32>,
    suffix_tokens: Vec<u32>,
}

impl QwenReranker {
    /// Opens the official immutable artifact without downloading or publishing it.
    pub fn open(root: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let device = Device::Cpu;
        let config: Config = serde_json::from_slice(&fs::read(root.join("config.json"))?)?;
        let tokenizer = Tokenizer::from_file(root.join("tokenizer.json"))?;
        let prefix_tokens = tokenizer.encode(PREFIX, false)?.get_ids().to_vec();
        let suffix_tokens = tokenizer.encode(SUFFIX, false)?.get_ids().to_vec();
        let yes = tokenizer.encode("yes", false)?.get_ids().to_vec();
        let no = tokenizer.encode("no", false)?.get_ids().to_vec();
        if yes != [QWEN_YES_TOKEN] || no != [QWEN_NO_TOKEN] {
            return Err(format!("unexpected yes/no tokens: yes={yes:?}, no={no:?}").into());
        }
        // SAFETY: the immutable safetensors file outlives the model and is not mutated
        // while its memory mapping is in use.
        let weights = unsafe {
            VarBuilder::from_mmaped_safetensors(
                &[root.join("model.safetensors")],
                DType::F32,
                &device,
            )?
        };
        let model = ModelForCausalLM::new(&config, weights)?;
        Ok(Self {
            tokenizer,
            model,
            device,
            prefix_tokens,
            suffix_tokens,
        })
    }

    pub fn score(
        &mut self,
        query: &str,
        document: &str,
    ) -> Result<f32, Box<dyn std::error::Error + Send + Sync>> {
        let body =
            format!("<Instruct>: {QWEN_INSTRUCTION}\n<Query>: {query}\n<Document>: {document}");
        let mut body_tokens = self.tokenizer.encode(body, false)?.get_ids().to_vec();
        let body_limit = QWEN_MAX_TOKENS
            .checked_sub(self.prefix_tokens.len() + self.suffix_tokens.len())
            .ok_or("Qwen fixed prompt exceeds maximum length")?;
        body_tokens.truncate(body_limit);

        let mut tokens = Vec::with_capacity(
            self.prefix_tokens.len() + body_tokens.len() + self.suffix_tokens.len(),
        );
        tokens.extend_from_slice(&self.prefix_tokens);
        tokens.extend_from_slice(&body_tokens);
        tokens.extend_from_slice(&self.suffix_tokens);

        self.model.clear_kv_cache();
        let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
        let logits = self
            .model
            .forward(&input, 0)?
            .squeeze(0)?
            .squeeze(0)?
            .to_dtype(DType::F32)?
            .to_vec1::<f32>()?;
        let yes = logits[QWEN_YES_TOKEN as usize];
        let no = logits[QWEN_NO_TOKEN as usize];
        if !yes.is_finite() || !no.is_finite() {
            return Err("Qwen produced non-finite yes/no logits".into());
        }
        let shift = yes.max(no);
        let yes_exp = (yes - shift).exp();
        Ok(yes_exp / (yes_exp + (no - shift).exp()))
    }
}
