//! One layer row: `LayerInfo` to `ListRow` (eye, thumbnails, kind icon, masks, brackets, trailing
//! parts, sub-rows).

use craft_behavior::{ColumnKey, IconId};
use craft_theme::{LabelColor, Rgba, Srgba};
use craft_ui::contract::Props as _;
use craft_ui::list::{ListRow, RowInk, ThumbSize, ThumbTarget, ToggleCell, TrailingFace, TrailingPart};
use craft_ui::primitives::textures::{Priority, ThumbPaint as PicturePaint, ThumbRequest};
use craft_ui::primitives::{ContentPainterCx, IconSpec};
use craft_ui::runtime::env::Label;
use craft_ui::style::{Corners, DataColor, SurfaceKind};
use egui::{Color32, Rect, Stroke, pos2, vec2};

use super::LayersPanelWords;
use super::context::{PanelContext, ThumbHost};
use super::drag::LayersDrag;
use crate::ui::thumbs::LayerThumb;
use crate::ui::tree::{LayerInfo, LayerKind, LayerPart, LayerRowKey, LayerTree, MaskInfo, SubRowGroup};

/// The eye column.
pub(super) const EYE: ColumnKey = ColumnKey::new("eye");
/// The lock column.
pub(super) const LOCK: ColumnKey = ColumnKey::new("lock");
/// The column that holds the thumbnails, the name and the indicators: the rest of the row after the toggle columns.
pub(super) const NAME: ColumnKey = ColumnKey::new("name");

/// The trailing lock indicator.
pub(super) const LOCK_PART: u16 = 0;
/// The trailing effects disclosure.
pub(super) const FX_PART: u16 = 1;

/// The cross over a disabled mask is a fixed data colour, like the label colours.
const DISABLED_MASK: LabelColor = LabelColor::Red;
const CROSS_INSET_SHARE: f32 = 0.12;
const MARK_WIDTH_SHARE: f32 = 0.06;
const BADGE_SHARE: f32 = 0.34;
const BADGE_GLYPH_SHARE: f32 = 0.28;

/// A data colour as the painter takes it.
pub(super) fn paint_color(color: Rgba) -> Color32 {
    Color32::from(Srgba::from(color))
}

/// `pattern` with `{row}` replaced by `row`.
pub(super) fn named(pattern: &Label, row: &str) -> Label {
    Label::from(craft_i18n::fmt(pattern.as_str(), &[("row", row)]))
}

pub(super) fn build_row<'r, T: LayerTree>(cx: &'r PanelContext<'r, T>, key: &LayerRowKey<T::Id>) -> ListRow<'r> {
    match key {
        LayerRowKey::Layer(id) => layer_row(cx, id),
        LayerRowKey::Effects(id, index) => sub_row(cx, id, SubRowGroup::Effects, *index),
        LayerRowKey::Filters(id, index) => sub_row(cx, id, SubRowGroup::SmartFilters, *index),
        LayerRowKey::Mask(id, _) => ListRow::new(cx.tree.info(id).name).layer(true),
    }
}

fn kind_icon(kind: LayerKind) -> Option<IconId> {
    Some(IconId::new(match kind {
        LayerKind::Group => "folder",
        LayerKind::Adjustment => "contrast",
        LayerKind::Fill => "paint_bucket",
        LayerKind::Text => "text",
        LayerKind::Shape => "rectangle",
        LayerKind::SmartObject => "image",
        LayerKind::Artboard => "artboard",
        LayerKind::Path => "spline",
        LayerKind::Image => "image",
        LayerKind::Object => "square",
        LayerKind::Pixel => return None,
    }))
}

/// The row's description for assistive technology: its kind, its flags and its target.
pub(super) fn row_description(words: &LayersPanelWords, info: &LayerInfo, target: Option<LayerPart>) -> String {
    let kind = match info.kind {
        LayerKind::Pixel => &words.kind_pixel,
        LayerKind::Group => &words.kind_group,
        LayerKind::Adjustment => &words.kind_adjustment,
        LayerKind::Fill => &words.kind_fill,
        LayerKind::Text => &words.kind_text,
        LayerKind::Shape => &words.kind_shape,
        LayerKind::SmartObject => &words.kind_smart_object,
        LayerKind::Artboard => &words.kind_artboard,
        LayerKind::Path => &words.kind_path,
        LayerKind::Image => &words.kind_image,
        LayerKind::Object => &words.kind_object,
    };
    let has = |part: LayerPart| info.masks.iter().any(|mask| mask.part == part);
    let flags = [
        (info.clipped, &words.clipped),
        (info.locks.any(), &words.locked),
        (info.linked, &words.linked),
        (has(LayerPart::LayerMask), &words.layer_mask),
        (has(LayerPart::VectorMask), &words.vector_mask),
        (info.masks.iter().any(|mask| !mask.enabled), &words.disabled_mask),
        (info.has_effects, &words.has_effects),
        (info.has_smart_filters, &words.has_smart_filters),
        (!info.visible, &words.hidden),
    ];
    let target = target.map(|part| match part {
        LayerPart::Pixels => &words.pixels_targeted,
        LayerPart::LayerMask => &words.layer_mask_targeted,
        LayerPart::VectorMask => &words.vector_mask_targeted,
        LayerPart::FilterMask => &words.filter_mask_targeted,
    });
    let mut parts: Vec<&str> = std::iter::once(kind.as_str()).chain(flags.iter().filter(|(on, _)| *on).map(|(_, word)| word.as_str())).collect();
    parts.extend(target.map(Label::as_str));
    parts.extend(info.description.as_deref());
    parts.extend(info.warning.as_deref());
    parts.join(", ")
}

