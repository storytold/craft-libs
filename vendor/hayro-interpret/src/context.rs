use crate::cache::{Cache, CacheKey, DecodedImageCache};
use crate::color::ColorSpace;
use crate::convert::convert_transform;
use crate::font::{Font, StandardFont};
use crate::interpret::state::{ClipType, State, TextStateFont};
use crate::ocg::OcgState;
use crate::util::{BezPathExt, Float64Ext};
use crate::{ClipPath, Device, FillRule, InterpreterSettings, InterpreterWarning, StrokeProps};
use hayro_syntax::content::ops::Transform;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::Stream;
use hayro_syntax::page::Resources;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Maximum nesting depth for interpreting `XObject`'s/patterns/streams.
pub(crate) const MAX_NESTED_INTERPRETATION_DEPTH: u32 = 50;
/// PdfCraft patch: maximum nesting for content that is painted recursively by the device
/// (tiling patterns, Type 3 glyphs). Either can refer back to itself through resources it
/// inherits, which recursed until the stack overflowed (found by fuzzing).
pub(crate) const MAX_PAINT_NESTING: u32 = 16;

/// PdfCraft patch: how many nested paints (a form inside a form, a Type 3 glyph or tiling
/// pattern painted from inside another one) one page may do. Nesting depth alone doesn't bound
/// the work: a glyph that shows ten glyphs of its own font, nested sixteen deep, is 10^16
/// paints (a fuzzed file hung). Ordinary pages stay far below this.
pub(crate) const MAX_NESTED_PAINTS: u32 = 50_000;

std::thread_local! {
    static NESTED_PAINTS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
    static CONTENT_LEFT: core::cell::Cell<usize> = const { core::cell::Cell::new(MAX_PAGE_CONTENT) };
    // PdfCraft patch (18): whether this page's content budget ran out, reported as
    // `InterpreterWarning::ContentTruncated`.
    static CONTENT_TRUNCATED: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
    // PdfCraft patch (89): the form XObjects being drawn on this page, outermost first, across
    // the contexts of the tiling patterns, Type 3 glyphs, soft masks and groups painted while
    // they draw. A form already on it draws itself and is skipped. Its length is bounded by the
    // nesting limits: every form on it is nested in the ones before it.
    static FORMS_DRAWING: RefCell<Vec<hayro_syntax::object::ObjectIdentifier>> =
        const { RefCell::new(Vec::new()) };
}

// PdfCraft patch (#7380).
/// The Type 3 glyph procedures some content is drawn inside, outermost first. A procedure that
/// shows its own glyph describes nothing finite, and is not drawn inside itself (see
/// `Type3::render_glyph`). A glyph takes the list from the content that shows it, and a soft
/// mask, tiling pattern or CMYK group from the content that sets it, so the list holds where the
/// content is, however late a device interprets it: a soft mask set on the page and drawn while a
/// glyph paints may show that glyph. Every procedure on it is a different one.
#[derive(Clone, Default, Hash)]
pub(crate) struct GlyphProcs(Rc<[hayro_syntax::object::ObjectIdentifier]>);

impl GlyphProcs {
    /// Whether the procedure `id` is among these: whether it is drawing itself.
    pub(crate) fn contains(&self, id: hayro_syntax::object::ObjectIdentifier) -> bool {
        self.0.contains(&id)
    }

    /// These and then `id`, for the content of the procedure `id`.
    pub(crate) fn with(&self, id: hayro_syntax::object::ObjectIdentifier) -> Self {
        Self(self.0.iter().copied().chain([id]).collect())
    }
}

/// PdfCraft patch: decoded content-stream bytes one page may hold at once: its contents, and
/// the forms, tiling patterns and Type 3 glyph procedures being painted. Each stream already
/// stops at `MAX_DECODED_STREAM`, but forms nest fifty deep and every level keeps its content
/// while the next one runs: a form painting itself around a stream that inflates to 256 MiB
/// would have held over 12 GB. A level gives its bytes back when it finishes, so content
/// painted again and again doesn't use the budget up. Ordinary pages hold a few megabytes.
pub(crate) const MAX_PAGE_CONTENT: usize = 512 << 20;

/// PdfCraft patch: a content stream decoded within the page's [`MAX_PAGE_CONTENT`]. The bytes
/// it decoded count against the budget until it is dropped; a stream without filters is
/// borrowed from the file and costs nothing.
pub(crate) struct DecodedContent<'a> {
    data: Cow<'a, [u8]>,
    held: usize,
}

impl AsRef<[u8]> for DecodedContent<'_> {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Drop for DecodedContent<'_> {
    fn drop(&mut self) {
        let held = self.held;
        CONTENT_LEFT.with(|c| c.set(c.get().saturating_add(held).min(MAX_PAGE_CONTENT)));
    }
}

/// PdfCraft patch: decode a content stream within what is left of this page's
/// [`MAX_PAGE_CONTENT`]. `None` once the budget is spent (the content is skipped).
pub(crate) fn decode_content<'a>(stream: &Stream<'a>) -> Option<DecodedContent<'a>> {
    let left = CONTENT_LEFT.with(core::cell::Cell::get);
    if left == 0 {
        CONTENT_TRUNCATED.with(|truncated| truncated.set(true));
        warn!("page content budget exceeded");
        return None;
    }
    let filtered = !stream.filters().is_empty();
    let limit = if filtered { left.min(stream.decode_limits().max_decoded_stream()) } else { left };
    let Ok(data) = stream.decoded_within(limit) else {
        CONTENT_TRUNCATED.with(|truncated| truncated.set(true));
        return None;
    };
    // The whole-stream decoders do not expose completion. Conservatively report a
    // filtered form, pattern or Type 3 procedure reaching its effective budget.
    if filtered && data.len() >= limit {
        CONTENT_TRUNCATED.with(|truncated| truncated.set(true));
    }
    let held = match &data {
        Cow::Owned(v) => v.len(),
        Cow::Borrowed(_) => 0,
    };
    charge_content(held);
    Some(DecodedContent { data, held })
}

