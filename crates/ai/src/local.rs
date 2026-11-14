//! Local GGUF inference on candle (pure Rust — keeps the no-C++ rule and
//! the door open for wasm later). CPU only for now; GPU comes from the
//! app's wgpu adapter in a later milestone.
//!
//! Both supported architectures speak ChatML — one prompt template covers
//! Qwen2.5 and SmolLM2.

use std::fs::File;
use std::path::Path;

use candle_core::quantized::gguf_file;
use candle_core::{Device, IndexOp, Tensor};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::{quantized_llama, quantized_qwen2};
use tokenizers::Tokenizer;

use crate::error::AiError;
use crate::provider::{GenOpts, Provider, ProviderInfo};
use crate::registry::{Arch, ModelEntry};

enum Weights {
    Qwen2(quantized_qwen2::ModelWeights),
    Llama(quantized_llama::ModelWeights),
}

impl Weights {
    fn forward(&mut self, ids: &Tensor, pos: usize) -> Result<Tensor, candle_core::Error> {
        match self {
            Self::Qwen2(w) => w.forward(ids, pos),
            Self::Llama(w) => w.forward(ids, pos),
        }
    }
}

/// A downloaded GGUF loaded and ready. Construction is the expensive
/// part; `complete` is then just a forward loop.
pub struct LocalGguf {
    weights: Weights,
    tokenizer: Tokenizer,
    device: Device,
    eos: u32,
    entry: &'static ModelEntry,
}

const SEED: u64 = 0xC0FFEE;

impl LocalGguf {
    /// Load the GGUF + tokenizer.json of `entry` from the model cache.
    /// All failures are `Inference` errors — a corrupt file, an OOM, a
    /// wrong-arch GGUF, all land in the status line, never in a panic.
    pub fn load(entry: &'static ModelEntry, gguf_path: &Path, tokenizer_path: &Path) -> Result<Self, AiError> {
        let device = Device::Cpu;
        let mut file = File::open(gguf_path).map_err(AiError::Io)?;
        // Malformed GGUF metadata (e.g. `general.alignment = 0`) makes
        // candle's parser panic on a division — hostile input must come
        // back as an error, not a crash.
        // Malformed GGUF metadata (e.g. `general.alignment = 0`, or a
        // zero `attention.head_count` reaching a division in the weight
        // constructors) makes candle panic — hostile input must come
        // back as an error, not a crash. The guard covers parsing *and*
        // weight construction.
        let weights = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let content = gguf_file::Content::read(&mut file).map_err(|e| AiError::Inference(format!("bad GGUF: {e}")))?;
            match entry.arch {
                Arch::Qwen2 => Ok(Weights::Qwen2(
                    quantized_qwen2::ModelWeights::from_gguf(content, &mut file, &device)
                        .map_err(|e| AiError::Inference(format!("cannot load {}: {e}", entry.file)))?,
                )),
                Arch::Llama => Ok(Weights::Llama(
                    quantized_llama::ModelWeights::from_gguf(content, &mut file, &device)
                        .map_err(|e| AiError::Inference(format!("cannot load {}: {e}", entry.file)))?,
                )),
            }
        }))
        .map_err(|_| AiError::Inference("GGUF loader panicked on malformed metadata".to_owned()))??;
        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| AiError::Inference(format!("bad tokenizer.json: {e}")))?;
        let eos = tokenizer.token_to_id("<|im_end|>").ok_or_else(|| AiError::Inference("tokenizer has no EOS token".into()))?;
        Ok(Self { weights, tokenizer, device, eos, entry })
    }

    /// ChatML wrap — Qwen2.5 and SmolLM2 both ship this template.
    fn chat(prompt: &str) -> String {
        format!(
            "<|im_start|>system\nYou are a precise translation engine. Follow the instructions exactly.<|im_end|>\n<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n",
        )
    }

    fn generate(&mut self, prompt: &str, opts: &GenOpts) -> Result<String, AiError> {
        let text = Self::chat(prompt);
        let encoding = self.tokenizer.encode(text, true).map_err(|e| AiError::Inference(format!("encode failed: {e}")))?;
        let mut tokens: Vec<u32> = encoding.get_ids().to_vec();

        let temp = if opts.temperature <= 0.02 { None } else { Some(f64::from(opts.temperature)) };
        let mut sampler = LogitsProcessor::new(SEED, temp, None);

        let mut pos = 0usize;
        let mut generated: Vec<u32> = Vec::with_capacity(opts.max_tokens.min(4096) as usize);
        let mut hit_eos = false;
        for _ in 0..opts.max_tokens {
            let input_len = tokens.len() - pos;
            let input =
                Tensor::new(&tokens[pos..], &self.device).and_then(|t| t.reshape((1, t.dims1()?))).map_err(|e| AiError::Inference(e.to_string()))?;
            let logits = self.weights.forward(&input, pos).map_err(|e| AiError::Inference(e.to_string()))?;
            pos += input_len;
            // `forward` already returns the last sequence position's
            // logits [batch, vocab] — index the batch row only.
            let logits = logits.i(0).map_err(|e| AiError::Inference(e.to_string()))?;
            let next = sampler.sample(&logits).map_err(|e| AiError::Inference(e.to_string()))?;
            if next == self.eos {
                hit_eos = true;
                break;
            }
            tokens.push(next);
            generated.push(next);
        }
        // No EOS inside the budget → the reply was cut mid-translation;
        // partial output would silently truncate the document.
        if !hit_eos {
            return Err(AiError::Inference(format!("generation hit max_tokens ({}) without EOS", opts.max_tokens)));
        }
        // Decode the generated ids through the tokenizer: byte-level BPE
        // vocab strings can't be concatenated (spaces/newlines/non-ASCII
        // bytes stay encoded, and UTF-8 can split across tokens).
        self.tokenizer.decode(&generated, true).map_err(|e| AiError::Inference(format!("decode failed: {e}")))
    }
}

impl Provider for LocalGguf {
    fn name(&self) -> String {
        format!("local: {}", self.entry.name)
    }

    fn complete(&mut self, prompt: &str, opts: &GenOpts) -> Result<String, AiError> {
        self.generate(prompt, opts).map(|s| s.trim().to_string())
    }

    fn test(&mut self) -> Result<ProviderInfo, AiError> {
        Ok(ProviderInfo { label: format!("loaded: {}", self.entry.name), models: vec![self.entry.id.to_string()] })
    }

    fn is_local(&self) -> bool {
        true
    }
}