fn sub_row_description(words: &LayersPanelWords, group: SubRowGroup, layer_name: &str) -> String {
    let pattern = match group {
        SubRowGroup::Effects => &words.effect_of,
        SubRowGroup::SmartFilters => &words.smart_filter_of,
    };
    craft_i18n::fmt(pattern.as_str(), &[("layer", layer_name)])
}

/// The thumbnail's picture, asked for at the cell it is painted in; false while nothing is ready.
fn paint_picture<Id: Clone + Eq + std::hash::Hash + Send + Sync + 'static>(
    host: Option<ThumbHost<'_, Id>>,
    thumb: &LayerThumb<Id>,
    painter: &ContentPainterCx<'_>,
    rect: Rect,
    paint: &PicturePaint,
) -> bool {
    let Some(host) = host else { return false };
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), painter.pixels_per_point());
    let state = host.thumbnails.state(painter.painter.ctx(), host.source, thumb, &request, Priority::VISIBLE);
    host.thumbnails.paint(painter, rect, &state, paint)
}

/// The cross of a disabled mask and the badge that names a layer or vector mask.
fn paint_mask_marks(painter: &ContentPainterCx<'_>, rect: Rect, mask: &MaskInfo, badges: bool) {
    let side = rect.width().min(rect.height());
    if !mask.enabled {
        let inner = rect.shrink(side * CROSS_INSET_SHARE);
        let stroke = Stroke::new(side * MARK_WIDTH_SHARE, paint_color(DISABLED_MASK.fill()));
        painter.painter.line_segment([inner.left_top(), inner.right_bottom()], stroke);
        painter.painter.line_segment([inner.right_top(), inner.left_bottom()], stroke);
    }
    if badges {
        let badge = side * BADGE_SHARE;
        let area = Rect::from_min_size(pos2(rect.right() - badge, rect.bottom() - badge), vec2(badge, badge));
        painter.surface(area, SurfaceKind::Scrim, Corners::None);
        let ink = painter.ink_on(SurfaceKind::Scrim);
        let glyph = Stroke::new(badge * MARK_WIDTH_SHARE * 2.0, ink);
        if mask.part == LayerPart::VectorMask {
            let inset = area.shrink(badge * CROSS_INSET_SHARE * 2.0);
            painter.painter.line_segment([inset.left_bottom(), inset.right_top()], glyph);
        } else {
            painter.painter.circle_stroke(area.center(), badge * BADGE_GLYPH_SHARE, glyph);
        }
    }
}

