//! Opt-in complete image validation and checked-DOM normalization.
//! Legacy ImageResolver entry points retain their existing behavior.
use super::draw::{RasterValidationFailure, decode_raster, probe_raster_pixels};
use super::{
    ChildRefusal, Error, ImageUsage, Limits, Preflight, child_total_depth, declared_raster, embedded_text, image_href, preflight, preflight_child,
    usvg,
};
use crate::svgz;
use std::sync::{Arc, Mutex, PoisonError};

struct State {
    document_bytes: usize,
    pixel_limit: u64,
    pixels_left: u64,
    svg_byte_limit: usize,
    svg_bytes_left: usize,
    svg_failure_exhausted: bool,
    classification_bytes_left: usize,
    classification_content_left: usize,
    classification_text_bytes_left: usize,
    encoded_left: usize,
    invalid_raster: usize,
    invalid_svg: usize,
    raster_limits: usize,
    svg_limits: usize,
    svg_resource_limits: usize,
    root_active: bool,
    xml_depth: usize,
    depth: usize,
    svg_count: usize,
    svg_depth: usize,
}

impl State {
    fn new(limits: Limits) -> Self {
        Self {
            document_bytes: limits.bytes,
            pixel_limit: limits.embedded_raster_pixels,
            pixels_left: limits.embedded_raster_pixels,
            svg_byte_limit: limits.embedded_bytes,
            svg_bytes_left: limits.embedded_bytes,
            svg_failure_exhausted: false,
            classification_bytes_left: limits.embedded_bytes,
            classification_content_left: limits.embedded_bytes,
            classification_text_bytes_left: limits.embedded_bytes,
            encoded_left: limits.bytes.saturating_add(limits.embedded_bytes),
            invalid_raster: 0,
            invalid_svg: 0,
            raster_limits: 0,
            svg_limits: 0,
            svg_resource_limits: 0,
            root_active: false,
            xml_depth: 0,
            depth: 0,
            svg_count: 0,
            svg_depth: 0,
        }
    }

    fn usage(&self) -> ImageUsage {
        ImageUsage {
            raster_pixels: self.pixel_limit.saturating_sub(self.pixels_left),
            embedded_svg_bytes: self.svg_byte_limit.saturating_sub(self.svg_bytes_left),
            embedded_svg_count: self.svg_count,
            embedded_svg_depth: self.svg_depth,
            invalid_raster_images: self.invalid_raster,
            raster_limit_images: self.raster_limits,
            invalid_svg_images: self.invalid_svg,
            svg_limit_images: self.svg_limits,
            svg_resource_limit_images: self.svg_resource_limits,
        }
    }

    fn classify_after_svg_failure(&mut self, data: &[u8]) {
        if self.svg_resource_limits != 0 {
            self.svg_limits = self.svg_limits.saturating_add(1);
            self.svg_resource_limits = self.svg_resource_limits.saturating_add(1);
            return;
        }
        let compressed = data.starts_with(&[0x1f, 0x8b]);
        let content_left = self.classification_content_left;
        let limit = if compressed { content_left.min(self.classification_bytes_left) } else { content_left }.min(self.document_bytes);
        if compressed && limit == 0 {
            self.svg_limits = self.svg_limits.saturating_add(1);
            self.svg_resource_limits = self.svg_resource_limits.saturating_add(1);
            return;
        }
        let resource_refusal = match crate::svgz::inflate(data, limit) {
            Ok(bytes) => {
                if compressed {
                    self.classification_bytes_left = self.classification_bytes_left.saturating_sub(bytes.len());
                }
                let Some(text_left) = self.classification_text_bytes_left.checked_sub(bytes.len()) else {
                    self.svg_limits = self.svg_limits.saturating_add(1);
                    self.svg_resource_limits = self.svg_resource_limits.saturating_add(1);
                    return;
                };
                self.classification_text_bytes_left = text_left;
                match crate::svgz::decode(&bytes) {
                    Ok(text) => text.len() > content_left || text.len() > self.document_bytes,
                    Err(_) => {
                        self.invalid_svg = self.invalid_svg.saturating_add(1);
                        return;
                    }
                }
            }
            Err(crate::svgz::Error::TooLarge(_)) => {
                if compressed {
                    self.classification_bytes_left = self.classification_bytes_left.saturating_sub(limit);
                }
                true
            }
            Err(error) => {
                // Nested data was fully inflated before refusal. Other failures
                // may have consumed the entire attempted inflation allowance.
                if compressed {
                    let charged = match error {
                        crate::svgz::Error::Nested(bytes) => bytes,
                        _ => limit,
                    };
                    self.classification_bytes_left = self.classification_bytes_left.saturating_sub(charged);
                }
                self.invalid_svg = self.invalid_svg.saturating_add(1);
                return;
            }
        };
        self.svg_limits = self.svg_limits.saturating_add(1);
        if resource_refusal {
            self.svg_resource_limits = self.svg_resource_limits.saturating_add(1);
        }
    }
}