/// PdfCraft patch (87): what is left of this page's [`MAX_PAGE_CONTENT`].
pub(crate) fn content_left() -> usize {
    CONTENT_LEFT.with(core::cell::Cell::get)
}

/// PdfCraft patch (87): `len` bytes of content held within the page's [`MAX_PAGE_CONTENT`]
/// until dropped, as [`DecodedContent`] holds what it decoded: content kept from an earlier
/// decode (a cached Type 3 glyph procedure) counts as it did when it was decoded.
pub(crate) struct HeldContent(usize);

impl Drop for HeldContent {
    fn drop(&mut self) {
        let held = self.0;
        CONTENT_LEFT.with(|c| c.set(c.get().saturating_add(held).min(MAX_PAGE_CONTENT)));
    }
}

/// PdfCraft patch (87): hold `len` bytes of content within what is left of the page's
/// [`MAX_PAGE_CONTENT`]. `None`, and the page reported truncated, when they don't fit, as
/// [`decode_content`] skips content the budget can't hold.
pub(crate) fn hold_content(len: usize) -> Option<HeldContent> {
    let left = content_left();
    if left == 0 || left < len {
        CONTENT_TRUNCATED.with(|truncated| truncated.set(true));
        warn!("page content budget exceeded");
        return None;
    }
    charge_content(len);
    Some(HeldContent(len))
}

/// PdfCraft patch: count `len` bytes held for the rest of the page (its own contents).
pub(crate) fn charge_content(len: usize) {
    CONTENT_LEFT.with(|c| c.set(c.get().saturating_sub(len)));
}

/// PdfCraft patch (18): whether content was skipped because the page's budget ran out.
pub(crate) fn content_was_truncated() -> bool {
    CONTENT_TRUNCATED.with(core::cell::Cell::get)
}

/// PdfCraft patch: count a paint at `depth` (top-level ones are free); `false` once this
/// page's budget is spent, and the paint is skipped.
pub(crate) fn take_nested_paint(depth: u32) -> bool {
    if depth < 2 {
        return true;
    }
    NESTED_PAINTS.with(|n| {
        let used = n.get();
        if used >= MAX_NESTED_PAINTS {
            return false;
        }
        n.set(used + 1);
        true
    })
}

/// A cache used by the interpreter.
///
/// Ideally, such a cache should be constructed once per PDF and then reused across
/// multiple interpreter invocations on the same document.
#[derive(Clone)]
pub struct InterpreterCache<'a> {
    pub(crate) font_cache: Rc<RefCell<HashMap<u128, Option<Font<'a>>>>>,
    pub(crate) object_cache: Cache,
    /// PdfCraft patch: decoded images, shared by every render through this cache so a tile
    /// no longer decodes its page's images again.
    pub(crate) decoded_cache: DecodedImageCache,
    /// PdfCraft patch (87): what the Type 3 fonts of this cache keep of their decoded glyph
    /// procedures, together (see `font::type3`).
    pub(crate) type3_procs: Rc<Type3Procs>,
}

/// PdfCraft patch (87): decoded Type 3 glyph procedure bytes all the fonts of one
/// [`InterpreterCache`] keep together. Fonts live as long as the cache, so the bound is on the
/// cache, not on each font: a document can have thousands of Type 3 fonts.
pub(crate) const MAX_CACHED_TYPE3_PROCS: usize = 32 << 20;

/// PdfCraft patch (87): the decoded glyph procedure bytes the Type 3 fonts of a cache keep,
/// and how many procedures they decoded.
#[derive(Default)]
pub(crate) struct Type3Procs {
    pub(crate) held: core::cell::Cell<usize>,
    pub(crate) decodes: core::cell::Cell<usize>,
}

impl<'a> Default for InterpreterCache<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> InterpreterCache<'a> {
    /// Create a new interpreter cache.
    pub fn new() -> Self {
        Self {
            font_cache: Rc::new(RefCell::new(HashMap::new())),
            object_cache: Cache::new(),
            decoded_cache: DecodedImageCache::new(),
            type3_procs: Rc::default(),
        }
    }

    /// PdfCraft patch (87): how many Type 3 glyph procedures the fonts of this cache decoded,
    /// for diagnostics and tests. A procedure kept from an earlier copy of its glyph isn't
    /// decoded again.
    pub fn type3_proc_decodes(&self) -> usize {
        self.type3_procs.decodes.get()
    }

    /// PdfCraft patch (32): how many images were decoded through this cache (its misses, not
    /// counting images that are never cached), for diagnostics and tests.
    pub fn decoded_image_decodes(&self) -> usize {
        self.decoded_cache.decodes()
    }

    /// PdfCraft patch (82): how many of [`Self::decoded_image_decodes`] were of soft masks and
    /// `/Mask` images, for diagnostics and tests.
    pub fn decoded_mask_decodes(&self) -> usize {
        self.decoded_cache.mask_decodes()
    }

    /// PdfCraft patch (48): how many `/ICCBased` profiles were read through this cache, for
    /// diagnostics and tests. A profile used again, by the same stream object or by a copy with
    /// the same dictionary and bytes, is read once.
    pub fn icc_profile_reads(&self) -> usize {
        self.object_cache.icc_profile_reads()
    }

    /// PdfCraft patch (80): how many `/Indexed` lookup tables were read through this cache, for
    /// diagnostics and tests. A colour space named by a resource is built, and its table read,
    /// once however many images or colours use it.
    pub fn indexed_lookup_reads(&self) -> usize {
        self.object_cache.indexed_lookup_reads()
    }
}

