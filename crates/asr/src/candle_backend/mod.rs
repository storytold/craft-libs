//! Whisper inference through candle, on the CPU.
//!
//! The model files are the user's: [`WhisperEngine::load`] verifies every file against
//! [`crate::whisper_tiny`] before reading it, bounds each read by the manifest's size and checks
//! the dimensions in `config.json` before they reach an allocation or a tensor shape.
//!
//! The encoder always takes a 30-second window of 16 kHz mono audio, so a decode keeps the last
//! 30 seconds of the stream. The capture device and the worker thread stay with the app; this
//! module only turns samples into text, following Radford et al.
//! ([arXiv:2212.04356](https://arxiv.org/abs/2212.04356)).

mod mel;

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self, Config, N_FRAMES, N_SAMPLES, audio, model::Whisper};
use tokenizers::Tokenizer;

use crate::audio::{AudioBuffer, AudioError, Limits, TARGET_RATE};
use crate::manifest;
use crate::provider::{AsrError, Recognizer, Segment, State, Status};

/// The language tokens Whisper recognises. The tokenizer also knows special tokens that are not
/// languages; only these become a detection candidate or a prompt token.
const LANGUAGE_CODES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms",
    "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be",
    "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha",
    "ba", "jw", "su",
];

/// A loaded Whisper model.
pub struct WhisperEngine {
    model: Whisper,
    tokenizer: Tokenizer,
    config: Config,
    mel_filters: Vec<f32>,
    device: Device,
    sot: u32,
    transcribe: u32,
    no_ts: u32,
    eot: u32,
    no_speech: Vec<u32>,
    suppress: Vec<u32>,
    languages: Vec<(String, u32)>,
}

