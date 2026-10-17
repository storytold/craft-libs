//! Inference backends behind one trait. Apps hold a `Box<dyn Provider>`
//! and never care whether the bytes went to OpenAI, Ollama or a local GGUF.

use crate::error::AiError;

/// Generation parameters — kept minimal on purpose; the UI exposes
/// "translate", not a sampler playground.
#[derive(Clone, Debug)]
pub struct GenOpts {
    /// Max tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature; translation wants ~0.
    pub temperature: f32,
}

impl Default for GenOpts {
    fn default() -> Self {
        Self { max_tokens: 2048, temperature: 0.1 }
    }
}

/// What `test()` learned about a provider — shown next to the green check.
#[derive(Clone, Debug)]
pub struct ProviderInfo {
    /// Human label, e.g. "Ollama at http://localhost:11434".
    pub label: String,
    /// Model ids the endpoint advertised (may be empty if it doesn't say).
    pub models: Vec<String>,
}

/// An AI backend the suite can use. `Send` so apps can move a loaded
/// provider onto a worker thread for long generations.
pub trait Provider: Send {
    /// Short name for the status line ("local: Qwen2.5 1.5B", "OpenAI-compatible").
    fn name(&self) -> String;
    /// One completion. Prompt construction is the caller's job — this
    /// trait is deliberately text-in/text-out so every consumer (translate,
    /// summarize, custom prompt) shares one path.
    fn complete(&mut self, prompt: &str, opts: &GenOpts) -> Result<String, AiError>;
    /// Can we reach it? Never sends document text — just a handshake.
    fn test(&mut self) -> Result<ProviderInfo, AiError>;
    /// True when document text stays on this machine.
    fn is_local(&self) -> bool;
}

/// Any OpenAI-compatible `/v1/chat/completions` endpoint: OpenAI, Azure,
/// Ollama (`http://localhost:11434/v1`), LM Studio, Groq, OpenRouter…
/// The token is a plain string — storage (OS keyring, settings file) is
/// the app's choice, and it is never logged or echoed into errors.
#[cfg(not(target_arch = "wasm32"))]
pub struct OpenAiCompat {
    /// Base URL ending at the API root, e.g. `https://api.openai.com/v1`.
    pub endpoint: String,
    /// Bearer token; `None` for unauthenticated local endpoints (Ollama).
    pub token: Option<String>,
    /// Model id sent in every request.
    pub model: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl OpenAiCompat {
    pub fn new(endpoint: impl Into<String>, token: Option<String>, model: impl Into<String>) -> Self {
        Self { endpoint: endpoint.into(), token, model: model.into() }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.endpoint.trim_end_matches('/'), path)
    }

    fn auth(&self, req: ureq::RequestBuilder<ureq::typestate::WithBody>) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        match &self.token {
            Some(t) if !t.is_empty() => req.header("Authorization", format!("Bearer {t}")),
            _ => req,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Provider for OpenAiCompat {
    fn name(&self) -> String {
        format!("{} ({})", self.model, self.endpoint)
    }

    fn complete(&mut self, prompt: &str, opts: &GenOpts) -> Result<String, AiError> {
        if self.endpoint.trim().is_empty() {
            return Err(AiError::NotConfigured);
        }
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": opts.temperature,
            "max_tokens": opts.max_tokens,
        });
        let mut resp =
            self.auth(ureq::post(self.url("chat/completions"))).header("Content-Type", "application/json").send_json(body).map_err(|e| match e {
                ureq::Error::StatusCode(401 | 403) => AiError::Rejected("authentication failed — check the API token".into()),
                ureq::Error::StatusCode(404) => AiError::Rejected("model or endpoint not found — check both fields".into()),
                ureq::Error::StatusCode(c) => AiError::Rejected(format!("HTTP {c}")),
                other => AiError::Http(other.to_string()),
            })?;
        let v: serde_json::Value = resp.body_mut().read_json().map_err(|e| AiError::BadResponse(e.to_string()))?;
        v.pointer("/choices/0/message/content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AiError::BadResponse("no choices[0].message.content in response".into()))
    }

    fn test(&mut self) -> Result<ProviderInfo, AiError> {
        if self.endpoint.trim().is_empty() {
            return Err(AiError::NotConfigured);
        }
        // GET /v1/models — a handshake only, no user text leaves the app.
        let mut req = ureq::get(self.url("models"));
        if let Some(t) = &self.token
            && !t.is_empty()
        {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let mut resp = req.call().map_err(|e| match e {
            ureq::Error::StatusCode(401 | 403) => AiError::Rejected("authentication failed — check the API token".into()),
            ureq::Error::StatusCode(c) => AiError::Rejected(format!("HTTP {c}")),
            other => AiError::Http(other.to_string()),
        })?;
        let v: serde_json::Value = resp.body_mut().read_json().map_err(|e| AiError::BadResponse(e.to_string()))?;
        let models = v
            .get("data")
            .and_then(serde_json::Value::as_array)
            .map(|arr| arr.iter().filter_map(|m| m.get("id").and_then(serde_json::Value::as_str).map(str::to_string)).collect())
            .unwrap_or_default();
        Ok(ProviderInfo { label: format!("reachable: {}", self.endpoint), models })
    }

    fn is_local(&self) -> bool {
        // Local endpoints keep text on the machine; we can't verify a
        // remote URL's forwarding, so honesty means: loopback → local.
        let host = self.endpoint.trim_start_matches("http://").trim_start_matches("https://");
        host.starts_with("localhost") || host.starts_with("127.0.0.1") || host.starts_with("[::1]")
    }
}