/// A per-page interpretation context that borrows shared data from an [`InterpreterCache`].
pub struct Context<'a> {
    states: Vec<State<'a>>,
    path: BezPath,
    sub_path_start: Point,
    last_point: Point,
    clip: Option<FillRule>,
    root_transforms: Vec<Affine>,
    bbox: Vec<Rect>,
    pub(crate) settings: InterpreterSettings,
    pub(crate) interpreter_cache: InterpreterCache<'a>,
    pub(crate) xref: &'a XRef,
    pub(crate) ocg_state: OcgState,
    nesting_depth: u32,
    /// PdfCraft patch: estimated bytes held by saved states, and for each `q` not saved because
    /// a limit was reached, the clip count to return to at its `Q`.
    saved_bytes: usize,
    skipped_saves: Vec<usize>,
    /// PdfCraft patch (68): the `num_states` at which the nested content stream (a form, an
    /// appearance, a glyph or pattern cell) being interpreted started, or 0 in page content. A
    /// `Q` there has no `q` of its own stream to match (see `stream_floor_reached`).
    stream_floor: usize,
    // PdfCraft patch (63).
    /// This is the content of a luminosity soft mask whose group colour space is `/DeviceCMYK`,
    /// so a `/DeviceCMYK` fill or stroke colour paints as its gray (ISO 32000-2 §11.5.3,
    /// §10.4.2.3) rather than through the CMYK profile used for display.
    pub(crate) luminosity_in_device_cmyk: bool,
    // PdfCraft patch (#7380).
    /// The Type 3 glyph procedures this content is drawn inside.
    glyph_procs: GlyphProcs,
}

/// PdfCraft patch: most graphics states saved at once. Every `q` copies the current state, dash
/// array and clip list included, so without a limit short operators multiply one large state.
const MAX_SAVED_STATES: usize = 4096;
/// PdfCraft patch: most bytes all saved states may hold together.
const MAX_SAVED_STATE_BYTES: usize = 64 << 20;

/// Roughly what a saved copy of `state` holds.
fn state_bytes(state: &State<'_>) -> usize {
    size_of::<State<'_>>()
        .saturating_add(
            state
                .graphics_state
                .stroke_props
                .dash_array
                .len()
                .saturating_mul(size_of::<f32>()),
        )
        .saturating_add(state.clips.len().saturating_mul(size_of::<ClipType>()))
        .saturating_add(state.text_state.clip_paths.elements().len().saturating_mul(size_of::<PathEl>()))
}

impl<'a> Context<'a> {
    /// Create a new context.
    pub fn new(
        initial_transform: Affine,
        bbox: Rect,
        cache: &InterpreterCache<'a>,
        xref: &'a XRef,
        settings: InterpreterSettings,
    ) -> Self {
        let state = State::new(initial_transform);
        // PdfCraft patch: a page (or other top-level content) starts with fresh budgets.
        NESTED_PAINTS.with(|n| n.set(0));
        CONTENT_LEFT.with(|c| c.set(MAX_PAGE_CONTENT));
        CONTENT_TRUNCATED.with(|truncated| truncated.set(false));
        // PdfCraft patch (89): no form of another page is being drawn.
        FORMS_DRAWING.with(|forms| forms.borrow_mut().clear());
        // PdfCraft patch (73): and fresh JBIG2 decoding, which every image of it shares.
        hayro_syntax::set_page_jbig2_work(settings.jbig2_work);
        // PdfCraft patch (55): a fallback left over from another page is not this page's.
        crate::color::take_color_fallback();
        crate::encode::reset_mesh_budget();

        Self::new_with(initial_transform, bbox, cache, xref, settings, state, 0)
    }

    pub(crate) fn new_with(
        initial_transform: Affine,
        bbox: Rect,
        cache: &InterpreterCache<'a>,
        xref: &'a XRef,
        settings: InterpreterSettings,
        state: State<'a>,
        nesting_depth: u32,
    ) -> Self {
        let mut ocg_state = {
            let root_ref = xref.root_id();
            xref.get::<Dict<'_>>(root_ref)
                .map(|catalog| OcgState::from_catalog(&catalog, xref, settings.view_auto_state))
                .unwrap_or_default()
        };
        // PdfCraft patch: apply viewer layer toggles.
        for (num, generation, visible) in settings.ocg_overrides.iter() {
            ocg_state.set_active(hayro_syntax::object::ObjectIdentifier::new(*num, *generation), *visible);
        }

        Self {
            states: vec![state],
            settings,
            xref,
            root_transforms: vec![initial_transform],
            last_point: Point::default(),
            sub_path_start: Point::default(),
            clip: None,
            bbox: vec![bbox],
            path: BezPath::new(),
            interpreter_cache: cache.clone(),
            ocg_state,
            nesting_depth,
            saved_bytes: 0,
            skipped_saves: Vec::new(),
            stream_floor: 0,
            luminosity_in_device_cmyk: false,
            glyph_procs: GlyphProcs::default(),
        }
    }

    /// PdfCraft patch (89): begins drawing the form `id`; `false` when that form is already
    /// being drawn, so drawing it again would never end (ISO 32000-2 §8.10.1: a form is a
    /// self-contained description; one that contains itself describes nothing finite). The
    /// inner drawing is skipped; that is not reported as `InterpreterWarning::ContentTruncated`,
    /// because the page was not cut short by a size budget: a cycle was skipped as the
    /// specification requires. A soft mask (or a knockout mask built from one) may legally
    /// paint a form already on the stack; reporting that skip as truncation made VectorCraft
    /// reject a 200-shape knockout-mask PDF that otherwise imported completely
    /// (storytold/craft-libs#778). A form without an object number can't be told apart and is
    /// only bounded by the nesting limits.
    ///
    /// The forms being drawn are those of this page's context and of every context painting
    /// inside it (a tiling pattern, a Type 3 glyph, a soft mask or a group), so a form that
    /// reaches itself through one of those is skipped too, while a form drawn again after it
    /// has finished draws as usual.
    pub(crate) fn enter_form(
        &mut self,
        id: Option<hayro_syntax::object::ObjectIdentifier>,
    ) -> bool {
        let Some(id) = id else {
            return true;
        };
        // The stack is never borrowed across a call, so the borrow succeeds; if it didn't,
        // the form draws, bounded by the nesting limits as before this patch.
        let entered = FORMS_DRAWING.with(|forms| {
            let Ok(mut forms) = forms.try_borrow_mut() else {
                return None;
            };
            if forms.contains(&id) {
                return Some(false);
            }
            forms.push(id);
            Some(true)
        });
        if entered == Some(false) {
            warn!("form XObject draws itself; the inner drawing is skipped");
            return false;
        }
        true
    }

