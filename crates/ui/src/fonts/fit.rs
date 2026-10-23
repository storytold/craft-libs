//! Row metrics for the UI language's script face.
//!
//! An app's theme sets `interact_size` for rows of its own face. When the UI language's script face
//! leads the stack and has taller rows, widgets need that much more height; the theme's own height
//! and padding stay as they are. CJK faces that follow the app's face are fitted into its rows
//! instead ([`into_row`]).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use egui::epaint::text::{Fonts, TextOptions};
use egui::{FontData, FontDefinitions, FontFamily, FontId, Style, TextStyle, Theme};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider as _};

const BASE: &str = "craft-ui base face";
const SCRIPT: &str = "craft-ui script face";

/// The app's face and the UI language's script face, each as its own family.
#[derive(Clone)]
pub(super) struct RowFaces(FontDefinitions);

impl RowFaces {
    pub(super) fn new(base: Arc<FontData>, script: Arc<FontData>) -> Self {
        let mut definitions = FontDefinitions::empty();
        for (name, data) in [(BASE, base), (SCRIPT, script)] {
            definitions.font_data.insert(name.to_owned(), data);
            definitions.families.insert(FontFamily::Name(name.into()), vec![name.to_owned()]);
        }
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            definitions.families.entry(family).or_default();
        }
        Self(definitions)
    }
}

/// How much taller a row of the script face is than a row of the app's face, at `style`'s body and
/// button sizes; `0` when it isn't taller.
pub(super) fn extra_height(faces: Option<&RowFaces>, style: &Style) -> f32 {
    let Some(RowFaces(definitions)) = faces else { return 0.0 };
    let mut fonts = Fonts::new(TextOptions::default(), definitions.clone());
    let mut view = fonts.with_pixels_per_point(1.0);
    let mut row = |size: f32, face: &str| view.row_height(&FontId::new(size, FontFamily::Name(face.into())));
    let extra = text_sizes(style).into_iter().map(|size| row(size, SCRIPT) - row(size, BASE)).fold(0.0_f32, f32::max);
    if extra.is_finite() { extra } else { 0.0 }
}

fn text_sizes(style: &Style) -> Vec<f32> {
    [TextStyle::Body, TextStyle::Button].iter().filter_map(|s| style.text_styles.get(s)).map(|font| font.size).collect()
}

/// Kana and full-width brackets: the characters a CJK face fitted into a Latin row keeps inside it.
/// The brackets reach as high and as deep as the face's ideographs and Hangul. Only characters
/// every cut face keeps ([`craft_fonts_build::KEPT_IN_EVERY_CUT`]), so the fit doesn't depend on
/// what an app's catalogs use.
pub(super) const ROW_SAMPLES: &str = "あぎアギ「」『』（）【】";

/// Fit `face`, a CJK fallback after `primary`, into `primary`'s rows: its glyphs sit on
/// `primary`'s baseline and the row keeps `primary`'s height. The face is scaled down only as far
/// as keeps [`ROW_SAMPLES`] inside the row.
pub(super) fn into_row(primary: &FontData, face: &mut FontData) {
    let (Some(p), Some(f)) = (VerticalMetrics::of(primary), VerticalMetrics::of(face)) else { return };
    let ascent = primary.tweak.scale * p.ascent;
    let height = primary.tweak.scale * p.height();
    let (above, below) = ink_extent(face, ROW_SAMPLES);
    let mut scale = 1.0_f32;
    if above > ascent {
        scale = scale.min(ascent / above);
    }
    if below > height - ascent {
        scale = scale.min((height - ascent) / below);
    }
    #[allow(unused_imports, reason = "unused when egui has the inherent method")]
    use super::SetBaselineFallback as _;
    // egui centres a fallback's row in the primary's row without growing it; the offset moves the
    // centred baseline onto the primary's. Both are in units of the font size times the scale.
    face.tweak.set_baseline_fallback(false);
    face.tweak.scale = scale;
    let centred = 0.5 * (height - scale * f.height()) + scale * f.ascent;
    face.tweak.y_offset_factor = (ascent - centred + primary.tweak.scale * primary.tweak.y_offset_factor) / scale;
    face.tweak.y_offset = primary.tweak.y_offset;
}

/// A face's ascent, descent and line gap as epaint reads them, in ems.
struct VerticalMetrics {
    ascent: f32,
    descent: f32,
    line_gap: f32,
}

