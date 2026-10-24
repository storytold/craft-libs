//! The registry contract, through the public API, with three differently shaped fake backends
//! (WIA-like, SANE-like, eSCL-like). Each test makes its own `Registry`, so they do not share
//! state; the process-wide registry has its own test files.

mod common;

use common::escl_like::{AdfState, ESCL_ID, EsclLike, SERVICE_UNAVAILABLE};
use common::request;
use common::sane_like::{NO_DOCS, SANE_ID, SaneLike};
use common::wia_like::{WIA_ID, WiaLike};
use craft_scan::bw::{PageRule, page_for};
use craft_scan::{
    Acquired, Backend, Color, ErrorContext, InterruptKind, Phase, ProgressToken, RegisterError, Registry, ScanError, ScanSettings, ScannerRef,
    Source, auto_fallback_to_glass, bmp, image_format, preflight,
};
use std::sync::{Arc, Mutex};

/// The one native id [`Recorder`] accepts.
const RECORDER_ID: &str = "usb:001:7";

/// A backend that records the device id `Registry::acquire` hands it and accepts only [`RECORDER_ID`], so a test can
/// see what the registry stripped, kept or sent.
struct Recorder {
    /// Every `settings.device` this backend's `acquire` was called with, in order.
    seen: Arc<Mutex<Vec<String>>>,
}

impl Recorder {
    /// A recorder and the shared list of the ids it receives.
    fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (Self { seen: Arc::clone(&seen) }, seen)
    }
}

impl Backend for Recorder {
    fn name(&self) -> &'static str {
        "rec"
    }

    fn scanner_list(&self) -> Result<Vec<ScannerRef>, ScanError> {
        Ok(vec![ScannerRef::new(RECORDER_ID.into(), "Recorder".into(), None)])
    }

    fn scanner_details(&self, native_id: &str) -> Result<craft_scan::Scanner, ScanError> {
        Err(ScanError::Failed(format!("rec: no details for {native_id}")))
    }

    fn scanners(&self) -> Result<Vec<craft_scan::Scanner>, ScanError> {
        Ok(Vec::new())
    }

    fn diagnostic_dump(&self, _filter: &str) -> Result<String, ScanError> {
        Ok(String::new())
    }

    fn acquire(&self, settings: &ScanSettings) -> Result<Acquired, ScanError> {
        self.seen.lock().expect("seen").push(settings.device.clone());
        if settings.device == RECORDER_ID || settings.device.is_empty() {
            Ok(Acquired { source: Source::Glass, pages: Vec::new() })
        } else {
            Err(ScanError::Failed(format!("rec: unknown device {:?}", settings.device)))
        }
    }
}

/// A registry with the Windows-, SANE- and eSCL-like backends registered.
fn all_three() -> Registry {
    let registry = Registry::new();
    registry.register(Box::new(WiaLike::new())).expect("wia");
    registry.register(Box::new(SaneLike::new(1))).expect("sane");
    registry.register(Box::new(EsclLike::new(AdfState::Loaded, 2))).expect("escl");
    registry
}

/// The ids of `items`, as owned strings.
fn ids<T>(items: &[T], id: impl Fn(&T) -> &str) -> Vec<String> {
    items.iter().map(|item| id(item).to_string()).collect()
}

#[test]
fn an_empty_registry_is_unsupported_for_everything() {
    let registry = Registry::new();
    assert!(!registry.supported());
    assert!(registry.backend_names().is_empty());
    assert_eq!(registry.scanner_list().err(), Some(ScanError::Unsupported));
    assert_eq!(registry.scanners().err(), Some(ScanError::Unsupported));
    assert_eq!(registry.scanner_details("sane:x").err(), Some(ScanError::Unsupported));
    assert_eq!(registry.status("sane:x").err(), Some(ScanError::Unsupported));
    assert_eq!(registry.diagnostic_dump("").err(), Some(ScanError::Unsupported));
    assert_eq!(registry.acquire(&ScanSettings::default()).err(), Some(ScanError::Unsupported));
}