    /// PdfCraft patch (89): ends drawing a form begun with `enter_form`.
    pub(crate) fn leave_form(&mut self, id: Option<hayro_syntax::object::ObjectIdentifier>) {
        if let Some(id) = id {
            FORMS_DRAWING.with(|forms| {
                if let Ok(mut forms) = forms.try_borrow_mut()
                    && forms.last() == Some(&id)
                {
                    forms.pop();
                }
            });
        }
    }

    pub(crate) fn save_state(&mut self) {
        let Some(cur) = self.states.last() else {
            warn!("attempted to save state without existing state");
            return;
        };

        // PdfCraft patch: past either limit the `q` is recorded but nothing is copied. Its `Q`
        // still removes the clips added since; other changes made in between are kept.
        let bytes = state_bytes(cur);
        if !self.skipped_saves.is_empty()
            || self.states.len() >= MAX_SAVED_STATES
            || self.saved_bytes.saturating_add(bytes) > MAX_SAVED_STATE_BYTES
        {
            if self.skipped_saves.is_empty() {
                warn!("too many saved graphics states; further saves are not kept");
            }
            self.skipped_saves.push(cur.clips.len());
            return;
        }

        let cur = cur.clone();
        self.saved_bytes = self.saved_bytes.saturating_add(bytes);
        self.states.push(cur);
    }

    pub(crate) fn bbox(&self) -> Rect {
        self.bbox.last().copied().unwrap_or_else(|| {
            warn!("failed to get a bbox");

            Rect::new(0.0, 0.0, 1.0, 1.0)
        })
    }

    fn push_bbox(&mut self, bbox: Rect) {
        let new = self.bbox().intersect(bbox);
        self.bbox.push(new);
    }

    pub(crate) fn push_clip_path(
        &mut self,
        clip_path: BezPath,
        fill: FillRule,
        device: &mut impl Device<'a>,
    ) {
        if let Some(clip_rect) = path_as_rect(&clip_path) {
            let cur_bbox = self.bbox();

            // If the clip path is a rect and completely covers the current bbox, don't emit it.
            if cur_bbox
                .min_x()
                .is_nearly_greater_or_equal(clip_rect.min_x())
                && cur_bbox
                    .min_y()
                    .is_nearly_greater_or_equal(clip_rect.min_y())
                && cur_bbox.max_x().is_nearly_less_or_equal(clip_rect.max_x())
                && cur_bbox.max_y().is_nearly_less_or_equal(clip_rect.max_y())
            {
                self.get_mut().clips.push(ClipType::Dummy);
                return;
            }
        }

        let bbox = clip_path.bounding_box();
        device.push_clip_path(&ClipPath {
            path: clip_path,
            fill,
        });
        self.push_bbox(bbox);
        self.get_mut().clips.push(ClipType::Real);
    }

    pub(crate) fn pop_clip_path(&mut self, device: &mut impl Device<'a>) {
        if let Some(ClipType::Real) = self.get_mut().clips.pop() {
            device.pop_clip_path();
            self.pop_bbox();
        }
    }

    fn pop_bbox(&mut self) {
        self.bbox.pop();
    }

    pub(crate) fn push_root_transform(&mut self) {
        self.root_transforms.push(self.get().ctm);
    }

    pub(crate) fn pop_root_transform(&mut self) {
        self.root_transforms.pop();
    }

    pub(crate) fn root_transform(&self) -> Affine {
        self.root_transforms
            .last()
            .copied()
            .unwrap_or(Affine::IDENTITY)
    }

    pub(crate) fn restore_state(&mut self, device: &mut impl Device<'a>) {
        // PdfCraft patch: a save that was not kept is the most recent one.
        if let Some(target_clips) = self.skipped_saves.pop() {
            while self.get().clips.len() > target_clips {
                self.pop_clip_path(device);
            }
            return;
        }

        let Some(target_clips) = self
            .states
            .get(self.states.len().saturating_sub(2))
            .map(|s| s.clips.len())
        else {
            warn!("underflowed graphics state");
            return;
        };

        while self.get().clips.len() > target_clips {
            self.pop_clip_path(device);
        }

        // The first state should never be popped.
        if self.states.len() > 1 {
            self.states.pop();
            // PdfCraft patch: the state now on top is the copy that was charged when saved.
            if let Some(top) = self.states.last() {
                self.saved_bytes = self.saved_bytes.saturating_sub(state_bytes(top));
            }
        }

        device.set_soft_mask(
            self.states
                .last()
                .and_then(|l| l.graphics_state.soft_mask.clone()),
        );
    }

    pub(crate) fn path(&self) -> &BezPath {
        &self.path
    }

    pub(crate) fn path_mut(&mut self) -> &mut BezPath {
        &mut self.path
    }

    pub(crate) fn sub_path_start(&self) -> &Point {
        &self.sub_path_start
    }

    pub(crate) fn sub_path_start_mut(&mut self) -> &mut Point {
        &mut self.sub_path_start
    }

    pub(crate) fn last_point(&self) -> &Point {
        &self.last_point
    }

    pub(crate) fn last_point_mut(&mut self) -> &mut Point {
        &mut self.last_point
    }

    pub(crate) fn clip_mut(&mut self) -> &mut Option<FillRule> {
        &mut self.clip
    }

