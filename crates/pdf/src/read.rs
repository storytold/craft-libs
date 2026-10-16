//! Bounded opening and native interpreter guards. App policy remains with callers.
#[cfg(feature = "interpret")]
use crate::interpret;
use crate::syntax;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Maximum input accepted by the default open operation (512 MiB).
pub const MAX_INPUT_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("PDF exceeds the {limit}-byte input limit")]
    InputLimit { limit: usize },
    #[error("PDF input could not be allocated")]
    Allocation,
    #[error("PDF parser rejected the document: {0:?}")]
    Load(syntax::LoadPdfError),
    #[error("PDF parser failed internally")]
    BackendPanic,
}

/// Check the input bound before an application allocates or starts another PDF reader.
pub fn check_input_size(size: usize) -> Result<(), OpenError> {
    check_input_size_with(size, &OpenOptions::default())
}

/// Caller policy for accepted input and ordinary stream decoding, fixed for one document.
/// These limits are not a bound on total parser or process memory.
#[derive(Clone, Copy, Debug)]
pub struct OpenOptions {
    /// Largest file accepted before copying or parsing. Callers preserve their own file policy.
    pub max_input_bytes: usize,
    /// Ordinary-stream limits, including xref streams and repair/fallback paths.
    /// Structural repair or objects omitted by these limits have no separate warning;
    /// a successful open and empty rendering diagnostics do not certify completeness.
    /// Image-pixel and streaming page-content budgets remain separate.
    /// A bare Flate page uses the streaming budget during interpretation/rasterization;
    /// lowering this ordinary limit only bounds its materialized `page_stream` path.
    pub decode_limits: syntax::DecodeLimits,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self { max_input_bytes: MAX_INPUT_BYTES, decode_limits: syntax::DecodeLimits::default() }
    }
}

/// Check a caller's input policy before allocation or invoking another reader.
pub fn check_input_size_with(size: usize, options: &OpenOptions) -> Result<(), OpenError> {
    if size > options.max_input_bytes {
        return Err(OpenError::InputLimit { limit: options.max_input_bytes });
    }
    Ok(())
}

/// Open a PDF without copying its shared input. Password errors retain the backend's type
/// so applications can distinguish a missing/wrong password and implement owner-password
/// compatibility. Parsing is bounded by the backend's own structural limits too.
///
/// Native unwinding panics become errors. WASM aborts and process-wide allocation failures
/// cannot be caught; these guards do not replace input and allocation limits.
pub fn open(bytes: Arc<Vec<u8>>, password: Option<&str>) -> Result<syntax::Pdf, OpenError> {
    open_with_options(bytes, password, &OpenOptions::default())
}

/// Open shared input with explicit immutable caller limits.
pub fn open_with_options(bytes: Arc<Vec<u8>>, password: Option<&str>, options: &OpenOptions) -> Result<syntax::Pdf, OpenError> {
    check_input_size_with(bytes.len(), options)?;
    catch_unwind(AssertUnwindSafe(|| syntax::Pdf::new_with_password_and_limits(bytes, password.unwrap_or(""), options.decode_limits)))
        .map_err(|_| OpenError::BackendPanic)?
        .map_err(OpenError::Load)
}

/// Copy a borrowed input only after checking its size and reserving its storage.
pub fn open_slice(bytes: &[u8], password: Option<&str>) -> Result<syntax::Pdf, OpenError> {
    open_slice_with_options(bytes, password, &OpenOptions::default())
}

/// Copy borrowed input after checking caller limits and reserving storage fallibly.
pub fn open_slice_with_options(bytes: &[u8], password: Option<&str>, options: &OpenOptions) -> Result<syntax::Pdf, OpenError> {
    check_input_size_with(bytes.len(), options)?;
    let mut owned = Vec::new();
    owned.try_reserve_exact(bytes.len()).map_err(|_| OpenError::Allocation)?;
    owned.extend_from_slice(bytes);
    open_with_options(Arc::new(owned), password, options)
}

/// A guarded page interpreter entry point for editable import and text devices.
/// After an error the caller must discard the partially populated device and its cache.
/// This catches native unwinding panics, not WASM aborts.
#[cfg(feature = "interpret")]
pub fn interpret_page<'a>(
    page: &'a syntax::page::Page<'a>,
    context: &mut interpret::Context<'a>,
    device: &mut impl interpret::Device<'a>,
) -> Result<(), ExecutionError> {
    catch_unwind(AssertUnwindSafe(|| interpret::interpret_page(page, context, device))).map_err(|_| ExecutionError::BackendPanic)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    #[error("PDF page interpretation failed internally")]
    BackendPanic,
}