impl WhisperEngine {
    /// Load `model.safetensors`, `config.json` and `tokenizer.json` from `dir`.
    ///
    /// The directory may be one the user picked, so files are checked against the manifest
    /// (size and SHA-256) before anything is parsed, and every read is bounded by the recorded
    /// size.
    pub fn load(dir: &Path) -> Result<Self, AsrError> {
        let manifest = crate::whisper_tiny();
        manifest.validate().map_err(|error| AsrError::provider(format!("the model manifest is damaged: {error}")))?;
        for file in &manifest.files {
            manifest::verify_file(dir, file).map_err(|error| AsrError::provider(format!("the model files do not match the manifest: {error}")))?;
        }
        let limit = |name: &str| manifest.file(name).map_or(0, |file| file.bytes);
        let config_bytes = bounded_read(&dir.join("config.json"), limit("config.json"))?;
        let config: Config =
            serde_json::from_slice(&config_bytes).map_err(|error| AsrError::provider(format!("the model config is damaged: {error}")))?;
        validate_config(&config)?;
        let tokenizer =
            Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|error| AsrError::provider(format!("cannot read the tokenizer: {error}")))?;
        let weights = bounded_read(&dir.join("model.safetensors"), limit("model.safetensors"))?;
        if weights.is_empty() {
            return Err(AsrError::provider("the weights file is empty"));
        }
        let device = Device::Cpu;
        let variables = VarBuilder::from_buffered_safetensors(weights, whisper::DTYPE, &device)
            .map_err(|error| AsrError::provider(format!("the weights are damaged: {error}")))?;
        let model =
            Whisper::load(&variables, config.clone()).map_err(|error| AsrError::provider(format!("the model could not be built: {error}")))?;
        let special = |name: &str| -> Result<u32, AsrError> {
            tokenizer.token_to_id(name).ok_or_else(|| AsrError::provider(format!("the tokenizer has no {name}")))
        };
        let no_speech = whisper::NO_SPEECH_TOKENS.iter().filter_map(|token| tokenizer.token_to_id(token)).collect();
        let languages =
            LANGUAGE_CODES.iter().filter_map(|code| tokenizer.token_to_id(&format!("<|{code}|>")).map(|id| ((*code).to_string(), id))).collect();
        let mel_filters = mel::mel_filters(config.num_mel_bins);
        Ok(WhisperEngine {
            sot: special(whisper::SOT_TOKEN)?,
            transcribe: special(whisper::TRANSCRIBE_TOKEN)?,
            no_ts: special(whisper::NO_TIMESTAMPS_TOKEN)?,
            eot: special(whisper::EOT_TOKEN)?,
            no_speech,
            suppress: config.suppress_tokens.clone(),
            languages,
            mel_filters,
            model,
            tokenizer,
            config,
            device,
        })
    }

    /// The model's vocabulary size (for diagnostics).
    pub fn vocab_size(&self) -> usize {
        self.config.vocab_size
    }

    /// Transcribe up to 30 seconds of 16 kHz mono audio. `language` is a Whisper language token
    /// (`"en"`, `"es"`); `None` detects it from the audio.
    pub fn transcribe(&mut self, audio: &[f32], language: Option<&str>) -> Result<String, AsrError> {
        let pcm = window(audio);
        let features = self.encode(&pcm)?;
        let language = match language.map(str::trim).filter(|tag| !tag.is_empty()) {
            Some(tag) => self.language_token(tag)?,
            None => self.detect_language(&features)?,
        };
        let mut tokens = vec![self.sot, language, self.transcribe, self.no_ts];
        let mut ids: Vec<u32> = Vec::new();
        let max = self.config.max_target_positions.min(224);
        for step in 0..max {
            let input = Tensor::new(tokens.as_slice(), &self.device).map_err(tensor_error)?.unsqueeze(0).map_err(tensor_error)?;
            let hidden = self.model.decoder.forward(&input, &features, step == 0).map_err(tensor_error)?;
            let (_, sequence, _) = hidden.dims3().map_err(tensor_error)?;
            let last = hidden.narrow(1, sequence.saturating_sub(1), 1).map_err(tensor_error)?;
            let logits = self.model.decoder.final_linear(&last).map_err(tensor_error)?;
            let mut row = logits.i((0, 0)).map_err(tensor_error)?.to_vec1::<f32>().map_err(tensor_error)?;
            for id in &self.suppress {
                if let Some(slot) = row.get_mut(*id as usize) {
                    *slot = f32::NEG_INFINITY;
                }
            }
            let Some(next) = argmax(&row) else { break };
            if next == self.eot || self.no_speech.contains(&next) {
                break;
            }
            tokens.push(next);
            ids.push(next);
        }
        let text = self.tokenizer.decode(&ids, true).map_err(|error| AsrError::provider(format!("cannot decode the transcript: {error}")))?;
        Ok(text.trim().to_string())
    }

    fn language_token(&self, tag: &str) -> Result<u32, AsrError> {
        self.languages
            .iter()
            .find(|(code, _)| code == tag)
            .map(|(_, id)| *id)
            .ok_or_else(|| AsrError::provider(format!("`{tag}` is not a Whisper language")))
    }

    fn encode(&mut self, pcm: &[f32]) -> Result<Tensor, AsrError> {
        let bins = self.config.num_mel_bins;
        if bins == 0 {
            return Err(AsrError::provider("the model config has no mel bins"));
        }
        let mel = audio::pcm_to_mel(&self.config, pcm, &self.mel_filters);
        let frames = mel.len().checked_div(bins).unwrap_or(0);
        if frames < N_FRAMES {
            return Err(AsrError::provider("the audio window is too short for the model"));
        }
        let mel = Tensor::from_vec(mel, (1, bins, frames), &self.device).map_err(tensor_error)?;
        let mel = mel.narrow(2, 0, N_FRAMES).map_err(tensor_error)?;
        self.model.reset_kv_cache();
        self.model.encoder.forward(&mel, true).map_err(tensor_error)
    }

    /// Whisper's own language detection: the most likely language token after the start token.
    fn detect_language(&mut self, features: &Tensor) -> Result<u32, AsrError> {
        self.model.decoder.reset_kv_cache();
        let input = Tensor::new(&[self.sot][..], &self.device).map_err(tensor_error)?.unsqueeze(0).map_err(tensor_error)?;
        let hidden = self.model.decoder.forward(&input, features, true).map_err(tensor_error)?;
        let logits = self.model.decoder.final_linear(&hidden).map_err(tensor_error)?;
        let row = logits.i((0, 0)).map_err(tensor_error)?.to_vec1::<f32>().map_err(tensor_error)?;
        let mut best: Option<(f32, u32)> = None;
        for (_, id) in &self.languages {
            let Some(probability) = row.get(*id as usize).copied().filter(|probability| probability.is_finite()) else { continue };
            if best.is_none_or(|(top, _)| probability > top) {
                best = Some((probability, *id));
            }
        }
        best.map(|(_, id)| id).ok_or_else(|| AsrError::provider("the model recognized no language"))
    }
}