    pub(crate) fn get(&self) -> &State<'a> {
        self.states.last().unwrap()
    }

    pub(crate) fn get_mut(&mut self) -> &mut State<'a> {
        self.states.last_mut().unwrap()
    }

    pub(crate) fn pre_concat_transform(&mut self, transform: Transform) {
        self.pre_concat_affine(convert_transform(transform));
    }

    pub(crate) fn pre_concat_affine(&mut self, transform: Affine) {
        self.get_mut().ctm *= transform;
    }

    /// PdfCraft patch (80): an `/Indexed` resource with no lookup table is built, marked
    /// tableless, for the image and colour operators to handle (see `Indexed::new`).
    pub(crate) fn get_color_space(
        &mut self,
        resources: &Resources<'_>,
        name: &Name<'_>,
    ) -> Option<ColorSpace> {
        let cs_object = resources.get_color_space(name)?;
        self.interpreter_cache
            .object_cache
            .get_or_insert_with(cs_object.cache_key(), || {
                ColorSpace::new_allowing_tableless(
                    cs_object.clone(),
                    &self.interpreter_cache.object_cache,
                )
            })
    }

    pub(crate) fn stroke_props(&self) -> StrokeProps {
        self.get().graphics_state.stroke_props.clone()
    }

    /// PdfCraft patch (68): starts a nested content stream at the current depth and returns the
    /// floor of the stream it runs inside, for `leave_stream`.
    pub(crate) fn enter_stream(&mut self) -> usize {
        let depth = self.num_states();
        std::mem::replace(&mut self.stream_floor, depth)
    }

    /// PdfCraft patch (68): ends a content stream begun with `enter_stream`.
    pub(crate) fn leave_stream(&mut self, outer_floor: usize) {
        self.stream_floor = outer_floor;
    }

    /// PdfCraft patch (68): every `q` of the current content stream has been matched, so a `Q`
    /// now would restore a state its caller saved (a stream balances its own `q` and `Q`,
    /// ISO 32000-2 §8.4.2, §8.10.1).
    pub(crate) fn stream_floor_reached(&self) -> bool {
        self.num_states() <= self.stream_floor
    }

    pub(crate) fn num_states(&self) -> usize {
        // PdfCraft patch: saves not kept still count, so callers restore back past them.
        self.states.len().saturating_add(self.skipped_saves.len())
    }

    pub(crate) fn nesting_depth(&self) -> u32 {
        self.nesting_depth
    }

    // PdfCraft patch (#7380).
    /// The Type 3 glyph procedures this content is drawn inside.
    pub(crate) fn glyph_procs(&self) -> &GlyphProcs {
        &self.glyph_procs
    }

    // PdfCraft patch (#7380).
    /// This content is drawn inside the glyph procedures `procs`.
    pub(crate) fn set_glyph_procs(&mut self, procs: GlyphProcs) {
        self.glyph_procs = procs;
    }

    pub(crate) fn begin_nested_interpretation(&mut self) -> bool {
        if self.nesting_depth >= MAX_NESTED_INTERPRETATION_DEPTH {
            warn!("interpreter nesting depth exceeded");

            return false;
        }
        if !take_nested_paint(self.nesting_depth + 1) {
            warn!("nested paint budget exceeded");

            return false;
        }

        self.nesting_depth += 1;

        true
    }

    pub(crate) fn end_nested_interpretation(&mut self) {
        self.nesting_depth = self.nesting_depth.saturating_sub(1);
    }
    pub(crate) fn resolve_font(&mut self, font_dict: &Dict<'a>) -> Option<TextStateFont<'a>> {
        let cache_key = font_dict.cache_key();

        let resolved = {
            let mut font_cache = self.interpreter_cache.font_cache.borrow_mut();
            font_cache
                .entry(cache_key)
                .or_insert_with(|| {
                    Font::new(
                        font_dict,
                        &self.settings.font_resolver,
                        &self.settings.cmap_resolver,
                    )
                })
                .clone()
        };

        // PdfCraft patch (90): tell the caller a Type 3 font lacks the /Widths it requires,
        // so the spacing taken from its glyph procedures can be reported.
        if resolved.as_ref().is_some_and(Font::type3_widths_missing) {
            (self.settings.extended_warning_sink)(crate::ExtendedWarning::Type3WidthsMissing);
        }

        if let Some(resolved) = resolved {
            Some(TextStateFont::Font(resolved))
        } else {
            Font::new_standard(StandardFont::Helvetica, &self.settings.font_resolver)
                .map(TextStateFont::Fallback)
        }
    }
}

pub(crate) fn path_as_rect(path: &BezPath) -> Option<Rect> {
    // One MoveTo, three LineTo, one ClosePath
    if path.elements().len() != 5 {
        return None;
    }

    // PdfCraft patch: the points have to trace the rectangle, not merely reach its corners. The
    // length check above admits one MoveTo and four LineTo as well as the intended one MoveTo,
    // three LineTo and one ClosePath, and asking only that each corner of the bounding box was
    // visited says nothing about the order or the steps between: a five-point polyline whose
    // ends reached every corner of its own bounding box was painted as that box, and a clip of
    // that shape was discarded. Four distinct points, each step along one axis, the last point
    // back at the first: that is a rectangle and nothing else is.
    let mut points: Vec<Point> = Vec::with_capacity(5);
    let mut closed = false;
    for (idx, el) in path.elements().iter().enumerate() {
        match el {
            PathEl::MoveTo(p) => {
                if idx != 0 {
                    return None;
                }
                points.push(*p);
            }
            PathEl::LineTo(p) => points.push(*p),
            PathEl::QuadTo(_, _) => return None,
            PathEl::CurveTo(_, _, _) => return None,
            // A subpath that ends and starts again is more than one shape.
            PathEl::ClosePath => {
                if idx + 1 != path.elements().len() {
                    return None;
                }
                closed = true;
            }
        }
    }

    // Exactly equal, not nearly: `is_nearly_equal` allows a 256th of a path unit, and the matrix
    // is applied after this, so a `cm` of 10,000 turns that into tens of device units -- a path
    // that only nearly closes would be painted as the rectangle, with a side it does not have.
    // A rectangle's coordinates are equal in the content stream, not approximately equal, so
    // nothing that is one loses the fast path by this; a path that misses takes the general route
    // and is painted as written (#3327 review).
    let same = |a: Point, b: Point| a.x == b.x && a.y == b.y;
    // `m l l l l`: the fourth line goes back to the start, which `m l l l h` does with ClosePath.
    if !closed {
        if points.len() != 5 || !same(points[4], points[0]) {
            return None;
        }
        points.pop();
    }
    if points.len() != 4 {
        return None;
    }
    // Distinct, so a degenerate path is not one, and each step along one axis only: the four
    // points of an axis-aligned rectangle, in order round it.
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % 4];
        let along_x = p.y == q.y && p.x != q.x;
        let along_y = p.x == q.x && p.y != q.y;
        if !(along_x || along_y) {
            return None;
        }
        if points.iter().skip(i + 1).any(|o| same(*p, *o)) {
            return None;
        }
    }

    Some(path.fast_bounding_box())
}