#[test]
fn backends_register_side_by_side_and_listings_aggregate_with_namespaced_ids() {
    let registry = all_three();
    assert!(registry.supported());
    assert_eq!(registry.backend_names(), ["wia", "sane", "escl"]);
    let list = registry.scanner_list().expect("list");
    assert!(list.is_complete());
    assert_eq!(ids(&list.items, |s| &s.id), [format!("wia:{WIA_ID}"), format!("sane:{SANE_ID}"), format!("escl:{ESCL_ID}")]);
    let scanners = registry.scanners().expect("scanners");
    assert_eq!(ids(&scanners.items, |s| &s.id), ids(&list.items, |s| &s.id));
    // An id may contain colons; only the first one is the prefix.
    let details = registry.scanner_details(&format!("sane:{SANE_ID}")).expect("details");
    assert_eq!(details.id, format!("sane:{SANE_ID}"));
    assert_eq!(registry.scanner_details(&format!("escl:{ESCL_ID}")).expect("escl").id, format!("escl:{ESCL_ID}"));
    assert_eq!(registry.scanner_details(&format!("wia:{WIA_ID}")).expect("wia").id, format!("wia:{WIA_ID}"));
}

#[test]
fn an_unknown_or_missing_prefix_is_a_clear_error() {
    let registry = all_three();
    let unknown = registry.scanner_details("ica:whatever").expect_err("unknown prefix").to_string();
    assert!(unknown.contains("no scanner backend named \"ica\"") && unknown.contains("wia, sane, escl"), "{unknown}");
    let bare = registry.scanner_details("pixma:libusb").expect_err("bare").to_string();
    assert!(bare.contains("no scanner backend named \"pixma\""), "{bare}");
    let none = registry.scanner_details("no-colon-here").expect_err("no prefix").to_string();
    assert!(none.contains("no backend prefix") && none.contains("<backend>:<id>"), "{none}");
    // A backend's own error for a native id it does not know comes through.
    assert!(registry.scanner_details("sane:missing").is_err());
}

#[test]
fn a_failing_backend_does_not_hide_the_others_devices() {
    let registry = Registry::new();
    registry.register(Box::new(WiaLike::new())).expect("wia");
    registry.register(Box::new(SaneLike { loaded: 1, fail_listing: true })).expect("sane");
    registry.register(Box::new(EsclLike::new(AdfState::Loaded, 1))).expect("escl");
    for listing in
        [registry.scanner_list().map(|l| (ids(&l.items, |s| &s.id), l.errors)), registry.scanners().map(|l| (ids(&l.items, |s| &s.id), l.errors))]
    {
        let (found, errors) = listing.expect("listing");
        assert_eq!(found, [format!("wia:{WIA_ID}"), format!("escl:{ESCL_ID}")]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].backend, "sane");
        assert_eq!(errors[0].error.interrupt_kind(), InterruptKind::DeviceError);
        assert_eq!(errors[0].error.native().map(|n| n.domain), Some("sane"));
    }
    // All backends failing is still a listing (empty, with every error), not `Unsupported`.
    let all_fail = Registry::new();
    all_fail.register(Box::new(SaneLike { loaded: 0, fail_listing: true })).expect("sane");
    let listing = all_fail.scanners().expect("listing");
    assert!(listing.items.is_empty() && listing.errors.len() == 1 && !listing.is_complete());
}

#[test]
fn registering_a_name_twice_is_an_error_and_the_first_stays() {
    let registry = Registry::new();
    registry.register(Box::new(SaneLike::new(1))).expect("first");
    let error = registry.register(Box::new(SaneLike::new(9))).expect_err("duplicate");
    assert_eq!(error, RegisterError::DuplicateName { name: "sane" });
    assert_eq!(error.to_string(), "a scanner backend named \"sane\" is already registered");
    assert_eq!(registry.backend_names(), ["sane"]);
    // The first backend is the one still answering (one sheet loaded, not nine).
    let acquired = registry.acquire(&request("", Source::Feeder, 0)).expect("acquire");
    assert_eq!(acquired.pages.len(), 1);
}