/// A [`Recognizer`] over [`WhisperEngine`]: audio accumulates until [`Recognizer::finish`], which
/// decodes the last 30 seconds as one final segment. The worker thread and the microphone stay
/// with the app.
pub struct WhisperRecognizer {
    model_dir: PathBuf,
    language: Option<String>,
    limits: Limits,
    engine: Option<WhisperEngine>,
    buffer: Vec<f32>,
    queue: VecDeque<Segment>,
    status: Status,
    started: bool,
}

impl WhisperRecognizer {
    pub fn new(model_dir: impl Into<PathBuf>, language: Option<String>, limits: Limits) -> Self {
        WhisperRecognizer {
            model_dir: model_dir.into(),
            language,
            limits,
            engine: None,
            buffer: Vec::new(),
            queue: VecDeque::new(),
            status: Status::idle(),
            started: false,
        }
    }
}

impl Recognizer for WhisperRecognizer {
    fn start(&mut self) -> Result<(), AsrError> {
        self.status = Status::starting(crate::WHISPER_TINY_ID, self.language.clone());
        match WhisperEngine::load(&self.model_dir) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.buffer.clear();
                self.queue.clear();
                self.started = true;
                self.status = Status::running(crate::WHISPER_TINY_ID, self.language.clone());
                Ok(())
            }
            Err(error) => {
                self.engine = None;
                self.started = false;
                self.status = Status::failed(error.to_string());
                Err(error)
            }
        }
    }

    fn feed(&mut self, audio: &AudioBuffer) -> Result<(), AsrError> {
        if !self.started || self.status.state != State::Running {
            return Err(AsrError::NotRunning);
        }
        audio.validate(&self.limits)?;
        let mono = audio.mono_16k();
        let total = self.buffer.len().saturating_add(mono.len());
        if total > self.limits.max_samples {
            return Err(AsrError::Audio(AudioError::TooManySamples { samples: total, max: self.limits.max_samples }));
        }
        self.buffer.extend_from_slice(&mono);
        Ok(())
    }

    fn finish(&mut self) -> Result<(), AsrError> {
        if self.started && !self.buffer.is_empty() {
            let Some(engine) = self.engine.as_mut() else {
                return Err(AsrError::provider("the model is not loaded"));
            };
            let text = match engine.transcribe(&self.buffer, self.language.as_deref()) {
                Ok(text) => text,
                Err(error) => {
                    self.status = Status::failed(error.to_string());
                    self.started = false;
                    return Err(error);
                }
            };
            let seconds = self.buffer.len() as f64 / f64::from(TARGET_RATE);
            if seconds > 0.0 {
                if self.limits.max_segments == 0 {
                    self.status = Status::failed("the segment limit is zero");
                    return Err(AsrError::SegmentLimit { max: 0 });
                }
                self.queue.push_back(Segment::final_line(0.0, seconds, text));
            }
        }
        self.buffer.clear();
        self.engine = None;
        self.started = false;
        self.status = Status::stopped();
        Ok(())
    }

    fn poll(&mut self) -> Vec<Segment> {
        self.queue.drain(..).collect()
    }

    fn status(&self) -> Status {
        self.status.clone()
    }
}

