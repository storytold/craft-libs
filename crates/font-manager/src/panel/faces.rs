//! Drawing text in a font face itself: each face shown is a document face of craft-ui's registry, which loads them
//! in the background, keeps the ones drawn lately and wakes the UI when one lands.

use std::sync::Arc;

use craft_i18n::{Direction, base_direction};
use craft_theme::Srgba;
use craft_ui::fonts::{FaceBytes, FontRef};
use craft_ui::primitives::{ContentPainterCx, DocRun, DocText, FacesCx, TextLayout};
use craft_ui::style::DataColor;
use egui::{Color32, Pos2, Rect};

use crate::panel::subsets::SubsetFont;
use crate::{Catalog, FaceInfo};

/// Larger files are not drawn: a whole CJK family can be hundreds of megabytes.
const MAX_FILE_BYTES: u64 = 48 << 20;

/// Whose catalog a face belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Owner {
    /// The installed fonts.
    Installed,
    /// Files downloaded for a preview.
    Preview,
}

/// A face as the registry has it: ready to draw, still loading, or not drawable.
pub(crate) use craft_ui::fonts::FaceState as Shown;

/// The registry key of `face`: one family per file, index and variation setting.
fn key(owner: Owner, face: &FaceInfo) -> String {
    let file = match &face.path {
        Some(path) => path.display().to_string(),
        None => face.postscript_name.clone().or_else(|| face.full_name.clone()).unwrap_or_else(|| face.family.clone()),
    };
    let coords: String = face.variations.iter().map(|(tag, v)| format!(" {}={v}", String::from_utf8_lossy(tag))).collect();
    format!("face:{owner:?}:{file}#{}{coords}", face.index)
}

/// The bytes of `face` from `catalog`, unless its file is too large to draw with.
fn load(catalog: &Catalog, face: &FaceInfo) -> Option<FaceBytes> {
    let data = catalog.load(face).ok()?;
    Some(FaceBytes::new(data.bytes, data.index).coords(face.variations.iter().map(|(tag, v)| (*tag, *v))))
}

/// The font that draws `face`, once the registry has loaded it in the background. `at` is the context the caller
/// draws or measures in.
pub(crate) fn shown(at: &impl FacesCx, catalog: &Arc<Catalog>, owner: Owner, face: &FaceInfo) -> Shown {
    if face.file_bytes.is_some_and(|bytes| bytes > MAX_FILE_BYTES) {
        return Shown::Unavailable;
    }
    at.document_face(&key(owner, face), || {
        let (catalog, info) = (Arc::clone(catalog), face.clone());
        move || load(&catalog, &info)
    })
}

/// The font that draws a subset of the preview service, once the registry has taken it.
pub(crate) fn subset(at: &impl FacesCx, subset: &SubsetFont) -> Shown {
    at.document_face(&format!("subset:{}", subset.key), || {
        let face = subset.face();
        move || Some(face)
    })
}

/// The bytes of `face` once [`shown`] has it ready, for reading its tables without loading the file again.
pub(crate) fn bytes(at: &impl FacesCx, owner: Owner, face: &FaceInfo) -> Option<FaceBytes> {
    at.document_face_bytes(&key(owner, face))
}

/// The layout of a sample's `runs`: one row, cut with an ellipsis where `room` does not fit it. Measure and paint both
/// lay the sample out with this, so the measured size is the painted size.
pub(crate) fn sample_text<'a>(runs: &'a [DocRun<'a>], room: Option<f32>) -> DocText<'a> {
    DocText { wrap: room, ..DocText::new(runs) }.elide()
}

/// The text the interface face's line is laid out with for a band's baseline: any letter gives the face's line.
const BASELINE_PROBE: &str = "x";

/// The most times a sample is laid out smaller to fit its band. A glyph's ink covers whole device pixels, so a face
/// scaled by the ratio its ink crossed by can still cross by a pixel; the next step settles it.
const FIT_STEPS: usize = 3;

/// A sample laid out for its band: the layout and the baseline it sits on, below the band's top.
pub(crate) struct SampleLine {
    /// The sample at the size it is drawn at.
    pub(crate) layout: TextLayout,
    /// The band's baseline, from its top.
    pub(crate) baseline: f32,
}

/// A sample's text and the face, size and colour it is drawn in.
#[derive(Clone, Copy)]
pub(crate) struct Sample<'a> {
    pub(crate) text: &'a str,
    pub(crate) font: &'a FontRef,
    /// The variation coordinates (axis tag, value) the face is drawn at; empty for the face as registered.
    pub(crate) coords: &'a [([u8; 4], f32)],
    pub(crate) points: f32,
    pub(crate) color: DataColor,
}