#[test]
fn acquire_routes_by_prefix_and_strips_it() {
    let registry = all_three();
    let sane = registry.acquire(&request(&format!("sane:{SANE_ID}"), Source::Glass, 1)).expect("sane");
    assert_eq!((sane.pages[0].format, sane.source), ("jpeg", Source::Glass));
    let escl = registry.acquire(&request(&format!("escl:{ESCL_ID}"), Source::Feeder, 2)).expect("escl");
    assert_eq!((escl.pages.len(), escl.pages[0].format), (2, "jpeg"));
    let wia = registry.acquire(&request(&format!("wia:{WIA_ID}"), Source::Glass, 1)).expect("wia");
    assert_eq!(wia.pages[0].format, "bmp");
}

#[test]
fn acquire_hands_the_backend_its_native_id() {
    let (recorder, seen) = Recorder::new();
    let registry = all_three();
    registry.register(Box::new(recorder)).expect("rec");
    let sent = || std::mem::take(&mut *seen.lock().expect("seen"));

    // A registered prefix is removed, and only the first colon is the prefix: the native id keeps its own.
    registry.acquire(&request(&format!("rec:{RECORDER_ID}"), Source::Glass, 1)).expect("prefixed");
    assert_eq!(sent(), [RECORDER_ID]);
    // A wrong native id under the right prefix reaches the backend and is its error, not the registry's.
    let wrong = registry.acquire(&request("rec:nope", Source::Glass, 1)).expect_err("wrong native id").to_string();
    assert!(wrong.contains("rec: unknown device \"nope\""), "{wrong}");
    assert_eq!(sent(), ["nope"]);
    // The prefix is the backend's name exactly: another backend's prefix never reaches it.
    registry.acquire(&request(&format!("sane:{SANE_ID}"), Source::Glass, 1)).expect("sane");
    assert!(sent().is_empty(), "the other backend's request never reached the recorder");
    // A name fragment with several backends is the registry's error, and no backend is called.
    assert!(registry.acquire(&request("Desk", Source::Glass, 1)).is_err());
    assert!(sent().is_empty());

    // With one backend: a device with no registered prefix goes through whole (its own colons are not a prefix), an
    // empty device stays empty, and the backend's own prefix is still removed.
    let (only, seen_only) = Recorder::new();
    let single = Registry::new();
    single.register(Box::new(only)).expect("rec");
    single.acquire(&request(RECORDER_ID, Source::Glass, 1)).expect("passed on whole");
    single.acquire(&request("", Source::Glass, 1)).expect("empty");
    single.acquire(&request(&format!("rec:{RECORDER_ID}"), Source::Glass, 1)).expect("prefixed");
    assert_eq!(*seen_only.lock().expect("seen"), [RECORDER_ID.to_string(), String::new(), RECORDER_ID.to_string()]);
}

#[test]
fn acquire_with_no_prefix_goes_to_the_only_backend_or_the_first_that_lists() {
    // Several backends and an empty device: the first one that lists a scanner.
    let registry = all_three();
    let acquired = registry.acquire(&request("", Source::Glass, 1)).expect("default");
    assert_eq!(acquired.pages[0].format, "bmp", "wia is first and lists a scanner");
    let skipping = Registry::new();
    skipping.register(Box::new(SaneLike { loaded: 1, fail_listing: true })).expect("sane");
    skipping.register(Box::new(EsclLike::new(AdfState::Loaded, 1))).expect("escl");
    assert_eq!(skipping.acquire(&request("", Source::Glass, 1)).expect("escl").pages[0].format, "jpeg");
    // A name fragment is ambiguous across backends.
    let ambiguous = registry.acquire(&request("Desk", Source::Glass, 1)).expect_err("ambiguous").to_string();
    assert!(ambiguous.contains("does not name a registered backend") && ambiguous.contains("wia, sane, escl"), "{ambiguous}");
    let unknown = registry.acquire(&request("ica:x", Source::Glass, 1)).expect_err("unknown prefix").to_string();
    assert!(unknown.contains("ica:x"), "{unknown}");
    // With one backend a fragment (or a native id that happens to contain colons) is passed through.
    let single = Registry::new();
    single.register(Box::new(EsclLike::new(AdfState::Loaded, 1))).expect("escl");
    assert!(single.acquire(&request("Desk", Source::Glass, 1)).is_ok());
    assert!(single.acquire(&request("urn:uuid:1234", Source::Glass, 1)).is_ok());
}