/// Copy the last 30 seconds of `audio` into a fixed window, padding the end with silence (the
/// window the encoder always takes) and clamping anything that is not a sample.
fn window(audio: &[f32]) -> Vec<f32> {
    let mut pcm = vec![0.0f32; N_SAMPLES];
    let tail = &audio[audio.len().saturating_sub(N_SAMPLES)..];
    for (slot, sample) in pcm.iter_mut().zip(tail) {
        // A NaN has no amplitude, so it becomes silence; an infinity is clamped like any other
        // out-of-range sample.
        *slot = if sample.is_nan() { 0.0 } else { sample.clamp(-1.0, 1.0) };
    }
    pcm
}

fn tensor_error(error: candle_core::Error) -> AsrError {
    AsrError::provider(format!("the model failed: {error}"))
}

fn argmax(values: &[f32]) -> Option<u32> {
    let mut best: Option<(f32, u32)> = None;
    for (index, probability) in values.iter().enumerate() {
        if !probability.is_finite() {
            continue;
        }
        if best.is_none_or(|(top, _)| *probability > top) {
            best = Some((*probability, u32::try_from(index).unwrap_or(u32::MAX)));
        }
    }
    best.map(|(_, index)| index)
}

/// Read at most `limit` bytes, refusing a file that claims more before allocating. The manifest's
/// recorded size is the limit; `0` (no manifest entry) reads nothing.
fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, AsrError> {
    if limit == 0 {
        return Err(AsrError::provider(format!("{} is not part of the model manifest", path.display())));
    }
    let meta = std::fs::metadata(path).map_err(|error| AsrError::provider(format!("cannot read {}: {error}", path.display())))?;
    if !meta.is_file() {
        return Err(AsrError::provider(format!("{} is not a file", path.display())));
    }
    if meta.len() > limit {
        return Err(AsrError::provider(format!("{} is larger than the manifest says", path.display())));
    }
    let mut file = std::fs::File::open(path).map_err(|error| AsrError::provider(format!("cannot read {}: {error}", path.display())))?;
    let mut out = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    // One byte past the limit, so a file that grew between `metadata` and the read is refused
    // rather than truncated silently.
    file.by_ref()
        .take(limit.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|error| AsrError::provider(format!("cannot read {}: {error}", path.display())))?;
    if out.len() as u64 > limit {
        return Err(AsrError::provider(format!("{} is larger than the manifest says", path.display())));
    }
    Ok(out)
}

