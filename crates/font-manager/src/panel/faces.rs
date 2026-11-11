//! Drawing text in a font face itself: each face shown is a document face of craft-ui's registry, which loads
//! them a few per pass, keeps the ones on screen and wakes the UI while one loads.

use craft_i18n::{Direction, base_direction};
use craft_ui::fonts::{FaceBytes, FontRef};
use craft_ui::primitives::{ContentPainterCx, DocRun, DocText, FacesCx};
use craft_ui::style::DataColor;
use egui::{Color32, Pos2, Rect};

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
    let too_large = face.path.as_ref().and_then(|path| std::fs::metadata(path).ok()).is_some_and(|meta| meta.len() > MAX_FILE_BYTES);
    if too_large {
        return None;
    }
    let data = catalog.load(face).ok()?;
    Some(FaceBytes::new(data.bytes, data.index).coords(face.variations.iter().map(|(tag, v)| (*tag, *v))))
}

/// The font that draws `face`, once the registry has loaded it. `at` is the context the caller draws or measures in.
pub(crate) fn shown(at: &impl FacesCx, catalog: &Catalog, owner: Owner, face: &FaceInfo) -> Shown {
    at.document_face(&key(owner, face), || load(catalog, face))
}

/// `text` on one line in `font` at `points`, at the reading start of `rect` and centred between its top and bottom,
/// clipped to `rect`. The sample keeps its own direction whatever the interface's.
pub(crate) fn paint_sample(painter: &ContentPainterCx<'_>, rect: Rect, font: &FontRef, text: &str, points: f32, color: DataColor) {
    let runs = [DocRun::new(text, font.clone(), points, color)];
    let layout = painter.layout(&DocText::new(&runs));
    let size = layout.size();
    let x = match base_direction(text) {
        Some(Direction::Rtl) => rect.right() - size.x,
        _ => rect.left(),
    };
    let at = Pos2::new(x, rect.center().y - size.y / 2.0);
    let clip = rect.intersect(painter.painter.clip_rect());
    painter.painter.with_clip_rect(clip).galley(at, layout.galley, Color32::PLACEHOLDER);
}
