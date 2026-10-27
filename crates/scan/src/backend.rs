//! Scanner backends and the registry that holds them.
//!
//! A platform backend (Windows WIA, SANE, eSCL over HTTP, macOS ImageCaptureCore, a test double)
//! needs platform calls and often `unsafe`, so none lives in this crate. The app implements
//! [`Backend`], registers it once at start-up with [`register`], and calls the free functions of
//! this crate ([`scanner_list`], [`acquire`] and the rest). Several backends can be registered at
//! once (Windows might have WIA and eSCL for a network printer); the functions aggregate across
//! them and route by device id. With none registered they answer [`ScanError::Unsupported`], which
//! is how a platform without a scanner stack behaves.
//!
//! # Device ids are namespaced
//!
//! A backend names devices with its own **native ids**. The registry shows and accepts them as
//! `"<backend>:<native id>"` (`wia:{...}`, `sane:pixma:libusb:001:004`, `escl:http://...`), where
//! `<backend>` is [`Backend::name`]. A native id may itself contain colons: only the first one
//! separates the prefix. [`scanner_details`] needs a registered prefix. [`acquire`] reads
//! `ScanSettings::device`, which may also be a name fragment or empty (see [`Registry::acquire`]).
//!
//! # Partial failure
//!
//! Listing asks every backend and keeps what answers. A backend that fails does not hide the
//! others' devices: the result is a [`Listing`] with the devices found and one [`BackendError`] per
//! backend that failed. The listing is `Err` only when no backend is registered at all.
//!
//! # Writing a backend
//!
//! Calls block, and the app calls them from worker threads. Cancellation and progress travel in
//! the [`ScanSettings`] given to [`Backend::acquire`] (`cancel`, `progress`): publish pages through
//! the `ProgressToken` as they arrive and stop when the `CancelToken` is requested.
//!
//! * **Capabilities are lists per source**, [`Scanner::glass`] and [`Scanner::feeder_choices`], and
//!   every list may be empty or `None`, which means "not published". A UI offers what is listed and
//!   falls back to the defaults when it is not.
//! * **Settings are requests.** Image format, compression, intent and page size are hints a backend
//!   may ignore; the `Choices` lists tell the UI what is honoured. DPI, colour and source are
//!   expected to be honoured or refused with an error.
//! * **Pages are encoded images**: [`Page::format`] and [`Page::bytes`] are whatever the backend
//!   delivered (BMP from WIA; JPEG, PNG, TIFF or PDF from eSCL, SANE or ImageCaptureCore). Nothing
//!   outside a Windows backend may assume BMP. Black-and-white enforcement ([`crate::bw`]) decodes
//!   JPEG, PNG and TIFF as well.
//! * **Status is optional.** [`Backend::status`] returns `None` when the backend cannot say what
//!   the paper path is doing before a scan (SANE reports an empty feeder only when the scan
//!   fails). Report such conditions as errors with their kind: [`ScanError::native_error`] with
//!   [`InterruptKind::FeederEmpty`]. Only the Windows WSD driver's "busy, or maybe empty" is
//!   [`InterruptKind::BusyOrFeederEmpty`]; other backends do not produce it.
//! * **Errors keep a neutral kind and the native code**: build them with
//!   [`ScanError::native_error`] and a [`NativeCode`] in the domain you own.
//! * **Partial results are kept**: when a later page fails, return [`ScanError::Incomplete`] with
//!   the pages that arrived.

use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use craft_job::{BackendName, TagSupport, namespaced_id, split_id};

#[cfg(doc)]
use crate::Page;
use crate::{Acquired, HandlingStatus, InterruptKind, NativeCode, PageLengthSupport, ScanError, ScanSettings, Scanner, ScannerRef, Source};

