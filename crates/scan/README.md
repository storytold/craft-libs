# craft-scan

The portable core of scanning for the Craft apps: scan settings and the enums behind them, the
neutral error contract (`ScanError`, `ScanErrorInfo`, `NativeCode`, recovery hints), paper-path
status that may be unknown, the auto-fallback-to-glass decision, cancel and
progress tokens, the BMP parser and software 1-bit conversion, an optional JSONL acquire trace, and
a registry of scanner backends, and `ScanJob`, the record of one scan across its attempts. Extracted from PdfCraft's `crates/scan`. Requires Rust 1.95.

There is **no platform code, no unsafe, no network and no UI** here, and nothing in this crate
opens a scanner. The code that talks to a device (Windows WIA, SANE, eSCL, ImageCaptureCore, ...)
stays in the app, which plugs it in through the `Backend` trait. The crate builds for
`wasm32-unknown-unknown`.

## Registering backends

```rust
use craft_scan::{Acquired, Backend, ScanError, ScanSettings, Scanner, ScannerRef};

struct MyBackend;

impl Backend for MyBackend {
    fn name(&self) -> &'static str {
        "mine"
    }
    fn scanner_list(&self) -> Result<Vec<ScannerRef>, ScanError> {
        Ok(vec![ScannerRef::new("scanner-1".into(), "Desk scanner".into(), None)])
    }
    fn scanner_details(&self, _native_id: &str) -> Result<Scanner, ScanError> {
        Err(ScanError::Failed("not implemented in this example".into()))
    }
    fn scanners(&self) -> Result<Vec<Scanner>, ScanError> {
        Ok(Vec::new())
    }
    fn diagnostic_dump(&self, _filter: &str) -> Result<String, ScanError> {
        Ok("nothing to report\n".into())
    }
    fn acquire(&self, _settings: &ScanSettings) -> Result<Acquired, ScanError> {
        Err(ScanError::Failed("not implemented in this example".into()))
    }
}

// Once per backend, early in the app. The same name twice is a `RegisterError`, not a panic.
craft_scan::register(Box::new(MyBackend))?;

// Everywhere else call the crate-level functions; they aggregate and route across backends.
let listing = craft_scan::scanner_list()?;
assert_eq!(listing.items[0].id, "mine:scanner-1"); // "<backend>:<native id>"
assert!(listing.is_complete()); // a backend that failed would be in `listing.errors`
# Ok::<(), Box<dyn std::error::Error>>(())
```

* **Several backends at once.** Windows could register WIA and eSCL together. Device ids are
  namespaced `"<backend>:<native id>"`; `scanner_details` and `acquire` route by that prefix (an
  unknown prefix is an error that names the registered backends). `ScanSettings::device` may also
  be empty or a name fragment, see `Registry::acquire`.
* **Partial failure.** `scanner_list` and `scanners` return a `Listing`: the devices found, plus one
  `BackendError` per backend that failed. One failing backend does not hide another's devices.
* **Nothing registered** behaves as before: every function returns `ScanError::Unsupported` and
  `supported()` is `false`.
* **Isolation.** `Registry` is also usable standalone (`Registry::new()`); the free functions are a
  thin wrapper around one process-wide `Registry`.

The free functions keep the names `scanners`, `scanner_list`, `scanner_details` and `acquire`;
`dump_devices` is now `diagnostic_dump(filter)`, which returns the backend's own text.

## Writing a backend

A backend implements `Backend` (blocking calls, made from app worker threads; progress and
cancellation arrive in `ScanSettings::progress` and `::cancel`):

| Method | Provide |
|---|---|
| `name()` | Short lowercase name; the id prefix (`"wia"`, `"sane"`, `"escl"`). |
| `scanner_list()` | The devices the system lists, native ids, without opening them. |
| `scanner_details(id)` / `scanners()` | Capabilities. Lists per source (`glass`, `feeder_choices`), each optional: unpublished is `None`/empty. |
| `status(id)` | Optional. `None` = cannot preflight. Fill only what you can read; `feeder_ready` is `Some(true)`, `Some(false)` or `None` (unknown). |
| `diagnostic_dump(filter)` | Backend-specific text for support: WIA properties, SANE option descriptors, an eSCL capabilities document. |
| `acquire(settings)` | The pages, as encoded images in memory. |

Rules the app relies on:

* **Pages are encoded images.** `Page::format` and `Page::bytes` are what the device delivered: BMP
  from WIA, JPEG/PNG/TIFF/PDF from eSCL, SANE or ImageCaptureCore. Nothing outside a Windows
  backend may assume BMP. `bw::page_for` enforces black and white on BMP, JPEG, PNG and TIFF.
* **Settings are requests.** Image format, compression, intent and page size are hints a backend may
  ignore; the `ImageChoices` lists tell the UI what is honoured.
* **Errors are neutral plus native.** Build them with `ScanError::native_error(kind, NativeCode {
  domain, code, name }, message)`. `domain` is yours (`"hresult"`, `"sane"`, `"http"`, `"nserror"`);
  the kind, phase and recovery flags stay neutral, and `ScanErrorInfo::hresult()`/`code_name()`
  answer only for the `"hresult"` domain. A backend keeps its own code-to-name and code-to-kind
  tables; `interrupt_kind(message)` sentence matching is only a documented fallback.
