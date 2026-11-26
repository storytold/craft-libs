//! One channel row: eye, thumbnail, name, shortcut chord.

use std::fmt::Debug;
use std::hash::Hash;

use craft_behavior::ColumnKey;
use craft_ui::contract::Props as _;
use craft_ui::list::{ListRow, ThumbSize, ToggleCell};
use craft_ui::primitives::textures::{Priority, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails, fit_rect};
use craft_ui::runtime::env::Label;
use egui::{Color32, Rect, vec2};

use super::labels::ChannelsWords;
use super::model::{ChannelKind, ChannelRow};

/// The eye column.
pub(super) const EYE: ColumnKey = ColumnKey::new("eye");
/// The column that holds the thumbnail and the name: the rest of the row after the eye.
pub(super) const NAME: ColumnKey = ColumnKey::new("name");

/// A spot channel's ink swatch at the thumbnail's bottom trailing corner, as a share of the thumbnail's side.
const SPOT_SWATCH_SHARE: f32 = 0.3;

/// The thumbnail cache and the app's source of the channels' pictures.
pub(super) struct ThumbHost<'a, K: Clone + Eq + Hash + Send + Sync + 'static> {
    pub(super) thumbnails: &'a Thumbnails<K>,
    pub(super) source: &'a dyn ThumbSource<K>,
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Clone for ThumbHost<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Copy for ThumbHost<'_, K> {}

/// What the rows share in one pass.
pub(super) struct RowContext<'a, K: Clone + Eq + Hash + Send + Sync + 'static> {
    pub(super) rows: &'a [ChannelRow<K>],
    pub(super) numbers: Vec<Option<u8>>,
    pub(super) thumbs: Option<ThumbHost<'a, K>>,
    pub(super) size: ThumbSize,
    pub(super) words: &'a ChannelsWords,
}

fn named(pattern: &Label, row: &str) -> Label {
    Label::from(craft_i18n::fmt(pattern.as_str(), &[("row", row)]))
}

/// Builds the [`ListRow`] of the row at `at` for the panel's list:
///
/// - The layer form with the eye column (`ToggleCell::eye` named `ChannelsWords::visibility` with the channel name;
///   tooltip `toggle_visibility`, or `mask_overlay` on a layer-mask row).
/// - The thumbnail at the panel's size, painted from the cache (the checkerboard shows through transparent channels);
///   a spot row adds the ink swatch at its corner.
/// - The name, italic for spot, Quick Mask and layer-mask rows; described "Hidden" when not visible.
/// - The shortcut chord at the trailing end.
pub(super) fn channel_row<'r, K: Clone + Eq + Hash + Debug + Send + Sync + 'static>(cx: &'r RowContext<'r, K>, at: usize) -> ListRow<'r> {
    let row = &cx.rows[at];
    let words = cx.words;
    let tooltip = if matches!(row.kind, ChannelKind::LayerMask(_)) { &words.mask_overlay } else { &words.toggle_visibility };
    let eye = ToggleCell::eye(EYE, row.visible).name(named(&words.visibility.clone(), &row.name)).tooltip(tooltip.clone());
    let mut list_row = ListRow::new(row.name.clone()).layer(true).toggle(eye).italic(row.kind.italic());
    if !row.visible {
        list_row = list_row.description(words.hidden.as_str().to_owned());
    }
    if cx.size != ThumbSize::None {
        let (host, key) = (cx.thumbs, row.key.clone());
        let ink = match row.kind {
            ChannelKind::Spot(ink) => Some(ink),
            _ => None,
        };
        list_row = list_row.thumb(cx.size, move |painter, rect| {
            let mut fitted = rect;
            if let Some(host) = host {
                let request = ThumbRequest::for_cell(rect.width().max(rect.height()), painter.pixels_per_point());
                let state = host.thumbnails.state(painter.painter.ctx(), host.source, &key, &request, Priority::VISIBLE);
                if let Some(texture) = state.shown() {
                    fitted = fit_rect(rect, texture.size_vec2());
                }
                host.thumbnails.paint(painter, rect, &state, &ThumbPaint::default());
            }
            if let Some(ink) = ink {
                paint_spot_swatch(painter, fitted, Color32::from(ink));
            }
        });
    }
    match cx.numbers[at].and_then(super::labels::chord_text) {
        Some(chord) => list_row.trailing(chord),
        None => list_row,
    }
}

/// A spot channel's ink as a square at the bottom right corner of the fitted thumbnail (pictures never mirror in a
/// right-to-left interface). The ink is data, exempt from contrast checks.
fn paint_spot_swatch(painter: &craft_ui::primitives::ContentPainterCx<'_>, thumb: Rect, ink: Color32) {
    let side = thumb.width().min(thumb.height()) * SPOT_SWATCH_SHARE;
    let swatch = Rect::from_min_size(thumb.right_bottom() - vec2(side, side), vec2(side, side));
    painter.painter.rect_filled(swatch, 0.0, ink);
}