impl VerticalMetrics {
    fn of(data: &FontData) -> Option<Self> {
        let font = FontRef::from_index(data.bytes(), data.index).ok()?;
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let em = f32::from(metrics.units_per_em);
        (em > 0.0).then(|| Self { ascent: metrics.ascent / em, descent: -metrics.descent / em, line_gap: metrics.leading / em })
    }

    fn height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }
}

/// How far the outlines of `text`'s characters in `data` reach above and below the baseline, in
/// ems, from their control points.
pub(super) fn ink_extent(data: &FontData, text: &str) -> (f32, f32) {
    let Ok(font) = FontRef::from_index(data.bytes(), data.index) else { return (0.0, 0.0) };
    let em = f32::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
    let (charmap, outlines) = (font.charmap(), font.outline_glyphs());
    let mut pen = Extent::default();
    for glyph in text.chars().filter_map(|c| charmap.map(c)).filter_map(|id| outlines.get(id)) {
        let _ = glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen);
    }
    if em > 0.0 { (pen.top / em, -pen.bottom / em) } else { (0.0, 0.0) }
}

#[derive(Default)]
struct Extent {
    top: f32,
    bottom: f32,
}

impl Extent {
    fn add(&mut self, y: f32) {
        self.top = self.top.max(y);
        self.bottom = self.bottom.min(y);
    }
}

impl OutlinePen for Extent {
    fn move_to(&mut self, _x: f32, y: f32) {
        self.add(y);
    }
    fn line_to(&mut self, _x: f32, y: f32) {
        self.add(y);
    }
    fn quad_to(&mut self, _cx0: f32, cy0: f32, _x: f32, y: f32) {
        self.add(cy0);
        self.add(y);
    }
    fn curve_to(&mut self, _cx0: f32, cy0: f32, _cx1: f32, cy1: f32, _x: f32, y: f32) {
        self.add(cy0);
        self.add(cy1);
        self.add(y);
    }
    fn close(&mut self) {}
}

/// A context's fit, kept in its data.
#[derive(Default)]
struct Fitted {
    faces: Option<RowFaces>,
    /// [`extra_height`] by text sizes, for the faces installed.
    measured: Vec<(Vec<f32>, f32)>,
    /// Per theme: the style's own height and the height it was set to.
    applied: [Option<(f32, f32)>; 2],
}

type Shared = Arc<Mutex<Fitted>>;

fn id() -> egui::Id {
    egui::Id::unique("craft_ui::fonts::fitted")
}

fn lock(shared: &Shared) -> MutexGuard<'_, Fitted> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Fit `ctx`'s styles to `faces` now, and again at the start of every pass after the app sets a
/// style with another height or text size.
pub(super) fn install(ctx: &egui::Context, faces: Option<RowFaces>) {
    let shared: Shared = ctx.data_mut(|d| Arc::clone(d.get_temp_mut_or_default::<Shared>(id())));
    {
        let mut fitted = lock(&shared);
        fitted.faces = faces;
        fitted.measured.clear();
    }
    ctx.add_plugin(Refit);
    refit(ctx, &shared);
}

fn refit(ctx: &egui::Context, shared: &Shared) {
    let mut fitted = lock(shared);
    for (i, theme) in [Theme::Dark, Theme::Light].into_iter().enumerate() {
        let style = ctx.style_of(theme);
        let height = style.spacing.interact_size.y;
        // Any other height than the one set last is the app's, e.g. from a theme applied since.
        let base = match fitted.applied[i] {
            Some((base, set)) if set == height => base,
            _ => height,
        };
        let sizes = text_sizes(&style);
        let extra = match fitted.measured.iter().find(|(s, _)| *s == sizes) {
            Some((_, extra)) => *extra,
            None => {
                let extra = extra_height(fitted.faces.as_ref(), &style);
                // Bounded, for an app that changes text sizes often.
                if fitted.measured.len() >= 4 {
                    fitted.measured.remove(0);
                }
                fitted.measured.push((sizes, extra));
                extra
            }
        };
        let target = base + extra;
        if target != height {
            ctx.style_mut_of(theme, |style| style.spacing.interact_size.y = target);
        }
        fitted.applied[i] = Some((base, target));
    }
}

/// Refits a context's styles at the start of each pass.
struct Refit;

impl egui::plugin::Plugin for Refit {
    fn debug_name(&self) -> &'static str {
        "craft_ui::fonts::Refit"
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let shared: Option<Shared> = ctx.data(|d| d.get_temp(id()));
        if let Some(shared) = shared {
            refit(&ctx, &shared);
        }
    }
}
