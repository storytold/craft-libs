# craft-job

The vocabulary that printing and scanning crates would otherwise each define for themselves, plus
two plain records (`JobView`, `DeviceView`) so a UI can show a print job and a scan job, a printer
and a scanner, the same way. A leaf crate: `std` only, no dependencies by default (the optional `serde` feature adds `serde`), no I/O, no threads,
`unsafe` forbidden, builds for `wasm32-unknown-unknown`. Data types and small helpers only.

## In

| Item | What it is |
|---|---|
| `NativeCode { domain, code, name }`, `HRESULT_DOMAIN`, `NativeCode::hresult` | the platform or protocol error code beside the neutral classification |
| `BackendName`, `JobId`, `DeviceId`, `ParseIdError` | validated `<backend>:<native>` ids; kinds `MissingSeparator`, `EmptyBackend`, `InvalidBackend`, `EmptyNative`; `namespaced_id`, `split_id` |
| `RetrySafety`, `RetryPolicy`, `RetryFacts`, `RetryAdvice`, `advise`, `Jitter` | the retry decision, with jitter from an injected seed |
| `JobKind`, `JobState`, `Progress`, `ExpectedTotals` | what kind of job, its one overall state (`is_terminal`, `is_success`), progress (`fraction`, `or_expected`) |
| `DocumentRef`, `JobDocuments` | documents by reference: name, format, pages, bytes; never contents or paths |
| `Timestamp`, `JobTimes` | caller-supplied milliseconds since the Unix epoch; created, started, finished |
| `Tag`, `TagSupport`, `TagPlacement`, `Fingerprint` | the idempotency properties: tag text ` [ref abc123de]` (`suffix`, `parse_from_title`), where a backend can carry it, and what was submitted |
| `JobView`, `ErrorSummary` | the UI-facing job snapshot |
| `Tri`, `Severity`, `Indicator`, `IndicatorKey`, `DeviceState`, `DeviceConnection`, `CapabilitiesSummary`, `PaperPathView`, `PaperSource`, `Supply`, `DeviceView` | the UI-facing device snapshot |

`advise` rules: not retryable gives `DoNotRetry`; an `OutcomeUnknown` request gives
`CheckDeviceFirst` and is never retried automatically; spent attempts give `DoNotRetry`; otherwise
exponential backoff with seeded jitter, raised to the device's `retry_after` hint capped at
`max_retry_after`. `Safe` and `Idempotent` both retry automatically.

## Not in (deliberately)

Lifecycle objects and state machines (`PrintJob`, `ScanJob`), error kinds, attempt records and
trails, clocks, remote-job listing, filters, capabilities queries and the reconcile engine,
cancel tokens, and UI. Tags and fingerprints are *data* here; deciding which listed job is ours is
the domain crate's business. Most public enums and records are `#[non_exhaustive]`; build them with
`new` and the `with_*` builders. `NativeCode`, `Progress`, `ExpectedTotals` and `RetryPolicy` have
public fields and are not `#[non_exhaustive]`, so adding a field to them is a breaking change.

## `JobView` for a print job and a scan job

| Field | Print job | Scan job |
|---|---|---|
| `id` | `JobId` of the accepted job (`wsd:...`, `cups:...`) | `JobId` of the acquisition (`wia:...`, `escl:...`) |
| `kind` | `Print` | `Scan` |
| `title` | job name as submitted (with the tag suffix) | the scan's name |
| `state` | queued `Pending`, printing `Processing`, `Held`, `Completed`, `Cancelled`, `Aborted`; `Abandoned` when the printer never reported an outcome | `Processing` while pages arrive; `NeedsAttention` for empty feeder or jam with pages kept; `CompletedWithErrors` for a partial stack |
| `reasons` | printer state reasons as text | interrupt cause text |
| `progress` | sheets done and expected total | pages received |
| `times` | created, started, finished from the device or the caller | started and finished by the caller |
| `documents` | the document sent (name, `pdf`, pages, bytes) | the pages produced (name, `png`/`jpeg`, bytes) |
| `attempts` | submits made | acquisitions made |
| `last_error` | kind label, message, `RetryAdvice`, native code | the same |
| `tag` | the tag placed in the title | `None` (WIA and the inbox WSD driver have no job tag) |
| `device_name` | printer name | scanner name |

## `DeviceView`: status bits onto indicators

A clear bit from some drivers means "unknown", not "no", so every indicator is a `Tri`
(`On`, `Off`, `NotReported`); use `Tri::from_option`. The inbox WSD scan driver reports every bit
clear, so all of those map to `NotReported`, never `Off`. The lamp bit is an error flag only (there
is no on/off readback), so `LampError` is `On` or `NotReported`.