/// The [`NativeCode::domain`] of the errors the [`Registry`] itself raises when it cannot route a device id to a backend.
pub const REGISTRY_DOMAIN: &str = "registry";
/// The device id has no `<backend>:` prefix where one is required ([`Registry::scanner_details`]). Matchable with
/// [`ScanError::native`]; the message still names the registered backends.
pub const NO_BACKEND_PREFIX: NativeCode = NativeCode::new(REGISTRY_DOMAIN, 1, Some("NO_BACKEND_PREFIX"));
/// The `<backend>:` prefix names no registered backend.
pub const UNKNOWN_BACKEND: NativeCode = NativeCode::new(REGISTRY_DOMAIN, 2, Some("UNKNOWN_BACKEND"));
/// Several backends are registered and the device (a name fragment, or an unregistered prefix) cannot tell which one is meant.
pub const AMBIGUOUS_DEVICE: NativeCode = NativeCode::new(REGISTRY_DOMAIN, 3, Some("AMBIGUOUS_DEVICE"));

/// A routing failure of the registry: [`InterruptKind::Other`], with a code a caller can match instead of the sentence.
fn routing_error(code: NativeCode, message: String) -> ScanError {
    ScanError::native_error(InterruptKind::Other, code, message)
}

/// The operations an app layer needs from a scanner stack. Ids passed in and returned are the
/// backend's **native** ids; the [`Registry`] adds and strips the `"<name>:"` prefix.
pub trait Backend: Send + Sync {
    /// The backend's registry name and id prefix: short, lowercase ASCII letters, digits, `-` and
    /// `_` (`"wia"`, `"sane"`, `"escl"`, `"ica"`).
    fn name(&self) -> &'static str;

    /// The scanners the system lists, without opening any of them.
    fn scanner_list(&self) -> Result<Vec<ScannerRef>, ScanError>;

    /// Open one scanner (a native id from [`Backend::scanner_list`]) and read what it can do.
    /// Feeds no paper.
    fn scanner_details(&self, native_id: &str) -> Result<Scanner, ScanError>;

    /// Every scanner that opens, with its capabilities. A device that fails to open is left out and
    /// the others are still returned. Feeds no paper.
    fn scanners(&self) -> Result<Vec<Scanner>, ScanError>;

    /// What the paper path is doing now, when the backend can tell without scanning. `None` means
    /// it cannot preflight, which is not the same as "all clear". Feeds no paper.
    fn status(&self, native_id: &str) -> Option<HandlingStatus> {
        let _ = native_id;
        None
    }

    /// Whether the item for `source` publishes sheet-length detection, and how long a page it can scan
    /// (see [`crate::PageLength`]). **Unknown by default**: a backend overrides it only when it reads the
    /// item's properties. Feeds no paper. [`Backend::scanner_details`] reports the same answers in
    /// [`Scanner::length_support`](crate::Scanner::length_support).
    fn page_length_support(&self, native_id: &str, source: Source) -> PageLengthSupport {
        let _ = (native_id, source);
        PageLengthSupport::default()
    }

    /// Whether the backend can carry an idempotency tag through to a job listing, so that a scan whose
    /// connection dropped could be recognised on the device afterwards. **`None` for every backend there is**:
    /// WIA and the inbox WSD driver have no job tag to set, SANE has no job list, and nothing in
    /// [`ScanSettings`] carries a job name. eSCL has `ScanJobs` and WS-Scan has job status, which a backend
    /// could one day read; until one does, a scan whose outcome is unknown is `CheckDeviceFirst`
    /// (see `ScanJob`). Reads nothing from the device.
    fn tag_support(&self) -> TagSupport {
        TagSupport::None
    }

    /// Backend-specific diagnostic text for the devices matching `filter` (an empty filter is every
    /// device): WIA properties, SANE option descriptors, an eSCL capabilities document. Reads
    /// only; feeds no paper.
    fn diagnostic_dump(&self, filter: &str) -> Result<String, ScanError>;

    /// Scan: pull sheets from the feeder or read the glass and return each page in memory.
    /// `settings.device` is the native id (or fragment) with the registry prefix removed.
    fn acquire(&self, settings: &ScanSettings) -> Result<Acquired, ScanError>;
}

