//! One layer row: `LayerInfo` to `ListRow` (eye, thumbnails, kind icon, masks, brackets, trailing
//! parts, sub-rows).

use craft_behavior::{ColumnKey, Cursor, CursorBadge, CursorIcon, IconId, PaintedCursor};
use craft_keymap::Mods;
use craft_theme::{LabelColor, Rgba, Srgba};
use craft_ui::contract::Props as _;
use craft_ui::list::{ListRow, RowDragSpot, RowInk, ThumbSize, ThumbTarget, ToggleCell, TrailingFace, TrailingPart};
use craft_ui::primitives::textures::{Priority, ThumbPaint as PicturePaint, ThumbRequest};
use craft_ui::primitives::{ContentPainterCx, IconSpec};
use craft_ui::runtime::env::Label;
use craft_ui::style::{Corners, DataColor, SurfaceKind};
use egui::emath::GuiRounding as _;
use egui::{Color32, Rect, Stroke, pos2, vec2};

use super::LayersPanelWords;
use super::body::thumb_click_replaces_select;
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
/// A Smart Filter's trailing Blending Options button.
pub(super) const OPTIONS_PART: u16 = 2;
/// The trailing `fx` badge: a drag handle that carries the layer's effects.
pub(crate) const FX_BADGE: u16 = 3;

/// The cross over a disabled mask is a fixed data colour, like the label colours.
const DISABLED_MASK: LabelColor = LabelColor::Red;
const CROSS_INSET_SHARE: f32 = 0.12;
const MARK_WIDTH_SHARE: f32 = 0.06;
const BADGE_GLYPH_SHARE: f32 = 0.28;
/// The kind glyph's share of the tile's shorter side.
const KIND_GLYPH_SHARE: f32 = 0.64;
/// A type layer's tile is ink on paper, the same in every theme, as the checkerboard is.
const TYPE_PAPER: Color32 = Color32::from_rgb(0xfa, 0xfa, 0xfa);
const TYPE_INK: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);

/// A data colour as the painter takes it.
pub(super) fn paint_color(color: Rgba) -> Color32 {
    Color32::from(Srgba::from(color))
}

/// The cursor over a thumbnail a Cmd/Ctrl click with `mods` loads as a selection: the pointing hand, with the mark of
/// the operation Shift and Alt pick (add, subtract, intersect).
fn load_cursor(mods: Mods) -> Cursor {
    let badge = match (mods.shift, mods.alt) {
        (true, true) => CursorBadge::Intersect,
        (true, false) => CursorBadge::Add,
        (false, true) => CursorBadge::Subtract,
        (false, false) => return Cursor::System(CursorIcon::PointingHand),
    };
    Cursor::Painted(PaintedCursor::badged(CursorIcon::PointingHand, badge))
}

/// `row` with the load cursor on its thumbnail `target` showing `part`, while Cmd/Ctrl is held and a click there loads
/// a selection.
fn with_load_cursor<'r>(row: ListRow<'r>, held: Mods, target: ThumbTarget, part: LayerPart, info: &LayerInfo) -> ListRow<'r> {
    if held.cmd && thumb_click_replaces_select(part, held, info) { row.thumb_cursor(target, load_cursor(held)) } else { row }
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
        LayerKind::Frame => "frame_placeholder",
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
        LayerKind::Frame => &words.kind_frame,
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
        LayerPart::FrameShape => &words.frame_shape_targeted,
        LayerPart::FrameContent => &words.frame_content_targeted,
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

/// The tile of a layer whose thumbnail has no picture: the sunken placeholder with the kind's glyph (a pixel layer has
/// none), or for a type layer its glyph in ink on paper.
fn paint_kind_tile(painter: &ContentPainterCx<'_>, rect: Rect, kind: LayerKind) {
    let rect = rect.round_to_pixels(painter.pixels_per_point());
    let side = rect.width().min(rect.height()) * KIND_GLYPH_SHARE;
    let glyph = Rect::from_center_size(rect.center(), vec2(side, side));
    let icon = kind_icon(kind).map(IconSpec::svg);
    if kind == LayerKind::Text {
        painter.painter.rect_filled(rect, 0.0, TYPE_PAPER);
        if let Some(icon) = icon {
            painter.icon(glyph, &icon.data_color(DataColor::new(TYPE_INK)));
        }
        return;
    }
    painter.surface(rect, SurfaceKind::Sunken, Corners::None);
    if let Some(icon) = icon {
        painter.icon(glyph, &icon.ink(SurfaceKind::Sunken.ink()));
    }
}

/// The square in the bottom corner of the thumbnail `rect` that a badge fills: the small icon size, the least that
/// still reads, within the thumbnail.
fn badge_area(painter: &ContentPainterCx<'_>, rect: Rect) -> Rect {
    let badge = painter.style_cx().tokens.size.icon.sm.min(rect.width().min(rect.height()));
    Rect::from_min_size(pos2(rect.right() - badge, rect.bottom() - badge), vec2(badge, badge))
}