* **An empty feeder is `InterruptKind::FeederEmpty`** whenever the backend can tell (SANE `NO_DOCS`,
  eSCL `AdfEmpty`). `BusyOrFeederEmpty` is a Windows WSD driver quirk that other backends never
  produce. `auto_fallback_to_glass` and the page-count rules work from `FeederEmpty` alone.
* **Keep partial pages**: when a later sheet fails, return `ScanError::Incomplete` with the pages
  that arrived and the cause.

Platform ideas, non-binding and **not provided here**: WIA (Windows, `unsafe` COM in the app), SANE
(Linux, `sane_*` calls or `scanimage`), eSCL/AirScan (HTTP to a network scanner), ImageCaptureCore
(macOS). The tests in `tests/common` carry a WIA-, a SANE- and an eSCL-shaped fake to show the
contract holds for all three; they are not shipped API.

## Tracking one scan: `ScanJob`

`craft_scan::job::ScanJob` is the record of **one user-visible acquisition** across the attempts it takes: a
large feeder scan that jams at sheet 12, a feeder that runs dry and is reloaded, a scanner that answers busy.
It does not run `acquire` (the app does, on its worker thread); it plans each attempt (`begin_attempt`,
`begin_attempt_with(Continuation::Append | Restart)`), records the pages streamed through the `ProgressToken`
and the result (`record_progress`, `finish_attempt`), and answers `retry_advice()` (a `craft_job::RetryAdvice`;
automatic retry only for safe, transient failures with nothing kept, `CheckDeviceFirst` for an unknown outcome).
Pages received survive a failed attempt; `snapshot()` is cheap (no page bytes), `view()` is a `craft_job::JobView`,
`take_pages()` hands the pages over. `device_view(&Scanner, status, source)` maps the status bits onto a
`craft_job::DeviceView`, a clear bit from a driver that reports nothing being `NotReported`, not `Off`.

No thread, clock, async or I/O. There is **no idempotency tag for a scan** (`Backend::tag_support()` is
`TagSupport::None` everywhere) and a duplicate `acquire` moves the carriage and pulls paper. The states,
continuation rules, what survives which failure and the limits are in [docs/scan-job.md](docs/scan-job.md).

The optional `serde` feature (off by default) adds `Serialize`/`Deserialize` on the snapshot, a `ScanJobJournal` that a
command-line run writes beside its pages, and the contract enums with their existing wire strings;
`ScanJob::continuing_from(..)` starts a later run from the journal. Page bytes and tokens are never serialised.

## Page length

A feeder often cannot tell how long the sheet it pulled is. `ScanSettings::page_length` says how the
length is decided; the backend applies it and the page records the answer in `Page::length`
(`LengthRecord`: `source`, `length_mils`, `approximate`). In priority order:

1. **Device detection** when the scanner publishes it (`Scanner::length_support(source).device_detect`,
   three-state `Presence`: `Present` only when the property or value is published). `LengthSource::Device`.
2. **An explicit `page_size`** becomes extents, which works on any scanner that takes an extent (inside the
   item's published range, never an unlisted page-size value). `LengthSource::Size`.
3. **Inference from the image**, opt in with `PageLength::AutoInfer`: request the item's maximum extent
   and cut at the end of the sheet found by `infer_sheet_end`. `LengthSource::Inferred`, always
   `approximate`.

`PageLength::Driver` is the default and changes nothing. `Size` is step 2 alone (the chosen size's length even when the
device could detect one). `Auto` is steps 1 and 2 (deterministic, never
looks at pixels); `AutoInfer` adds step 3 only when neither applies.

`infer_sheet_end(bytes, dpi_y, &InferOptions)` is pure. It treats the last row holding a pixel that is not
fill (exactly `255`; a small tolerance for JPEG) as the sheet end, adds a small margin, and refuses
unless the rows above the end show scanner paper noise (so a noiseless white margin is never mistaken for
fill). It never touches the width and does nothing when content runs to the last row. See the module
documentation of `length` for the rule and its limits. The rule was measured on one scanner that pads
everything beyond the sheet with pure white; treat it as a heuristic.

```rust
use craft_scan::{InferOptions, infer_sheet_end};
# let bytes: Vec<u8> = Vec::new();
// A BMP, PNG, TIFF or JPEG as delivered; None when nothing clear can be inferred.
let _ = infer_sheet_end(&bytes, 200, &InferOptions::for_bytes(&bytes));
```

A backend may override `Backend::page_length_support` (unknown by default) and fill
`Reported::glass_length` / `feeder_length` in `scanner_details`.

## Environment variables

| Variable | Meaning |
|---|---|
| `CRAFT_SCAN_TRACE` | `1` traces acquire phases to stderr, or a file path to append JSON lines to. Phases only: no ids, names, ports or image bytes. |

The older `PDFCRAFT_SCAN_TRACE` is still read when the new name is not set.

## Tests

`tests/bmp_fixtures.rs` checks the BMP readers against 118 synthetic files in
`tests/fixtures/bmp` (see [ATTRIBUTION.md](../../ATTRIBUTION.md)). `tests/registry_contract.rs`
drives standalone registries with the three fake backends; the process-wide registry cannot be
reset inside a process, so its scenarios (`registry_global_none`, `registry_global_two`) are their
own files. `tests/wia_reference.rs` holds the WIA tables and rules that are the Windows backend's,
not the core's.