/// Why [`Registry::register`] refused a backend.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    /// A backend with this name is already registered; it stays in place.
    #[error("a scanner backend named {name:?} is already registered")]
    DuplicateName { name: &'static str },
    /// The name is empty or has a character other than lowercase ASCII, digits, `-` and `_`.
    #[error("{name:?} is not a valid scanner backend name")]
    InvalidName { name: &'static str },
}

/// One backend's failure inside a [`Listing`].
#[derive(Debug, PartialEq, Eq)]
pub struct BackendError {
    pub backend: &'static str,
    pub error: ScanError,
}

/// What the registered backends found. `items` holds every device that was found, with namespaced
/// ids; `errors` holds one entry for each backend that failed. A failing backend does not hide the
/// others' devices.
#[derive(Debug, PartialEq, Eq)]
pub struct Listing<T> {
    pub items: Vec<T>,
    pub errors: Vec<BackendError>,
}

impl<T> Listing<T> {
    /// No backend failed.
    pub fn is_complete(&self) -> bool {
        self.errors.is_empty()
    }
}

/// The scanner backends of a process. The free functions of this crate use one global registry;
/// tests and apps that want isolation can make their own.
#[derive(Default)]
pub struct Registry {
    backends: RwLock<Vec<Arc<dyn Backend>>>,
}

/// A registry name is valid when craft-job accepts it as a [`BackendName`]: one or more of `a-z`, `0-9`, `-`
/// and `_`. The empty name is refused by both.
fn valid_name(name: &str) -> bool {
    BackendName::new(name).is_ok()
}

impl Registry {
    /// An empty registry; backends are added with [`Self::register`].
    pub fn new() -> Self {
        Self::default()
    }

    /// The registered backends as cloned handles, so callers do not hold the lock while a backend blocks.
    fn snapshot(&self) -> Vec<Arc<dyn Backend>> {
        // Clone the handles out so no lock is held while a backend blocks.
        self.backends.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Add a backend. Several can be registered; a name registered twice is a
    /// [`RegisterError::DuplicateName`] (the new backend is dropped), never a panic.
    pub fn register(&self, backend: Box<dyn Backend>) -> Result<(), RegisterError> {
        let name = backend.name();
        if !valid_name(name) {
            return Err(RegisterError::InvalidName { name });
        }
        let mut backends = self.backends.write().unwrap_or_else(PoisonError::into_inner);
        if backends.iter().any(|existing| existing.name() == name) {
            return Err(RegisterError::DuplicateName { name });
        }
        backends.push(Arc::from(backend));
        Ok(())
    }

    /// The registered names, in registration order.
    pub fn backend_names(&self) -> Vec<&'static str> {
        self.snapshot().iter().map(|backend| backend.name()).collect()
    }

    /// Whether any backend is registered.
    pub fn supported(&self) -> bool {
        !self.snapshot().is_empty()
    }

    /// The backend names joined with ", ", for error messages.
    fn names_text(backends: &[Arc<dyn Backend>]) -> String {
        backends.iter().map(|backend| backend.name()).collect::<Vec<_>>().join(", ")
    }

    /// Runs `ask` on every backend and gathers the results, calling `namespace` on each item with its backend's name.
    /// A backend that fails is recorded in [`Listing::errors`] and does not stop the others; `Unsupported` only when
    /// the registry is empty.
    fn aggregate<T>(
        &self,
        ask: impl Fn(&dyn Backend) -> Result<Vec<T>, ScanError>,
        namespace: impl Fn(&mut T, &str),
    ) -> Result<Listing<T>, ScanError> {
        let backends = self.snapshot();
        if backends.is_empty() {
            return Err(ScanError::Unsupported);
        }
        let mut listing = Listing { items: Vec::new(), errors: Vec::new() };
        for backend in &backends {
            match ask(backend.as_ref()) {
                Ok(mut found) => {
                    for item in &mut found {
                        namespace(item, backend.name());
                    }
                    listing.items.append(&mut found);
                }
                Err(error) => listing.errors.push(BackendError { backend: backend.name(), error }),
            }
        }
        Ok(listing)
    }

    /// The scanners every backend lists, without opening any. Ids are `"<backend>:<native id>"`.
    /// `Err(Unsupported)` only when no backend is registered; see [`Listing`] for partial failure.
    pub fn scanner_list(&self) -> Result<Listing<ScannerRef>, ScanError> {
        self.aggregate(|backend| backend.scanner_list(), |item, name| item.id = namespaced_id(name, &item.id))
    }

    /// Every scanner that opens, from every backend, with namespaced ids. See [`Self::scanner_list`].
    pub fn scanners(&self) -> Result<Listing<Scanner>, ScanError> {
        self.aggregate(|backend| backend.scanners(), |item, name| item.id = namespaced_id(name, &item.id))
    }

    /// Picks the backend named by the `<backend>:` prefix of `id` and returns it with the native id that follows.
    /// `Unsupported` for an empty registry; a [`NO_BACKEND_PREFIX`] or [`UNKNOWN_BACKEND`] error when the prefix is
    /// missing or names no registered backend.
    fn by_prefix<'a>(backends: &'a [Arc<dyn Backend>], id: &'a str) -> Result<(&'a Arc<dyn Backend>, &'a str), ScanError> {
        if backends.is_empty() {
            return Err(ScanError::Unsupported);
        }
        let Some((prefix, native)) = split_id(id) else {
            return Err(routing_error(
                NO_BACKEND_PREFIX,
                format!("scanner id {id:?} has no backend prefix; ids look like <backend>:<id> (registered: {})", Self::names_text(backends)),
            ));
        };
        match backends.iter().find(|backend| backend.name() == prefix) {
            Some(backend) => Ok((backend, native)),
            None => Err(routing_error(UNKNOWN_BACKEND, format!("no scanner backend named {prefix:?} (registered: {})", Self::names_text(backends)))),
        }
    }

    /// Open one scanner by namespaced id and read what it can do. The returned id is namespaced.
    /// An id without a registered prefix is an error that names the registered backends.
    pub fn scanner_details(&self, id: &str) -> Result<Scanner, ScanError> {
        let backends = self.snapshot();
        let (backend, native) = Self::by_prefix(&backends, id)?;
        let mut scanner = backend.scanner_details(native)?;
        scanner.id = namespaced_id(backend.name(), &scanner.id);
        Ok(scanner)
    }

    /// [`Backend::status`] of one scanner by namespaced id. `Ok(None)`: the backend cannot preflight.
    pub fn status(&self, id: &str) -> Result<Option<HandlingStatus>, ScanError> {
        let backends = self.snapshot();
        let (backend, native) = Self::by_prefix(&backends, id)?;
        Ok(backend.status(native))
    }

    /// [`Backend::page_length_support`] of one scanner by namespaced id.
    pub fn page_length_support(&self, id: &str, source: Source) -> Result<PageLengthSupport, ScanError> {
        let backends = self.snapshot();
        let (backend, native) = Self::by_prefix(&backends, id)?;
        Ok(backend.page_length_support(native, source))
    }

    /// [`Backend::tag_support`] of the backend that owns the namespaced scanner id.
    pub fn tag_support(&self, id: &str) -> Result<TagSupport, ScanError> {
        let backends = self.snapshot();
        let (backend, _) = Self::by_prefix(&backends, id)?;
        Ok(backend.tag_support())
    }

    /// Diagnostic text from the backends: one section per backend, headed `== <name> ==`, with a
    /// failing backend's error in its section. A `filter` that starts with a registered prefix
    /// (`sane:pixma`) goes to that backend alone, without the prefix.
    pub fn diagnostic_dump(&self, filter: &str) -> Result<String, ScanError> {
        let backends = self.snapshot();
        if backends.is_empty() {
            return Err(ScanError::Unsupported);
        }
        let targeted = split_id(filter).and_then(|(prefix, rest)| backends.iter().find(|b| b.name() == prefix).map(|b| (b, rest)));
        let mut out = String::new();
        let mut section = |backend: &Arc<dyn Backend>, filter: &str| {
            out.push_str(&format!("== {} ==\n", backend.name()));
            match backend.diagnostic_dump(filter) {
                Ok(text) => {
                    out.push_str(&text);
                    if !text.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Err(error) => out.push_str(&format!("error: {error}\n")),
            }
        };
        match targeted {
            Some((backend, rest)) => section(backend, rest),
            None => backends.iter().for_each(|backend| section(backend, filter)),
        }
        Ok(out)
    }

    /// Scan. `settings.device` picks the backend:
    ///
    /// * `"<backend>:<native id>"` with a registered prefix goes to that backend, prefix removed;
    /// * an empty device goes to the only backend, or, with several, to the first one that lists a
    ///   scanner (the backend then picks its own default);
    /// * anything else (a name fragment) goes to the only backend unchanged, and with several
    ///   backends is an [`AMBIGUOUS_DEVICE`] error that names them, since it cannot be told which one is meant.
    pub fn acquire(&self, settings: &ScanSettings) -> Result<Acquired, ScanError> {
        let backends = self.snapshot();
        let Some(first) = backends.first() else { return Err(ScanError::Unsupported) };
        let device = settings.device.as_str();
        let prefixed = split_id(device).and_then(|(prefix, rest)| backends.iter().find(|b| b.name() == prefix).map(|b| (b, rest)));
        let (backend, native): (&Arc<dyn Backend>, &str) = if let Some(found) = prefixed {
            found
        } else if backends.len() == 1 {
            (first, device)
        } else if device.trim().is_empty() {
            let listed = backends.iter().find(|b| b.scanner_list().is_ok_and(|list| !list.is_empty()));
            (listed.unwrap_or(first), "")
        } else {
            return Err(routing_error(
                AMBIGUOUS_DEVICE,
                format!("device {device:?} does not name a registered backend ({}); use <backend>:<id>", Self::names_text(&backends)),
            ));
        };
        let mut routed = settings.clone();
        routed.device = native.to_string();
        backend.acquire(&routed)
    }
}

static GLOBAL: OnceLock<Registry> = OnceLock::new();

/// The process-wide registry the free functions use.
pub fn global() -> &'static Registry {
    GLOBAL.get_or_init(Registry::new)
}