/// The app's badge on a layer thumbnail: its icon at the small glyph size on the scrim.
fn paint_thumb_badge(painter: &ContentPainterCx<'_>, rect: Rect, icon: IconId) {
    let area = badge_area(painter, rect);
    painter.surface(area, SurfaceKind::Scrim, Corners::None);
    let side = painter.style_cx().tokens.size.icon.glyph_sm.min(area.width());
    let glyph = Rect::from_center_size(area.center(), vec2(side, side));
    painter.icon(glyph, &IconSpec::svg(icon).ink(SurfaceKind::Scrim.ink()));
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
        let area = badge_area(painter, rect);
        let badge = area.width();
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
    // An artboard without masks heads its layers: a strong row drawn as at thumbnail size None, never taller than a
    // layer row. With a mask it keeps its thumbnail, which the mask's target and clicks go back to.
    let compact = info.kind == LayerKind::Artboard && info.masks.is_empty();
    if compact {
        row = row.heading();
    }

    if size != ThumbSize::None && !compact {
        let host = cx.thumbs;
        let (layer, kind, badge) = (id.clone(), info.kind, info.thumb_badge.clone());
        row = row.thumb(size, move |painter, rect| {
            if !paint_picture(host, &LayerThumb::pixels(layer, contents), painter, rect, &PicturePaint::default()) {
                paint_kind_tile(painter, rect, kind);
            }
            if let Some(icon) = badge {
                paint_thumb_badge(painter, rect, icon);
            }
        });
        let frame = info.kind == LayerKind::Frame;
        let first = if frame { LayerPart::FrameShape } else { LayerPart::Pixels };
        row = with_load_cursor(row, cx.held, ThumbTarget::First, first, &info);
        if frame {
            let layer = id.clone();
            row = row.extra_thumb(move |painter, rect| {
                if !paint_picture(host, &LayerThumb::frame_content(layer, contents), painter, rect, &PicturePaint::default()) {
                    paint_kind_tile(painter, rect, LayerKind::Frame);
                }
            });
            row = with_load_cursor(row, cx.held, ThumbTarget::Extra(0), LayerPart::FrameContent, &info);
        }
        let masks_from = usize::from(frame);
        for (at, mask) in info.masks.iter().enumerate() {
            let (layer, mask, badges, part) = (id.clone(), *mask, cx.options.show_layer_mask_badges, mask.part);
            row = row.extra_thumb(move |painter, rect| {
                paint_picture(host, &LayerThumb::of_part(layer, part, contents), painter, rect, &PicturePaint::default().checker(false));
                paint_mask_marks(painter, rect, &mask, badges);
            });
            let word = if part == LayerPart::VectorMask { &words.vector_mask } else { &words.layer_mask };
            let pattern = if mask.linked { &words.unlink_mask } else { &words.link_mask };
            row = row.thumb_link(mask.linked, craft_i18n::fmt(pattern.as_str(), &[("mask", word.as_str()), ("layer", info.name.as_str())]));
            let target = ThumbTarget::Extra(masks_from + at);
            row = row.part_drag(RowDragSpot::Thumb(target), LayersDrag::new(cx.list, vec![LayerRowKey::Mask(id.clone(), part)]));
            row = with_load_cursor(row, cx.held, target, part, &info);
        }
        if active {
            match cx.selection.target {
                LayerPart::FilterMask => {}
                // A frame as a whole: its shape and its content.
                LayerPart::Pixels if frame => row = row.thumb_target([ThumbTarget::First, ThumbTarget::Extra(0)]),
                LayerPart::Pixels | LayerPart::FrameShape => row = row.thumb_target(ThumbTarget::First),
                part => row = row.thumb_target(info.thumb(part).unwrap_or(ThumbTarget::First)),
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
        row = row.trailing_icon(IconSpec::svg(IconId::new("smart_filters")), words.smart_filters.clone());
    }
    if info.has_effects {
        // The badge drags every effect of the layer (Photoshop); a click on it is a click on the row.
        let badge = TrailingPart::new(FX_BADGE);
        row = row
            .trailing_part(badge, TrailingFace::Strong(words.fx.clone()), words.has_effects.clone())
            .part_drag(RowDragSpot::Trailing(badge), LayersDrag::new(cx.list, vec![LayerRowKey::Effects(id.clone(), None)]));
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
    if let Some(cell) = cx.trailing {
        row = cell.add(row, id, &info);
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
        row = with_load_cursor(row, cx.held, ThumbTarget::First, LayerPart::FilterMask, &cx.tree.info(id));
    } else if let Some(icon) = sub.icon {
        row = row.icon(IconSpec::svg(icon));
    }
    if sub.options {
        let face = TrailingFace::Icon(IconSpec::svg(IconId::new("adjustments")));
        row = row.trailing_part(TrailingPart::new(OPTIONS_PART), face, named(&words.blending_options, &sub.label));
    }
    row
}