fn layer_row<'r, T: LayerTree>(cx: &'r PanelContext<'r, T>, id: &T::Id) -> ListRow<'r> {
    let info = cx.tree.info(id);
    let words = cx.words;
    let size = cx.options.thumbnail_size;
    let contents = cx.options.thumbnail_contents;
    let active = cx.selection.active.as_ref() == Some(id);

    let mut eye = ToggleCell::eye(EYE, info.visible).name(named(&words.visibility, &info.name));
    if let Some(color) = info.label {
        eye = eye.fill(DataColor::new(paint_color(color.fill())));
    }
    let mut row = ListRow::new(info.name.clone()).layer(true).toggle(eye);
    if cx.lock_column {
        row = row.toggle(ToggleCell::lock(LOCK, info.locks.any()).name(named(&words.lock_cell, &info.name)));
    }
    row = row.clipped(info.clipped).highlight(cx.tree.hovered().as_ref() == Some(id));
    if info.dimmed {
        row = row.ink(RowInk::Dimmed);
    }

    if size != ThumbSize::None {
        let host = cx.thumbs;
        let layer = id.clone();
        row = row.thumb(size, move |painter, rect| {
            paint_picture(host, &LayerThumb::pixels(layer, contents), painter, rect, &PicturePaint::default());
        });
        for (at, mask) in info.masks.iter().enumerate() {
            let (layer, mask, badges, part) = (id.clone(), *mask, cx.options.show_layer_mask_badges, mask.part);
            row = row.extra_thumb(move |painter, rect| {
                paint_picture(host, &LayerThumb::of_part(layer, part, contents), painter, rect, &PicturePaint::default().checker(false));
                paint_mask_marks(painter, rect, &mask, badges);
            });
            let word = if part == LayerPart::VectorMask { &words.vector_mask } else { &words.layer_mask };
            let pattern = if mask.linked { &words.unlink_mask } else { &words.link_mask };
            row = row.thumb_link(mask.linked, craft_i18n::fmt(pattern.as_str(), &[("mask", word.as_str()), ("layer", info.name.as_str())]));
            row = row.thumb_drag(ThumbTarget::Extra(at), LayersDrag::new(cx.list, vec![LayerRowKey::Mask(id.clone(), part)]));
        }
        if active {
            let target = match cx.selection.target {
                LayerPart::Pixels => Some(ThumbTarget::First),
                LayerPart::FilterMask => None,
                part => Some(info.masks.iter().position(|mask| mask.part == part).map_or(ThumbTarget::First, ThumbTarget::Extra)),
            };
            if let Some(target) = target {
                row = row.thumb_target(target);
            }
        }
    } else if let Some(icon) = kind_icon(info.kind) {
        row = row.icon(IconSpec::svg(icon));
    }

    // The first trailing item is the outermost: the lock closes the row, the effects disclosure sits inside it, and the
    // blend label is the first to drop out when the panel is narrow.
    if info.locks.any() {
        let name = named(&words.unlock, &info.name);
        row = row.trailing_part(TrailingPart::new(LOCK_PART), TrailingFace::Icon(IconSpec::svg(IconId::new("lock"))), name);
    }
    if info.has_effects || info.has_smart_filters {
        let open = cx.tree.effects_open(id);
        let pattern = if open { &words.collapse_effects } else { &words.expand_effects };
        let name = craft_i18n::fmt(pattern.as_str(), &[("layer", info.name.as_str())]);
        row = row.trailing_part(TrailingPart::new(FX_PART), TrailingFace::disclosure(open), name);
    }
    if info.has_smart_filters {
        row = row.trailing_icon(IconSpec::svg(IconId::new("funnel")), words.smart_filters.clone());
    }
    if info.has_effects {
        row = row.trailing(words.fx.clone());
    }
    if info.linked {
        row = row.trailing_icon(IconSpec::svg(IconId::new("link")), words.linked.clone());
    }
    if let Some(label) = info.blend_label.clone() {
        row = row.optional(label);
    }
    if let Some(warning) = info.warning.clone() {
        row = row.trailing_icon(IconSpec::svg(IconId::new("warning")), warning);
    }
    row.description(row_description(words, &info, active.then_some(cx.selection.target)))
}

fn sub_row<'r, T: LayerTree>(cx: &'r PanelContext<'r, T>, id: &T::Id, group: SubRowGroup, index: Option<usize>) -> ListRow<'r> {
    let rows = cx.tree.sub_rows(id, group);
    let sub = rows.and_then(|rows| match index {
        None => Some(rows.header),
        Some(at) => rows.items.into_iter().nth(at),
    });
    let Some(sub) = sub else { return ListRow::new("").layer(true) };
    let words = cx.words;
    let layer_name = cx.tree.info(id).name;
    let mut row = ListRow::new(sub.label.clone()).layer(true).description(sub_row_description(words, group, &layer_name));
    if !sub.on {
        row = row.ink(RowInk::Dimmed);
    }
    if sub.eye_shown {
        row = row.toggle(ToggleCell::eye(EYE, sub.on).name(named(&words.visibility, &sub.label)));
    }
    let size = cx.options.thumbnail_size;
    let header_mask = group == SubRowGroup::SmartFilters && index.is_none() && sub.mask && size != ThumbSize::None;
    if header_mask {
        let (host, layer) = (cx.thumbs, id.clone());
        row = row.thumb(size, move |painter, rect| {
            paint_picture(host, &LayerThumb::filter_mask(layer), painter, rect, &PicturePaint::default().checker(false));
        });
        if cx.selection.active.as_ref() == Some(id) && cx.selection.target == LayerPart::FilterMask {
            row = row.thumb_target(ThumbTarget::First);
        }
    } else if let Some(icon) = sub.icon {
        row = row.icon(IconSpec::svg(icon));
    }
    row
}
