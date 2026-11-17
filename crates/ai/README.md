# craft-ai — AI providers for the Craft suite

Shared AI abstraction for every Craft app: translation, rewriting, summaries,
vision captions — one crate, one settings store, one model cache.

Status: **spec** — this file is the contract; implementation follows.

## The feature this powers

A per-app **AI tab** offering document-wide operations — the flagship being
"translate this whole document" — backed by either:

1. **BYOT** — the user's own API token against an OpenAI-compatible endpoint
   (OpenAI, Azure, Ollama, LM Studio, any compatible proxy), or
2. **Local model** — a one-click download of a curated open model, chosen for
   the machine it's running on.

The user experience is deliberately *informed*: every step reports what is
happening, why this model was picked, what it will cost in disk/RAM, and that
nothing leaves the machine in local mode.

## Modules

### `probe` — capability detection

Runs once, caches the result:

| Signal | Source |
| --- | --- |
| Total/free RAM | `sysinfo` or `std::alloc`-safe OS calls |
| CPU threads + features | `std::thread`, `is_x86_feature_detected!` (AVX2 gates Q4 speed) |
| GPU + VRAM | `wgpu` adapter info (apps already link wgpu — reuse, don't re-probe) |
| Free disk at the model cache dir | `fs2`/`std::fs` |
| Network reachability | optional HEAD to the model host, skipped offline |

Output: `MachineCaps { ram_gb, vram_gb, threads, avx2, disk_free_gb }`.

### `registry` — the curated model list

A small hand-maintained table (Rust consts, not runtime-fetched — supply
chain stays auditable). Each entry: `id`, display name, HF repo + file,
`sha256`, size, min RAM, recommended RAM, license id, quality tier,
languages covered.

Curation rule — **permissive licenses only** (the suite is MIT OR Apache-2.0
and commercial users run it):

- ✅ Qwen2.5-0.5B / 1.5B / 3B Instruct (Apache-2.0)
- ✅ SmolLM2-1.7B (Apache-2.0)
- ✅ Gemma-3 (Gemma Terms — allowed, flag in UI)
- ✅ Helsinki-NLP opus-mt-* per-language pairs (CC-BY/MIT)
- ❌ NLLB-200, M2M-100 — CC-BY-NC, excluded
- ❌ Llama-* — custom license acceptable? default: exclude until reviewed

`registry::suggest(&caps)` picks the best fitting model: prefers 3B when
VRAM ≥ 6 GB or RAM ≥ 16 GB, falls to 1.5B at 8 GB, 0.5B below. Always shows
alternatives — the suggestion is a default, not a cage.

### `provider` — inference backends

```rust
trait Provider {
    fn name(&self) -> &str;
    fn complete(&self, prompt: &str, opts: &GenOpts) -> Result<String, AiError>;
    fn test(&self) -> Result<ProviderInfo, AiError>;
}
```

- `OpenAiCompat` — POST `/v1/chat/completions`, configurable endpoint+token+
  model. Covers OpenAI, Ollama (`http://localhost:11434/v1`), LM Studio,
  Groq, OpenRouter… Token stored via the OS keyring (`keyring` crate) or a
  0600-permissioned config file — never in project files, never logged.
- `LocalGguf` — GGUF inference. First implementation target: **candle**
  (pure Rust, Apache/MIT — keeps the no-C++ toolchain rule). `llama-cpp-2`
  stays a possible perf upgrade later, gated behind a feature flag.

### `fetch` — one-click download

- Resumable HTTP GET (range requests), streamed to `*.part` then renamed —
  a half-downloaded GGUF can never be mistaken for a valid one.
- SHA-256 checked against the registry entry; mismatch = hard fail, delete.
- Progress events (`Download { bytes_done, bytes_total, bytes_per_sec }`)
  feed the UI's live status line.
- Cache dir is **shared across the suite**:
  `%LOCALAPPDATA%\artcraft\models` (Windows), `~/Library/Application
  Support/artcraft/models` (macOS), `~/.cache/artcraft/models` (Linux).

### `translate` — document segmentation

The model is the easy part; the hard part is *what not to touch*. The
translator walks the document as segments, not raw text:

- **Protected**: LaTeX commands/envs/math (`\cite{…}`, `$…$`), code fences,
  placeholders (`{count}`), URLs, citation keys, markup tags.
- **Translated**: prose runs, captions, headings, table cells.
- Per-app adapters declare the grammar: TexCraft protects `\command{…}` and
  math mode; WordCraft protects field codes; MatCraft protects code.
- Batch-friendly: segments → prompt with numbered items → parse numbered
  output; verify count matches, retry split halves on mismatch.
- Deterministic options: temperature 0-ish, glossary hook (`HashMap<term,
  translation>`) applied post-hoc.

## The AI tab (per app, `ui-egui/src/ai.rs`)

First-open flow — the "smooth informed" sequence:

1. Two cards: **Use an API provider** / **Run on this PC**.
2. BYOT card: endpoint, token, model fields + **Test connection** → green
   check with model list echo.
3. Local card: **Detect this PC** → probe runs →
   `Suggested: Qwen2.5-1.5B-Instruct (Q4_K_M, 1.1 GB) — fits your 16 GB RAM;
   est. ~8 tok/s` with an "Other models ▸" disclosure showing the registry.
4. **Download** → progress bar with MB/%/speed → `Verifying checksum…` →
   `Loading…` → `Ready — local, offline, nothing leaves this PC`.
5. Then the operations: "Translate document to [language ▾]", "Summarize
   selection", custom prompt box. Status line narrates each step;
   expandable activity log for the curious.
6. Language detected vs document language → gentle offer chip:
   `This document is French — translate to English?`

## Non-negotiables

- **Honesty**: the UI always says which provider is active, what model,
  and "cloud — your document text is sent to the endpoint" vs "local —
  nothing leaves this PC". No silent cloud calls; AI is strictly opt-in.
- **No crash**: model load failures, corrupt GGUFs, OOM — all `Result`,
  all surfaced as messages.
- **Rust only**: candle over llama.cpp bindings preserves the wasm story
  (local inference on wasm is future-possible with `candle` + WGPU backend).
- **Licensing**: registry carries SPDX ids; the UI shows them; CC-BY-NC and
  unreviewed custom licenses are excluded by policy.
- **Everything is a command**: `ai.translate_doc`, `ai.status`,
  `ai.models.list`, `ai.model.download`, `ai.provider.test` — reachable
  from the app's control channel and MCP, same as every Craft convention.

## Milestones

| # | Deliverable | Done when |
| --- | --- | --- |
| A1 | `probe` + `registry` + `fetch` | CLI-free test: probe reports real caps; `suggest` picks sanely per fixture; a tiny model downloads + sha256 verifies |
| A2 | `OpenAiCompat` provider | `translate` a string through Ollama/OpenAI with mocked HTTP tests |
| A3 | `LocalGguf` on candle | Qwen2.5-0.5B loads and answers on CPU in a test (model fixture fetched in-test, skipped without network) |
| A4 | `translate` segmentation | LaTeX/code/placeholder round-trip tests — protected spans byte-identical |
| A5 | AI tab in one pilot app (suggest TexCraft or WordCraft) | download → translate document end-to-end, live status UX |
| A6 | Commands + MCP surface | `ai.*` commands reachable via control channel |