/// Register a backend with the process-wide registry. Call it early, once per backend. See
/// [`Registry::register`].
pub fn register(backend: Box<dyn Backend>) -> Result<(), RegisterError> {
    global().register(backend)
}

/// The names of the registered backends. See [`Registry::backend_names`].
pub fn backend_names() -> Vec<&'static str> {
    global().backend_names()
}

/// Whether any backend is registered. It is a run-time answer: every platform compiles the same
/// code and reports [`ScanError::Unsupported`] when a scan is asked for and no backend is there.
pub fn supported() -> bool {
    global().supported()
}

/// The scanners the backends list, without opening any. See [`Registry::scanner_list`].
pub fn scanner_list() -> Result<Listing<ScannerRef>, ScanError> {
    global().scanner_list()
}

/// One scanner's capabilities. See [`Registry::scanner_details`].
pub fn scanner_details(id: &str) -> Result<Scanner, ScanError> {
    global().scanner_details(id)
}

/// Every scanner that opens. See [`Registry::scanners`].
pub fn scanners() -> Result<Listing<Scanner>, ScanError> {
    global().scanners()
}

/// What the paper path is doing before a scan. See [`Registry::status`].
pub fn status(id: &str) -> Result<Option<HandlingStatus>, ScanError> {
    global().status(id)
}

/// Sheet-length detection the scanner publishes. See [`Registry::page_length_support`].
pub fn page_length_support(id: &str, source: Source) -> Result<PageLengthSupport, ScanError> {
    global().page_length_support(id, source)
}

/// Whether the scanner's backend can carry a tag. See [`Registry::tag_support`].
pub fn tag_support(id: &str) -> Result<TagSupport, ScanError> {
    global().tag_support(id)
}

/// Backend diagnostics. See [`Registry::diagnostic_dump`].
pub fn diagnostic_dump(filter: &str) -> Result<String, ScanError> {
    global().diagnostic_dump(filter)
}

/// Scan. See [`Registry::acquire`].
pub fn acquire(settings: &ScanSettings) -> Result<Acquired, ScanError> {
    global().acquire(settings)
}
