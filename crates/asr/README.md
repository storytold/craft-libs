# craft-asr

Shared speech recognition for the Craft apps: a streaming recognizer protocol, bounded
microphone audio, the Whisper model manifest with verification, and a consent-gated download
loop. The icons, live-caption layout and slide-show timing stay with the app.

```rust
use craft_asr::{AudioBuffer, FakeRecognizer, Recognizer, TARGET_RATE};

let mut recognizer = FakeRecognizer::scripted(&["Hello there"]);
recognizer.start()?;
let audio = AudioBuffer::new(vec![0.1; TARGET_RATE as usize], TARGET_RATE, 1);
recognizer.feed(&audio)?;
let partial = recognizer.poll();          // "Hello there", final == false
recognizer.feed(&audio)?;
let final_line = recognizer.poll();       // "Hello there", final == true
recognizer.finish()?;
# Ok::<(), craft_asr::AsrError>(())
```

- `Recognizer` is `start` / `feed` / `finish` / `poll` / `status`. `Segment` carries `start`,
  `end`, `text` and the `final` flag (written `r#final` in Rust, because `final` is a reserved
  keyword); a partial line is still changing, a final line replaces it. Segments keep at most 500
  characters' newest text and drop control characters and bidi overrides.
- `AudioBuffer` is interleaved `f32`; `validate` rejects a zero or oversized sample rate, a zero
  or oversized channel count, a partial frame, more than 57,600,000 samples (230,400,000 bytes),
  or more than an hour of audio. `mono()` averages channels, `mono_16k()` resamples linearly to
  16 kHz. Non-finite samples become silence and every sample is clamped to `-1.0..=1.0`.
- `ModelManifest` is data: id, name, repository, revision, SPDX licence, source URL and one entry
  per file with its byte size, SHA-256 and HTTPS URL. `validate` bounds every field; file names
  are one safe path component and URLs must be `https://` on `huggingface.co` / `hf.co`.
  `verify_file` checks the size first and the digest second. `whisper_tiny()` is
  `openai/whisper-tiny` at a pinned revision.
- `Consent` is a three-state machine: `Pending` → `Accepted(manifest fingerprint)` or `Declined`.
  `permits` is true only for the exact manifest that was accepted — changing one digest, size,
  URL, revision or licence invalidates it. `download` refuses with `ConsentRequired` before it
  creates a directory or calls the transport.
- `download_with` writes each file to a `*.part` beside its final name, refuses a body past the
  manifest's size, verifies size and digest, then renames. A failure removes the temporary file
  and writes no provenance sidecar. `Transport` is injectable, so tests never touch the network;
  the `download` feature adds `HttpTransport` with connect, read and body timeouts.

## Features

- Default: the protocol, audio bounds, manifest verification, consent and the download loop
  against a caller-supplied `Transport`, plus the deterministic `FakeRecognizer`. No network and
  no model runtime.
- `candle`: Whisper inference on the CPU through [candle](https://github.com/huggingface/candle)
  (`WhisperEngine`, `WhisperRecognizer`). The model follows Radford et al., *Robust Speech
  Recognition via Large-Scale Weak Supervision*
  ([arXiv:2212.04356](https://arxiv.org/abs/2212.04356)); its weights are Apache-2.0 and are
  never shipped. Capture devices and worker threads stay with the app.
- `download`: the HTTPS transport for model files.

## Model evidence

The `whisper_tiny` manifest records three files from
`openai/whisper-tiny@169d4a4341b33bc18d8881c4b69c2e104e1cc0af`:

| file | bytes | SHA-256 |
| --- | ---: | --- |
| `model.safetensors` | 151,061,672 | `7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395` |
| `config.json` | 1,983 | `ffdccec4f3211f4c63310f2b7098f309fe70f3952cedc5e4d11e43f5b2379b98` |
| `tokenizer.json` | 2,480,466 | `27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566` |

The licence is Apache-2.0; the source page is <https://huggingface.co/openai/whisper-tiny>.

## Limits

No function panics on hostile input: damaged manifests, unsafe names, wrong sizes and digests,
non-finite audio, hostile sample rates and `config.json` dimensions all come back as errors.
Model files are at most 1 GiB each and 64 per model; a transcript keeps at most the configured
number of segments (100,000 by default).