#[test]
fn the_progress_token_receives_pages_as_they_arrive() {
    let registry = all_three();
    let progress = ProgressToken::new();
    let settings = ScanSettings { progress: Some(progress.clone()), ..request(&format!("escl:{ESCL_ID}"), Source::Feeder, 2) };
    let acquired = registry.acquire(&settings).expect("acquire");
    assert_eq!(acquired.pages.len(), 2);
    assert_eq!(progress.pages_done(), 2);
    assert_eq!(progress.take_pages().len(), 2);
}

#[test]
fn partial_pages_are_kept_when_a_later_page_fails_on_each_backend() {
    let registry = Registry::new();
    // SANE: one sheet in the tray, three wanted; NO_DOCS on the second frame.
    registry.register(Box::new(SaneLike::new(1))).expect("sane");
    // eSCL: the ADF jams after the first document.
    registry.register(Box::new(EsclLike { jam_after: Some(1), ..EsclLike::new(AdfState::Loaded, 3) })).expect("escl");
    // WIA: one sheet in the tray; the driver answers busy for the next one.
    registry.register(Box::new(WiaLike::new())).expect("wia");

    let settings = |device: String| ScanSettings { progress: Some(ProgressToken::new()), ..request(&device, Source::Feeder, 3) };

    let sane = settings(format!("sane:{SANE_ID}"));
    let error = registry.acquire(&sane).expect_err("sane shortfall");
    assert_eq!(craft_scan::pages_in_error(&error), 1);
    let info = error.info(ErrorContext::from_settings(&sane));
    assert_eq!(info.kind, InterruptKind::FeederEmpty, "an empty feeder SANE can tell is FeederEmpty, never the WSD quirk");
    assert_eq!(info.native, Some(NO_DOCS));
    assert_eq!((info.hresult(), info.code_name()), (None, None));
    assert_eq!(info.phase, Phase::BetweenPages);
    assert!(info.resumable && !info.retryable);

    let escl = settings(format!("escl:{ESCL_ID}"));
    let error = registry.acquire(&escl).expect_err("escl jam");
    assert_eq!(craft_scan::pages_in_error(&error), 1);
    let info = error.info(ErrorContext::from_settings(&escl));
    assert_eq!((info.kind, info.native), (InterruptKind::Jam, Some(SERVICE_UNAVAILABLE)));
    assert!(info.resumable);

    let wia = settings(format!("wia:{WIA_ID}"));
    let error = registry.acquire(&wia).expect_err("wia shortfall");
    assert_eq!(craft_scan::pages_in_error(&error), 1);
    assert_eq!(error.info(ErrorContext::from_settings(&wia)).kind, InterruptKind::FeederEmpty);
}