/// PdfCraft patch tests (ported from craft-libs): the graphics-state stack is bounded by count
/// and by bytes, and every `q` still pairs with its `Q`.
#[cfg(test)]
mod saved_state_tests {
    use super::*;
    use crate::device::DummyDevice;
    use hayro_syntax::Pdf;

    fn pdf() -> Pdf {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Count 0 /Kids [] >>",
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 3\n0000000000 65535 f \n");
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        Pdf::new(out).expect("synthetic pdf")
    }

    fn triangle() -> BezPath {
        let mut path = BezPath::new();
        path.move_to((10.0, 10.0));
        path.line_to((90.0, 10.0));
        path.line_to((50.0, 90.0));
        path.close_path();
        path
    }

    /// Saves `saves` more times, checks a clip added after the last save goes at its `Q`, and unwinds.
    fn save_and_unwind(ctx: &mut Context<'_>, saves: usize) {
        let before = ctx.num_states();
        for _ in 0..saves {
            ctx.save_state();
        }
        assert!(ctx.states.len() <= MAX_SAVED_STATES);
        assert!(ctx.saved_bytes <= MAX_SAVED_STATE_BYTES);
        assert_eq!(ctx.num_states(), before + saves, "every q still counts");

        let mut device = DummyDevice;
        let clips = ctx.get().clips.len();
        ctx.push_clip_path(triangle(), FillRule::NonZero, &mut device);
        assert_eq!(ctx.get().clips.len(), clips + 1);
        ctx.restore_state(&mut device);
        assert_eq!(ctx.get().clips.len(), clips);

        while ctx.num_states() > 1 {
            ctx.restore_state(&mut device);
        }
        assert_eq!(ctx.states.len(), 1);
        assert!(ctx.skipped_saves.is_empty());
        assert_eq!(ctx.saved_bytes, 0);
    }

    #[test]
    fn repeated_saves_stay_bounded_and_balanced() {
        let pdf = pdf();
        let cache = InterpreterCache::new();
        let bbox = Rect::new(0.0, 0.0, 100.0, 100.0);
        let settings = InterpreterSettings::default();
        let mut ctx = Context::new(Affine::IDENTITY, bbox, &cache, pdf.xref(), settings);

        // A small state reaches the count limit.
        save_and_unwind(&mut ctx, 20_000);

        // A large one (a 64 Ki-entry dash array, 256 KiB a copy) reaches the byte limit first.
        ctx.get_mut().graphics_state.stroke_props.dash_array = (0..1 << 16).map(|_| 2.0).collect();
        ctx.save_state();
        assert_eq!(ctx.states.len(), 2);
        while ctx.skipped_saves.is_empty() {
            ctx.save_state();
        }
        assert!(ctx.states.len() < MAX_SAVED_STATES);
        save_and_unwind(&mut ctx, 1000);

        // Once unwound, a kept save restores what changed after it.
        let mut device = DummyDevice;
        ctx.save_state();
        ctx.pre_concat_affine(Affine::scale(2.0));
        ctx.restore_state(&mut device);
        assert_eq!(ctx.get().ctm, Affine::IDENTITY);
    }
}

/// A form that paints itself is skipped so interpretation ends; that skip is not
/// reported as [`crate::interpret::InterpreterWarning::ContentTruncated`] (craft-libs#778).
#[cfg(test)]
mod form_cycle_tests {
    use super::*;
    use crate::device::DummyDevice;
    use crate::interpret::{InterpreterSettings, InterpreterWarning, interpret_page};
    use hayro_syntax::Pdf;
    use std::sync::{Arc, Mutex};

    fn pdf_from_objects(bodies: &[String]) -> Pdf {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
        Pdf::new(out).expect("synthetic pdf")
    }

    fn stream(dict: &str, data: &str) -> String {
        format!("<< {dict} /Length {} >>\nstream\n{data}endstream", data.len())
    }

    fn warnings_from(pdf: &Pdf) -> Vec<InterpreterWarning> {
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let settings = InterpreterSettings {
            warning_sink: {
                let warnings = warnings.clone();
                Arc::new(move |warning| warnings.lock().expect("warnings").push(warning))
            },
            ..InterpreterSettings::default()
        };
        let cache = InterpreterCache::new();
        let mut context = Context::new(Affine::IDENTITY, Rect::new(0.0, 0.0, 100.0, 100.0), &cache, pdf.xref(), settings);
        interpret_page(&pdf.pages()[0], &mut context, &mut DummyDevice);
        warnings.lock().expect("warnings").clone()
    }

    fn is_truncated(warnings: &[InterpreterWarning]) -> bool {
        warnings.iter().any(|w| matches!(w, InterpreterWarning::ContentTruncated))
    }