impl<'a> Sample<'a> {
    /// `text` in `font` as it is registered, at `points` in `color`.
    pub(crate) fn new(text: &'a str, font: &'a FontRef, points: f32, color: DataColor) -> Self {
        Sample { text, font, coords: &[], points, color }
    }

    /// The sample drawn at the coordinates `coords` of its face.
    pub(crate) fn at(self, coords: &'a [([u8; 4], f32)]) -> Self {
        Sample { coords, ..self }
    }
}

/// `sample` for a band `band` high, laid out by `layout` (a measure's or a painter's) and cut to `room` as
/// [`sample_text`] cuts it. Every face sits on one baseline: that of the interface face's line at `points` centred in
/// the band, as a control's line sets its texts. The sample is laid out at `points`, or, where its ink (what the
/// layout draws, as `Galley::mesh_bounds` holds it) would cross the band's top or bottom from that baseline, at the
/// largest size whose ink stays inside the band.
pub(crate) fn sample_line(layout: impl Fn(&DocText<'_>) -> TextLayout, sample: Sample<'_>, band: f32, room: Option<f32>) -> SampleLine {
    let Sample { text, font, coords, points, color } = sample;
    let probe = [DocRun::new(BASELINE_PROBE, FontRef::proportional(), points, color)];
    let line = layout(&DocText::new(&probe));
    let baseline = (band - line.size().y) / 2.0 + line.first_baseline;
    let at = |size: f32| {
        let runs = [DocRun::new(text, font.clone(), size, color).coords(coords)];
        layout(&sample_text(&runs, room))
    };
    let mut size = points;
    let mut laid = at(size);
    for _ in 0..FIT_STEPS {
        let ink = laid.galley.mesh_bounds;
        if !ink.is_positive() {
            break;
        }
        let above = laid.first_baseline - ink.top();
        let below = ink.bottom() - laid.first_baseline;
        let fit = share(baseline, above).min(share(band - baseline, below));
        if fit >= 1.0 || fit <= 0.0 {
            break;
        }
        size *= fit;
        laid = at(size);
    }
    SampleLine { layout: laid, baseline }
}

/// The share of `ink` that `room` holds, at most all of it.
fn share(room: f32, ink: f32) -> f32 {
    if ink > room { room / ink } else { 1.0 }
}

/// The ink of a sample over `rect`: black on a white paper it fills, in every theme, when `on_white`; else `ink`.
pub(crate) fn sample_ink(painter: &ContentPainterCx<'_>, rect: Rect, on_white: bool, ink: Color32) -> DataColor {
    if !on_white {
        return DataColor::new(ink);
    }
    painter.painter.rect_filled(rect, 0.0, Color32::from(Srgba::WHITE));
    DataColor::new(Color32::from(Srgba::BLACK))
}

/// `sample` on one line in the band `rect` ([`sample_line`]), at the reading start of `rect`, clipped to `rect`. The
/// sample keeps its own direction whatever the interface's.
pub(crate) fn paint_sample(painter: &ContentPainterCx<'_>, rect: Rect, sample: Sample<'_>) {
    let line = sample_line(|doc| painter.layout(doc), sample, rect.height(), Some(rect.width()));
    let size = line.layout.size();
    let x = match base_direction(sample.text) {
        Some(Direction::Rtl) => rect.right() - size.x,
        _ => rect.left(),
    };
    let at = Pos2::new(x, rect.top() + line.baseline - line.layout.first_baseline);
    let clip = rect.intersect(painter.painter.clip_rect());
    painter.painter.with_clip_rect(clip).galley(at, line.layout.galley, Color32::PLACEHOLDER);
}
