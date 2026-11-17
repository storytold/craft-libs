//! Opt-in complete image validation and checked-DOM normalization.
//! Legacy ImageResolver entry points retain their existing behavior.
use super::{Error, ImageUsage, Limits, Preflight, image_href, preflight, raster_format, usvg};
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

    fn raster(&mut self, kind: &usvg::ImageKind) -> bool {
        let encoded = match kind {
            usvg::ImageKind::SVG(_) => return true,
            usvg::ImageKind::PNG(data) | usvg::ImageKind::JPEG(data) | usvg::ImageKind::GIF(data) | usvg::ImageKind::WEBP(data) => data,
        };
        // usvg supplies a fresh Arc for every data-URL resolution, including
        // repeated <use> instances. Charge each actual validation, never retain
        // encoded buffers in an address-keyed memo that cannot hit this boundary.
        let Some(left) = self.encoded_left.checked_sub(encoded.len()) else {
            self.raster_limits = self.raster_limits.saturating_add(1);
            return false;
        };
        self.encoded_left = left;
        let result = super::draw::validate_raster(kind, self.pixel_limit, &mut self.pixels_left);
        match result {
            Ok(()) => true,
            Err(super::draw::RasterValidationFailure::Invalid) => {
                self.invalid_raster = self.invalid_raster.saturating_add(1);
                false
            }
            Err(super::draw::RasterValidationFailure::Limit) => {
                self.raster_limits = self.raster_limits.saturating_add(1);
                false
            }
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
        convert_checked(checked, &options)
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
                let detected = raster_format(&data);
                let format = match mime {
                    "image/jpg" | "image/jpeg" => (detected == Some(craft_image_codecs::Format::Jpeg)).then_some(craft_image_codecs::Format::Jpeg),
                    "image/png" => (detected == Some(craft_image_codecs::Format::Png)).then_some(craft_image_codecs::Format::Png),
                    "image/gif" => (detected == Some(craft_image_codecs::Format::Gif)).then_some(craft_image_codecs::Format::Gif),
                    "image/webp" => (detected == Some(craft_image_codecs::Format::WebP)).then_some(craft_image_codecs::Format::WebP),
                    "text/plain" => detected,
                    _ => None,
                };
                if let Some(format) = format {
                    let kind = match format {
                        craft_image_codecs::Format::Png => usvg::ImageKind::PNG(data),
                        craft_image_codecs::Format::Jpeg => usvg::ImageKind::JPEG(data),
                        craft_image_codecs::Format::Gif => usvg::ImageKind::GIF(data),
                        craft_image_codecs::Format::WebP => usvg::ImageKind::WEBP(data),
                        _ => return None,
                    };
                    return this.state.lock().unwrap_or_else(PoisonError::into_inner).raster(&kind).then_some(kind);
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
        let text = match svgz::text(data, room.min(self.limits.bytes)) {
            Ok(text) if text.len() <= room => text,
            error => {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.svg_bytes_left = 0;
                if matches!(error, Err(svgz::Error::TooLarge(_))) || error.is_ok() {
                    state.svg_limits = state.svg_limits.saturating_add(1);
                    state.svg_resource_limits = state.svg_resource_limits.saturating_add(1);
                } else {
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
        let checked = match preflight(text, &self.limits) {
            Ok(checked) => checked,
            Err(error) => {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                match error {
                    Error::TooDeep(_) => state.svg_limits = state.svg_limits.saturating_add(1),
                    Error::Text(svgz::Error::TooLarge(_)) => {
                        state.svg_limits = state.svg_limits.saturating_add(1);
                        state.svg_resource_limits = state.svg_resource_limits.saturating_add(1);
                    }
                    _ => state.invalid_svg = state.invalid_svg.saturating_add(1),
                }
                return None;
            }
        };
        let scope = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(total) = state.xml_depth.checked_add(checked.height).filter(|depth| *depth <= self.limits.tree_depth) else {
                state.svg_limits = state.svg_limits.saturating_add(1);
                return None;
            };
            let previous = state.xml_depth;
            state.xml_depth = total;
            state.depth = state.depth.saturating_add(1);
            state.svg_depth = state.svg_depth.max(state.depth);
            Scope { state: Arc::clone(&self.state), previous, child: true }
        };
        let result = convert_checked(checked, options);
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

fn convert_checked(checked: Preflight<'_>, options: &usvg::Options<'_>) -> Result<usvg::Tree, Error> {
    match image_href::normalize(&checked)? {
        std::borrow::Cow::Borrowed(_) => checked.parse(options),
        std::borrow::Cow::Owned(text) => {
            let limits = checked.limits;
            drop(checked);
            preflight(&text, &limits)?.parse(options)
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod xml_tests;