| craft-scan `HandlingStatus` | Indicator | Notes |
|---|---|---|
| `feeder_ready` | `FeederPaper` (readiness) and `PaperPathView::feeder_loaded` | `Option<bool>` becomes `Tri` |
| `flat_ready` | `FlatbedReady`, `PaperPathView::glass_available` | |
| `duplex_ready` | `DuplexReady` | |
| `cover_open` | `CoverOpen` | warning |
| `path_cover_open` | `PathCoverOpen` | warning |
| `jam` | `Jam` | error |
| `multiple_feed` | `MultiFeed` | error |
| `attention` | `Attention` | warning |
| `lamp_error` | `LampError` | error; no readback |

| craft-print printer state reason | Indicator |
|---|---|
| `media-jam` | `Jam` |
| `media-empty` / `media-needed` | `PaperOut` |
| `cover-open` / `door-open` | `CoverOpen` |
| `toner-low` / `marker-supply-low` | `SupplyLow` (and a `Supply`) |
| `offline` / `shutdown` | `Offline` |
| any other reason | `Other("<reason>")` |

`DeviceView::problems()` yields the warnings and errors; `worst_severity()` the highest.
`DeviceConnection` is named apart from craft-scan's `Connection`: `Usb` maps to `Usb`, `Wsd` to
`Network`, `Other` to `Other` and `Unknown` to `Unknown`. `Local` (a local port or virtual device)
has no scan counterpart today; a print spooler port uses it.

## Vocabulary: existing names

| craft-print | craft-scan | craft-job |
|---|---|---|
| `NativeCode` | `NativeCode`, `HRESULT_DOMAIN` | `NativeCode`, `HRESULT_DOMAIN` |
| `JobId`, `ParseJobIdError` | - | `JobId`, `ParseIdError` |
| namespaced printer id | `namespaced_id`, `split_id` (scanner id string) | `DeviceId`, `namespaced_id`, `split_id` |
| backend name check | `valid_name` in the registry | `BackendName` |
| `RetrySafety`, `RetryPolicy`, `Jitter`, `Progress`, `ExpectedTotals` | `retryable` on `ScanErrorInfo` | the same names |
| `RetryPolicy::next_delay` | - | `advise` -> `RetryAdvice` |

## Wire format (optional `serde` feature)

`craft-job = { version = "0.1", features = ["serde"] }` derives `Serialize` and `Deserialize` on the plain
data types, so an app can write a job journal or print a `JobView` without hand-written JSON. The feature
is off by default and changes nothing in a default build. The crate itself never reads or writes a file.

* **Names** are lowercase `snake_case` (`needs_attention`, `completed_with_errors`, `not_reported`,
  `job_name_suffix`). Struct fields keep their Rust names. This is the wire format and is meant to stay
  stable; new fields and variants are added, not renamed.
* **Ids and tags are strings**: `JobId` and `DeviceId` as `"<backend>:<native>"` (read back through
  `FromStr`, so a bad id is an error). `BackendName` is the bare name (`"wia"`, never namespaced; a name
  with a `:` or an empty one is an error). `Tag` is its 8 hex characters.
* **`Timestamp`** is milliseconds (`u64`). **Durations** (`RetryAdvice::RetryAfter`, `Fingerprint::window`)
  are whole milliseconds; finer precision is dropped. `RetryAdvice::RetryAfter(1.5 s)` is
  `{"retry_after":1500}`; `RetryNow` is `"retry_now"`.
* **`NativeCode`** is `{"domain","code","name"}`. Its `&'static str` fields cannot borrow from the input, so
  text read back is interned: the well-known domains (`hresult`, `win32`, `sane`, `http`, `ipp`,
  `wsd-fault`, `cups`, `nserror`) map to literals and anything else is leaked once into a table of at most
  256 strings of at most 64 bytes each. Beyond that a domain reads as `"other"` and a name as `null`.
  `IndicatorKey` is its snake_case name; an unknown name reads back as `IndicatorKey::Other(..)` under the
  same limits (so `Other("jam")` reads back as `Jam`).
* **Forward compatibility**: unknown fields are ignored; optional fields and collections may be missing
  (they default; `Progress::done` is required, so a record that lost it is an error and not "no progress yet"); a value this version does not know reads as `JobState::Unknown`,
  `DeviceState::Unknown`, `DeviceConnection::Unknown`, `Tri::NotReported` or `TagPlacement::None`.
  `JobKind`, `Severity`, `RetrySafety`, `RetryAdvice`, `TagSupport` and `PaperSource` have no such fallback and
  fail on an unknown value: a retry rule is not guessed.
* Round-trip tests live in `tests/serde_roundtrip.rs` (`serde_json` is a dev-dependency only).

## Migration

craft-print and craft-scan are expected to depend on this crate and `pub use` these names, so
their public paths do not change. craft-wsd has its own identical `RetrySafety`; it is not touched.
