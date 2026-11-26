//! The look of a button a sheet draws in or beside a cell: a header cell's filter button and the error checking tag.

use craft_theme::Stateful;
use craft_ui::style::{IconLook, StyleColor, StyleCx, SurfaceKind, SurfaceLook, UiTokens};

/// A small raised box with a control's corners, the variant's fill and edge: a spreadsheet's cell buttons are
/// field-like boxes in the grid, not the rounded buttons of a panel.
pub(super) fn surface(cx: &StyleCx<'_>) -> SurfaceLook {
    let parts = cx.variant_look();
    let mut surface = SurfaceLook::new(parts.fill.clone())
        .radius(UiTokens::corner(cx.tokens.shape.radius.control))
        .kind(SurfaceKind::Raised)
        .kind_down(SurfaceKind::Pressed);
    if let Some(edge) = parts.edge {
        surface = surface.border(edge, cx.tokens.stroke.thin);
    }
    surface
}

/// The glyph: the small icon size, so the box fits inside a text row.
pub(super) fn glyph(cx: &StyleCx<'_>, color: Stateful<StyleColor>) -> IconLook {
    IconLook::new(cx.tokens.size.icon.sm, color)
}