/// One opt-in drawing's image work, normalization and exact rendering validation.
///
/// Unlike legacy ImageResolver, every attempted raster callback spends encoded
/// and decoded work, even on failure. Rendering still decodes pixels afterwards:
/// resvg does not expose a decoded-pixel injection or decode-result hook.
/// All root and child conversions install this same resolver themselves.
#[derive(Clone)]
pub struct ValidatedImageResolver {
    limits: Limits,
    state: Arc<Mutex<State>>,
}

impl ValidatedImageResolver {
    /// Create a separate budget for one drawing. Encoded raster work is bounded
    /// by root text bytes plus aggregate child SVG bytes, without retaining data.
    pub fn new(limits: Limits) -> Self {
        Self { limits, state: Arc::new(Mutex::new(State::new(limits))) }
    }

    /// The immutable admission policy captured by this instance. Remaining work
    /// never changes these limits; use them for the matching opaque preflight.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Snapshot admitted work and optional-image omissions.
    pub fn usage(&self) -> ImageUsage {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).usage()
    }

    /// Convert exactly the DOM checked under these limits, with this instance's
    /// resolver. A caller-supplied image callback cannot bypass image admission.
    /// The caller still supplies the stack/worker and font policy.
    pub fn parse_preflight(&self, checked: Preflight<'_>, mut options: usvg::Options<'_>) -> Result<usvg::Tree, Error> {
        if checked.limits != self.limits {
            return Err(Error::Parse("checked SVG limits differ from the validated image resolver".into()));
        }
        // A shallow source can expand through use/paint references. Reserve its
        // expanded ancestor frames, excluding the callback leaf itself. The child
        // contributes its complete checked height, as in legacy child admission.
        let _scope = self.root_scope(checked.height.saturating_sub(1))?;
        options.image_href_resolver = self.resolver();
        self.convert_checked(checked, &options)
    }

    /// Resolve an already-decoded child SVG, charging original text before XML
    /// parsing or repair. This is not an unchecked/precharged admission hook.
    /// As with parse_preflight, unrelated image callbacks are replaced.
    pub fn resolve_text(&self, text: &str, mut options: usvg::Options<'_>) -> Result<Option<usvg::ImageKind>, Error> {
        let _scope = self.root_scope(0)?;
        options.image_href_resolver = self.resolver();
        Ok(self.embedded_text(text, &options))
    }

    fn root_scope(&self, depth: usize) -> Result<Scope, Error> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.root_active {
            return Err(Error::Parse("the validated SVG resolver is already converting a drawing".into()));
        }
        let previous = state.xml_depth;
        state.root_active = true;
        state.xml_depth = depth;
        Ok(Scope { state: Arc::clone(&self.state), previous, child: false })
    }

    fn resolver(&self) -> usvg::ImageHrefResolver<'static> {
        let this = self.clone();
        usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            resolve_data: Box::new(move |mime, data, options| {
                if let Some(format) = declared_raster(mime, &data) {
                    return this.raster_with(&data, |pixels| decode_raster(format, &data, pixels)).then(|| format.image_kind(data));
                }
                if !matches!(mime, "image/svg+xml" | "text/plain") {
                    if mime.starts_with("image/") {
                        let mut state = this.state.lock().unwrap_or_else(PoisonError::into_inner);
                        state.invalid_raster = state.invalid_raster.saturating_add(1);
                    }
                    return None;
                }
                this.embedded_bytes(&data, options)
            }),
        }
    }

    fn raster_with(&self, data: &[u8], decode: impl FnOnce(u64) -> Result<(), RasterValidationFailure>) -> bool {
        let result = self.reserve_raster(data).and_then(decode);
        if let Err(error) = result {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            match error {
                RasterValidationFailure::Invalid => state.invalid_raster = state.invalid_raster.saturating_add(1),
                RasterValidationFailure::Limit => state.raster_limits = state.raster_limits.saturating_add(1),
            }
            return false;
        }
        true
    }

    fn reserve_raster(&self, data: &[u8]) -> Result<u64, RasterValidationFailure> {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            // usvg uses a fresh Arc for each data-URL callback. Each actual validation
            // spends encoded work, including when the header or decoder refuses it.
            state.encoded_left = state.encoded_left.checked_sub(data.len()).ok_or(RasterValidationFailure::Limit)?;
        }
        let pixels = probe_raster_pixels(data, self.limits.embedded_raster_pixels)?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.pixels_left = state.pixels_left.checked_sub(pixels).ok_or(RasterValidationFailure::Limit)?;
        // The caller decodes after releasing the mutex. The work stays charged.
        Ok(pixels)
    }

    fn can_start_child(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.depth >= self.limits.embedded_depth {
            state.svg_limits = state.svg_limits.saturating_add(1);
            state.svg_resource_limits = state.svg_resource_limits.saturating_add(1);
            return false;
        }
        true
    }

    fn embedded_bytes(&self, data: &[u8], options: &usvg::Options<'_>) -> Option<usvg::ImageKind> {
        if !self.can_start_child() {
            return None;
        }
        let room = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.svg_failure_exhausted {
                state.classify_after_svg_failure(data);
                return None;
            }
            state.svg_bytes_left
        };
        let text = match embedded_text(data, room, &self.limits) {
            Ok(text) => text,
            Err(error) => {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if matches!(error, svgz::Error::Encoding) && !crate::sniff::is_svgz(data) {
                    // Plain text validation can spend at most the input's bytes.
                    // An invalid encoding does not exhaust a later sibling's room.
                    state.svg_bytes_left = state.svg_bytes_left.saturating_sub(data.len());
                    state.invalid_svg = state.invalid_svg.saturating_add(1);
                    return None;
                }
                state.svg_bytes_left = 0;
                if matches!(error, svgz::Error::TooLarge(_)) {
                    state.svg_limits = state.svg_limits.saturating_add(1);
                    state.svg_resource_limits = state.svg_resource_limits.saturating_add(1);
                } else {
                    // An unreadable compressed stream may have used the entire
                    // allowed inflation work. Later callbacks can only classify.
                    state.invalid_svg = state.invalid_svg.saturating_add(1);
                    state.svg_failure_exhausted = true;
                    state.classification_content_left = room;
                }
                return None;
            }
        };
        self.precharge_and_parse(&text, options)
    }

    fn embedded_text(&self, text: &str, options: &usvg::Options<'_>) -> Option<usvg::ImageKind> {
        if !self.can_start_child() {
            return None;
        }
        self.precharge_and_parse(text, options)
    }

    fn precharge_and_parse(&self, text: &str, options: &usvg::Options<'_>) -> Option<usvg::ImageKind> {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(left) = state.svg_bytes_left.checked_sub(text.len()) else {
                state.svg_bytes_left = 0;
                state.svg_limits = state.svg_limits.saturating_add(1);
                state.svg_resource_limits = state.svg_resource_limits.saturating_add(1);
                return None;
            };
            // Original UTF-8 bytes/work are never refunded when repair shortens
            // an attribute or conversion subsequently fails.
            state.svg_bytes_left = left;
        }
        let checked = match preflight_child(text, &self.limits) {
            Ok(checked) => checked,
            Err(error) => {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                match error {
                    ChildRefusal::Depth => state.svg_limits = state.svg_limits.saturating_add(1),
                    ChildRefusal::Bytes => {
                        state.svg_limits = state.svg_limits.saturating_add(1);
                        state.svg_resource_limits = state.svg_resource_limits.saturating_add(1);
                    }
                    ChildRefusal::Invalid => state.invalid_svg = state.invalid_svg.saturating_add(1),
                }
                return None;
            }
        };
        let scope = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(total) = child_total_depth(state.xml_depth, &checked, &self.limits) else {
                state.svg_limits = state.svg_limits.saturating_add(1);
                return None;
            };
            let previous = state.xml_depth;
            // Admission includes the child's complete height, but recursive
            // conversion reserves ancestors only, not its image callback leaf.
            state.xml_depth = total.saturating_sub(1);
            state.depth = state.depth.saturating_add(1);
            state.svg_depth = state.svg_depth.max(state.depth);
            Scope { state: Arc::clone(&self.state), previous, child: true }
        };
        let result = self.convert_checked(checked, options);
        drop(scope);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match result {
            Ok(tree) => {
                state.svg_count = state.svg_count.saturating_add(1);
                Some(usvg::ImageKind::SVG(tree))
            }
            Err(_) => {
                state.invalid_svg = state.invalid_svg.saturating_add(1);
                None
            }
        }
    }
    fn convert_checked(&self, checked: Preflight<'_>, options: &usvg::Options<'_>) -> Result<usvg::Tree, Error> {
        match image_href::normalize(&checked) {
            Ok(std::borrow::Cow::Borrowed(_)) => checked.parse(options),
            Ok(std::borrow::Cow::Owned(text)) => {
                let limits = checked.limits;
                drop(checked);
                preflight(&text, &limits)?.parse(options)
            }
            Err(repair_error) => {
                // Keep the original checked DOM if its URLs are safe to pass to
                // usvg. Malformed optional URLs are dropped by its parser.
                let omitted = image_href::unrepaired_image_omissions(&checked).ok_or(repair_error)?;
                let tree = checked.parse(options)?;
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.invalid_raster = state.invalid_raster.saturating_add(omitted);
                Ok(tree)
            }
        }
    }
}

struct Scope {
    state: Arc<Mutex<State>>,
    previous: usize,
    child: bool,
}
impl Drop for Scope {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.xml_depth = self.previous;
        if self.child {
            state.depth = state.depth.saturating_sub(1);
        } else {
            state.root_active = false;
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod xml_tests;
