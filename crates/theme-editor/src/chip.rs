//! The specimens the theme panels draw in a row's thumbnail: a colour, and a foreground colour beside its
//! background.

use craft_theme::Rgba;
use craft_ui::primitives::ContentPainterCx;
use egui::{Color32, Rect, pos2};

fn color_of(color: Rgba) -> Color32 {
    let [r, g, b, a] = color.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Fills `rect` with `color` over the transparency checker, which a translucent colour shows through.
fn fill(cx: &ContentPainterCx, rect: Rect, color: Rgba) {
    if color.0[3] < u8::MAX {
        cx.checker(rect);
    }
    cx.painter.rect_filled(rect, 0.0, color_of(color));
}

/// A thumbnail painter that fills the thumbnail with `color`, over the transparency checker where it is translucent.
pub(crate) fn chip(color: Rgba) -> impl FnOnce(&ContentPainterCx, Rect) + 'static {
    move |cx, rect| fill(cx, rect, color)
}

/// A thumbnail painter that shows `foreground` beside `background`: the background on the left half, the
/// foreground on the right half.
pub(crate) fn pair(foreground: Rgba, background: Rgba) -> impl FnOnce(&ContentPainterCx, Rect) + 'static {
    move |cx, rect| {
        let middle = rect.center().x;
        fill(cx, Rect::from_min_max(rect.min, pos2(middle, rect.max.y)), background);
        fill(cx, Rect::from_min_max(pos2(middle, rect.min.y), rect.max), foreground);
    }
}