    /// A form whose content draws itself: the recursive `Do` is skipped, the fill still
    /// paints, and the page is not reported as truncated.
    #[test]
    fn a_form_that_draws_itself_is_skipped_and_is_not_content_truncated() {
        let form = stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /F 5 0 R >> >>", "0 0 100 100 re f\n/F Do\n");
        let contents = stream("", "/F Do\n");
        let pdf = pdf_from_objects(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /F 5 0 R >> >> >>".into(),
            contents,
            form,
        ]);
        let warnings = warnings_from(&pdf);
        assert!(!content_was_truncated());
        assert!(!is_truncated(&warnings), "{warnings:?}");
    }

    /// Form A draws form B, which draws form A: the cycle is skipped the same way, and
    /// is not reported as a content-size truncation.
    #[test]
    fn a_pair_of_forms_that_draw_each_other_is_not_content_truncated() {
        let form_a = stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /B 6 0 R >> >>", "0 0 50 50 re f\n/B Do\n");
        let form_b =
            stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /A 5 0 R >> >>", "50 50 50 50 re f\n/A Do\n");
        let contents = stream("", "/A Do\n");
        let pdf = pdf_from_objects(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /A 5 0 R >> >> >>".into(),
            contents,
            form_a,
            form_b,
        ]);
        let warnings = warnings_from(&pdf);
        assert!(!content_was_truncated());
        assert!(!is_truncated(&warnings), "{warnings:?}");
    }
}

// PdfCraft patch (#7380) tests.
/// A Type 3 glyph procedure that shows its own glyph is drawn once, directly or through a form,
/// soft mask, tiling pattern or CMYK group it sets; its refused drawings of itself don't spend
/// the page's nested-paint budget or report the page truncated; and a soft mask set outside a
/// glyph draws that glyph wherever the device interprets the mask.
#[cfg(test)]
mod glyph_proc_cycle_tests {
    use super::*;
    use crate::cmyk_group::CmykGroup;
    use crate::font::Glyph;
    use crate::interpret::{InterpreterSettings, InterpreterWarning, interpret_page};
    use crate::pattern::Pattern;
    use crate::{BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask};
    use hayro_syntax::Pdf;
    use kurbo::Shape;
    use std::sync::{Arc, Mutex};

    fn stream(entries: &str, data: &str) -> String {
        format!("<< /Length {} {entries} >>\nstream\n{data}\nendstream", data.len() + 1)
    }

    /// A page drawing `content` with a Type 3 font `/T3` whose glyphs `a`, `b` and `c` are drawn
    /// by `procs`, in that order, and the page resources `resources` naming `objects`, which are
    /// numbered from `6 + procs.len()`. The font has no resources of its own, so a procedure
    /// inherits the page's and may show the font's glyphs. (A font naming itself in its own
    /// `/Resources` is not read: the reader refuses that reference as a cycle.)
    fn pdf(content: &str, procs: &[&str], resources: &str, objects: &[String]) -> Pdf {
        let names = ["a", "b", "c"];
        let char_procs: String = procs.iter().enumerate().map(|(i, _)| format!("/{} {} 0 R ", names[i], 6 + i)).collect();
        let mut bodies = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_string(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /Font << /T3 5 0 R >> {resources} >> >>"),
            stream("", content),
            format!(
                "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 100 10] /FontMatrix [1 0 0 1 0 0] /CharProcs << {char_procs}>> /Encoding << /Type /Encoding /Differences [97 /a /b /c] >> /FirstChar 97 /LastChar 99 /Widths [10 10 10] >>"
            ),
        ];
        bodies.extend(procs.iter().map(|proc| stream("", proc)));
        bodies.extend_from_slice(objects);
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
        Pdf::new(out).expect("synthetic pdf")
    }

    /// What a page drew. Like a renderer, the device draws each Type 3 glyph, tiling pattern
    /// and CMYK group by interpreting it into itself when it is given one, and it interprets
    /// each soft mask it is given at that time, apart.
    #[derive(Default)]
    struct Drawn {
        /// The left edge of each fill, in the order drawn.
        fills: Vec<f64>,
        /// For each time a soft mask was interpreted, the fills it drew.
        mask_fills: Vec<usize>,
        cmyk_groups: usize,
    }