/// Check the dimensions `config.json` asks for before they reach an allocation or a tensor shape.
/// The file is user-writable, so every count is bounded and the mel filter table is sized with
/// checked arithmetic.
fn validate_config(config: &Config) -> Result<(), AsrError> {
    let out_of_range = |what: &str, value: usize| AsrError::provider(format!("the model config's {what} ({value}) is out of range"));
    if !matches!(config.num_mel_bins, 80 | 128) {
        return Err(out_of_range("mel bins", config.num_mel_bins));
    }
    if config.num_mel_bins.checked_mul(mel::FFT_BINS).is_none() {
        return Err(out_of_range("mel bins", config.num_mel_bins));
    }
    if config.vocab_size == 0 || config.vocab_size > 60_000 {
        return Err(out_of_range("vocabulary size", config.vocab_size));
    }
    if config.max_target_positions == 0 || config.max_target_positions > 1_024 {
        return Err(out_of_range("context length", config.max_target_positions));
    }
    if config.max_source_positions == 0 || config.max_source_positions > 3_000 {
        return Err(out_of_range("audio context length", config.max_source_positions));
    }
    if config.d_model == 0 || config.d_model > 2_048 {
        return Err(out_of_range("model width", config.d_model));
    }
    for (what, heads) in [("encoder heads", config.encoder_attention_heads), ("decoder heads", config.decoder_attention_heads)] {
        if heads == 0 || heads > 64 || !config.d_model.is_multiple_of(heads) {
            return Err(out_of_range(what, heads));
        }
    }
    for (what, layers) in [("encoder layers", config.encoder_layers), ("decoder layers", config.decoder_layers)] {
        if layers == 0 || layers > 64 {
            return Err(out_of_range(what, layers));
        }
    }
    if config.suppress_tokens.iter().any(|token| *token as usize >= config.vocab_size) {
        return Err(AsrError::provider("the model config suppresses a token the vocabulary does not have"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_exactly_thirty_seconds_and_hostile_samples_are_clamped() {
        assert_eq!(window(&[0.5; 100]).len(), N_SAMPLES);
        let long = vec![0.25f32; N_SAMPLES + 10];
        let windowed = window(&long);
        assert_eq!(windowed.len(), N_SAMPLES);
        assert!(windowed.iter().all(|sample| (*sample - 0.25).abs() < 1e-9), "the last 30 seconds are kept");
        let hostile = [f32::NAN, f32::INFINITY, -4.0, 0.5];
        assert_eq!(&window(&hostile)[..4], &[0.0, 1.0, -1.0, 0.5]);
    }

    #[test]
    fn argmax_skips_non_finite_logits() {
        assert_eq!(argmax(&[f32::NAN, 0.1, 0.9, f32::NEG_INFINITY]), Some(2));
        assert_eq!(argmax(&[f32::NEG_INFINITY; 4]), None);
        assert_eq!(argmax(&[]), None);
    }

    fn tiny_config() -> Config {
        Config {
            num_mel_bins: 80,
            max_source_positions: 1_500,
            d_model: 384,
            encoder_attention_heads: 6,
            encoder_layers: 4,
            vocab_size: 51_865,
            max_target_positions: 448,
            decoder_attention_heads: 6,
            decoder_layers: 4,
            suppress_tokens: vec![],
        }
    }

    #[test]
    fn hostile_config_dimensions_are_refused_before_allocation() {
        assert!(validate_config(&tiny_config()).is_ok());
        type Change = (&'static str, fn(&mut Config));
        let cases: Vec<Change> = vec![
            ("mel bins", |config| config.num_mel_bins = usize::MAX),
            ("vocabulary size", |config| config.vocab_size = usize::MAX),
            ("context length", |config| config.max_target_positions = usize::MAX),
            ("audio context length", |config| config.max_source_positions = 0),
            ("model width", |config| config.d_model = 0),
            ("encoder heads", |config| config.encoder_attention_heads = 0),
            ("decoder heads", |config| config.decoder_attention_heads = 5),
            ("encoder layers", |config| config.encoder_layers = usize::MAX),
            ("decoder layers", |config| config.decoder_layers = 65),
        ];
        for (what, change) in cases {
            let mut config = tiny_config();
            change(&mut config);
            assert!(validate_config(&config).is_err(), "{what} was accepted");
        }
        let mut config = tiny_config();
        config.suppress_tokens = vec![u32::MAX];
        assert!(validate_config(&config).is_err(), "a suppressed token outside the vocabulary was accepted");
    }

    #[test]
    fn a_read_is_bounded_by_the_manifest_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("weights.bin");
        std::fs::write(&file, b"0123456789").expect("write");
        assert_eq!(bounded_read(&file, 10).map(|bytes| bytes.len()), Ok(10));
        assert!(bounded_read(&file, 9).is_err(), "a file past the limit is refused");
        assert!(bounded_read(&file, 0).is_err(), "a file with no manifest entry is refused");
        assert!(bounded_read(&dir.path().join("missing"), 10).is_err());
        assert!(bounded_read(dir.path(), 10).is_err(), "a directory is not a file");
    }

    #[test]
    fn a_recognizer_that_never_started_refuses_audio() {
        let mut recognizer = WhisperRecognizer::new("nowhere", None, Limits::default());
        let audio = AudioBuffer::new(vec![0.0; TARGET_RATE as usize], TARGET_RATE, 1);
        assert_eq!(recognizer.feed(&audio), Err(AsrError::NotRunning));
        assert!(recognizer.poll().is_empty());
        assert_eq!(recognizer.status().state, State::Idle);
    }
}