#[test]
fn a_backend_that_cannot_preflight_learns_an_empty_feeder_from_the_failure() {
    let registry = Registry::new();
    registry.register(Box::new(SaneLike::new(0))).expect("sane");
    // No status: nothing blocks, and `Auto` tries the feeder first.
    assert_eq!(registry.status(&format!("sane:{SANE_ID}")).expect("status"), None);
    assert!(preflight(Source::Feeder, None).is_ok());
    let scanner = registry.scanner_details(&format!("sane:{SANE_ID}")).expect("details");
    assert_eq!(scanner.feeder_ready, None);
    assert_eq!(Source::Auto.resolve(scanner.feeder_ready, scanner.feeder, scanner.flatbed), Ok(Source::Feeder));
    // The feeder is empty: the scan fails with SANE's NO_DOCS, which is FeederEmpty and falls back to the glass.
    let error = registry.acquire(&request("", Source::Auto, 1)).expect_err("empty");
    assert_eq!(error.interrupt_kind(), InterruptKind::FeederEmpty);
    assert_eq!(error.native(), Some(NO_DOCS));
    assert!(auto_fallback_to_glass(Source::Auto, Source::Feeder, scanner.flatbed, 0, &error));
    let glass = registry.acquire(&request("", Source::Glass, 1)).expect("glass");
    assert_eq!(glass.pages.len(), 1);
}

#[test]
fn a_backend_with_a_status_read_blocks_before_scanning() {
    for (adf, kind) in [
        (AdfState::Empty, InterruptKind::FeederEmpty),
        (AdfState::Jam, InterruptKind::Jam),
        (AdfState::DoorOpen, InterruptKind::CoverOpen),
        (AdfState::Multipick, InterruptKind::MultiFeed),
    ] {
        let registry = Registry::new();
        registry.register(Box::new(EsclLike::new(adf, 2))).expect("escl");
        let status = registry.status(&format!("escl:{ESCL_ID}")).expect("status");
        assert!(status.is_some(), "{adf:?}");
        assert_eq!(preflight(Source::Feeder, status).err().map(|e| e.interrupt_kind()), Some(kind), "{adf:?}");
        let error = registry.acquire(&request("", Source::Feeder, 1)).expect_err("blocked");
        assert_eq!((error.interrupt_kind(), craft_scan::pages_in_error(&error)), (kind, 0), "{adf:?}");
    }
    // An empty ADF does not stop the glass, and `Auto` reads the status to pick it.
    let registry = Registry::new();
    registry.register(Box::new(EsclLike::new(AdfState::Empty, 2))).expect("escl");
    let scanner = registry.scanner_details(&format!("escl:{ESCL_ID}")).expect("details");
    assert_eq!(scanner.feeder_ready, Some(false));
    assert_eq!(Source::Auto.resolve(scanner.feeder_ready, scanner.feeder, scanner.flatbed), Ok(Source::Glass));
    assert!(registry.acquire(&request("", Source::Glass, 1)).is_ok());
    // A jam blocks the glass too.
    let jammed = Registry::new();
    jammed.register(Box::new(EsclLike::new(AdfState::Jam, 2))).expect("escl");
    assert_eq!(jammed.acquire(&request("", Source::Glass, 1)).err().map(|e| e.interrupt_kind()), Some(InterruptKind::Jam));
}

#[test]
fn black_and_white_works_on_a_jpeg_page_from_a_backend_that_is_not_wia() {
    let registry = Registry::new();
    registry.register(Box::new(SaneLike::new(1))).expect("sane");
    let settings = ScanSettings { color: Color::BlackAndWhite, ..request("", Source::Glass, 1) };
    let delivered = registry.acquire(&settings).expect("acquire").pages.remove(0);
    assert_eq!(image_format(&delivered.bytes), "jpeg");
    let (page, from_bits) = page_for(&delivered.bytes, PageRule::for_settings(&settings));
    assert!(page.thresholded && from_bits.is_some());
    assert_eq!((page.format, page.width, page.height), ("bmp", 16, 8));
    let info = bmp::parse(&page.bytes).expect("a bmp");
    assert_eq!(info.bits, 1);
    // Colour requests leave the JPEG as it came, with its size read from the JPEG header.
    let plain = ScanSettings { color: Color::Gray, ..settings };
    let (kept, _) = page_for(&delivered.bytes, PageRule::for_settings(&plain));
    assert_eq!((kept.format, kept.width, kept.height, kept.thresholded), ("jpeg", 16, 8, false));
}