    impl<'a> Device<'a> for Drawn {
        fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
            if let Some(mask) = mask {
                let mut drawn = Self::default();
                mask.interpret(&mut drawn);
                self.mask_fills.push(drawn.fills.len());
                self.mask_fills.extend(drawn.mask_fills);
            }
        }
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, path: &BezPath, transform: Affine, paint: &Paint<'a>, _: &PathDrawMode) {
            self.fills.push(transform.transform_rect_bbox(path.bounding_box()).x0);
            if let Paint::Pattern(pattern) = paint
                && let Pattern::Tiling(tiling) = &**pattern
            {
                tiling.interpret(self, transform, false);
            }
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
        fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, glyph_transform: Affine, paint: &Paint<'a>, _: &GlyphDrawMode) {
            if let Glyph::Type3(glyph) = glyph {
                glyph.interpret(self, transform, glyph_transform, paint);
            }
        }
        fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
        fn draw_isolated_cmyk_group(&mut self, group: &CmykGroup<'a, '_>) -> bool {
            self.cmyk_groups += 1;
            group.interpret(self, (0.0, 0.0));
            true
        }
    }

    /// What the page drew, and whether it was reported truncated.
    fn draw(pdf: &Pdf) -> (Drawn, bool) {
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let settings = InterpreterSettings {
            warning_sink: {
                let warnings = warnings.clone();
                Arc::new(move |warning| warnings.lock().expect("warnings").push(warning))
            },
            ..InterpreterSettings::default()
        };
        let cache = InterpreterCache::new();
        let mut context = Context::new(Affine::IDENTITY, Rect::new(0.0, 0.0, 100.0, 100.0), &cache, pdf.xref(), settings);
        let mut drawn = Drawn::default();
        interpret_page(&pdf.pages()[0], &mut context, &mut drawn);
        let warned = warnings.lock().expect("warnings").iter().any(|w| matches!(w, InterpreterWarning::ContentTruncated));
        (drawn, warned || content_was_truncated())
    }

    /// Each fill is to the right of the one before it.
    fn left_to_right(fills: &[f64]) -> bool {
        fills.windows(2).all(|pair| pair[0] < pair[1])
    }

    /// Glyph `a` fills a square and then shows `a` again to its right: only the outer `a`
    /// draws. A glyph that shows another glyph of its own font draws both.
    #[test]
    fn a_type3_glyph_that_shows_itself_is_drawn_once() {
        let (drawn, truncated) = draw(&pdf("BT /T3 1 Tf (a) Tj ET", &["0 0 d0 0 0 10 10 re f BT /T3 1 Tf 20 0 Td (a) Tj ET"], "", &[]));
        assert_eq!(drawn.fills.len(), 1, "{:?}", drawn.fills);
        assert!(!truncated);

        let procs = ["0 0 d0 0 0 10 10 re f BT /T3 1 Tf 20 0 Td (b) Tj ET", "0 0 d0 0 0 10 10 re f"];
        let (drawn, truncated) = draw(&pdf("BT /T3 1 Tf (a) Tj ET", &procs, "", &[]));
        assert_eq!(drawn.fills.len(), 2, "{:?}", drawn.fills);
        assert!(left_to_right(&drawn.fills), "{:?}", drawn.fills);
        assert!(!truncated);
    }

    /// Glyph `a` shows `b`, whose procedure shows `b` 50,000 times, and then `c`. The refused
    /// drawings of `b` inside itself don't spend the page's nested-paint budget, so `c` draws.
    #[test]
    fn a_glyph_showing_itself_does_not_spend_the_budget_of_the_glyphs_after_it() {
        let b = format!("0 0 d0 0 0 10 10 re f BT /T3 1 Tf 20 0 Td ({}) Tj ET", "b".repeat(50_000));
        let procs = ["0 0 d0 0 0 10 10 re f BT /T3 1 Tf 20 0 Td (b) Tj ET BT /T3 1 Tf 40 0 Td (c) Tj ET", &b, "0 0 d0 0 0 10 10 re f"];
        let (drawn, truncated) = draw(&pdf("BT /T3 1 Tf (a) Tj ET", &procs, "", &[]));
        assert_eq!(drawn.fills.len(), 3, "a, b once and c: {:?}", drawn.fills);
        assert!(left_to_right(&drawn.fills), "{:?}", drawn.fills);
        assert!(!truncated);
    }

    /// Glyph `a` fills a square and sets something whose content shows `a`: a form it draws, a
    /// soft mask, a tiling pattern it fills with, or a CMYK group. That content is inside `a`,
    /// so its `a` is refused and only the outer square is drawn.
    #[test]
    fn a_glyph_reaching_itself_through_what_its_procedure_sets_is_drawn_once() {
        let shows_a = "BT /T3 1 Tf 20 0 Td (a) Tj ET";
        let form = |group: &str| stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 100 10] {group}"), shows_a);
        let cases = [
            ("a form", "0 0 d0 0 0 10 10 re f /F Do", "/XObject << /F 7 0 R >>", vec![form("")]),
            (
                "a soft mask",
                "0 0 d0 /G gs 0 0 10 10 re f",
                "/ExtGState << /G 7 0 R >>",
                vec![
                    "<< /Type /ExtGState /SMask << /Type /Mask /S /Luminosity /G 8 0 R >> >>".into(),
                    form("/Group << /S /Transparency /CS /DeviceGray >>"),
                ],
            ),
            (
                "a tiling pattern",
                "0 0 d0 /Pattern cs /P scn 0 0 10 10 re f",
                "/Pattern << /P 7 0 R >>",
                vec![stream(
                    "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 100 10] /XStep 100 /YStep 10 /Resources << /Font << /T3 5 0 R >> >>",
                    shows_a,
                )],
            ),
            (
                "a CMYK group",
                "0 0 d0 0 0 10 10 re f /K Do",
                "/XObject << /K 7 0 R >>",
                vec![form("/Group << /S /Transparency /I true /CS /DeviceCMYK >>")],
            ),
        ];
        for (what, a, resources, objects) in cases {
            let (drawn, truncated) = draw(&pdf("BT /T3 1 Tf (a) Tj ET", &[a], resources, &objects));
            assert_eq!(drawn.fills.len(), 1, "{what}: {:?}", drawn.fills);
            assert!(drawn.mask_fills.iter().all(|&fills| fills == 0), "{what}: {:?}", drawn.mask_fills);
            assert!(!truncated, "{what}");
            if what == "a CMYK group" {
                assert!(drawn.cmyk_groups > 0, "the device drew the group");
            }
        }
    }

    /// The page sets a soft mask whose group shows glyph `a`, then shows `a`. The device
    /// interprets the mask for each paint, inside `a` too, and every time the mask draws `a`:
    /// the mask is not inside `a`.
    #[test]
    fn a_soft_mask_set_outside_a_glyph_draws_that_glyph() {
        let objects = [
            "<< /Type /ExtGState /SMask << /Type /Mask /S /Luminosity /G 8 0 R >> >>".to_string(),
            stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceGray >>", "BT /T3 1 Tf (a) Tj ET"),
        ];
        let pdf = pdf("/G gs BT /T3 1 Tf (a) Tj ET", &["0 0 d0 0 0 10 10 re f"], "/ExtGState << /G 7 0 R >>", &objects);
        let (drawn, truncated) = draw(&pdf);
        assert!(drawn.mask_fills.len() > 1, "the mask is interpreted inside `a` too: {:?}", drawn.mask_fills);
        assert!(drawn.mask_fills.iter().all(|&fills| fills > 0), "{:?}", drawn.mask_fills);
        assert!(!truncated);
    }
}