#[test]
fn native_codes_stay_in_their_own_domain() {
    let registry = all_three();
    let wia = ScanSettings { progress: Some(ProgressToken::new()), ..request(&format!("wia:{WIA_ID}"), Source::Feeder, 3) };
    let error = registry.acquire(&wia).expect_err("shortfall");
    // The WIA-shaped fake's HRESULT is replaced by the shortfall sentence here; ask it directly.
    let hresult = common::wia_like::error_from_hresult(0x8021_0006, "");
    let info = hresult.info(ErrorContext::default());
    assert_eq!((info.hresult(), info.code_name(), info.kind), (Some(0x8021_0006), Some("WIA_ERROR_BUSY"), InterruptKind::Busy));
    assert_eq!(info.native.map(|n| n.domain), Some("hresult"));
    assert_eq!(error.interrupt_kind(), InterruptKind::FeederEmpty);
}

#[test]
fn diagnostics_are_per_backend_text() {
    let registry = all_three();
    let all = registry.diagnostic_dump("").expect("dump");
    assert!(all.contains("== wia ==") && all.contains("== sane ==") && all.contains("== escl =="), "{all}");
    assert!(all.contains("WIA_DPS_DOCUMENT_HANDLING_STATUS") && all.contains("--resolution") && all.contains("AdfState"), "{all}");
    let one = registry.diagnostic_dump("sane:pixma").expect("targeted");
    assert!(one.contains("== sane ==") && !one.contains("== wia =="), "{one}");
}

#[test]
fn the_registry_names_are_checked() {
    struct Named(&'static str);
    impl craft_scan::Backend for Named {
        fn name(&self) -> &'static str {
            self.0
        }
        fn scanner_list(&self) -> Result<Vec<craft_scan::ScannerRef>, ScanError> {
            Ok(Vec::new())
        }
        fn scanner_details(&self, _: &str) -> Result<craft_scan::Scanner, ScanError> {
            Err(ScanError::Unsupported)
        }
        fn scanners(&self) -> Result<Vec<craft_scan::Scanner>, ScanError> {
            Ok(Vec::new())
        }
        fn diagnostic_dump(&self, _: &str) -> Result<String, ScanError> {
            Ok(String::new())
        }
        fn acquire(&self, _: &ScanSettings) -> Result<craft_scan::Acquired, ScanError> {
            Err(ScanError::Unsupported)
        }
    }
    let registry = Registry::new();
    for bad in ["", "Has Space", "a:b", "UPPER"] {
        assert_eq!(registry.register(Box::new(Named(bad))), Err(RegisterError::InvalidName { name: bad }));
    }
    assert!(registry.register(Box::new(Named("ok-name_2"))).is_ok());
    // The default `status` answers "cannot preflight".
    assert_eq!(registry.status("ok-name_2:x"), Ok(None));
}

#[test]
fn page_length_support_is_unknown_unless_a_backend_reads_it() {
    use craft_scan::{PageLengthSupport, Presence};
    let registry = all_three();
    for id in [format!("wia:{WIA_ID}"), format!("sane:{SANE_ID}"), format!("escl:{ESCL_ID}")] {
        let support = registry.page_length_support(&id, Source::Feeder).expect("routes");
        assert_eq!(support, PageLengthSupport::default(), "{id}");
        assert_eq!(support.device_detect, Presence::Unknown);
    }
    assert!(registry.page_length_support("nobody:x", Source::Glass).is_err());
    assert_eq!(Registry::new().page_length_support("sane:x", Source::Glass).err(), Some(ScanError::Unsupported));
    // A scanner row that never read an item reports nothing for either source.
    let scanner = common::scanner("x", "X");
    assert_eq!(scanner.length_support(Source::Glass), PageLengthSupport::default());
    assert_eq!(scanner.length_support(Source::Feeder), PageLengthSupport::default());
}
