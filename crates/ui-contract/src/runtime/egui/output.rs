//! What a pass asks the platform to do through `PlatformOutput`: the cursor icon, the cursor image and opening a URL.

use crate::runtime::cursor::CursorBitmap;
use craft_behavior::CursorIcon;
use egui::Context;

/// Sets the system cursor of the running pass.
pub(crate) fn set_cursor(ctx: &Context, icon: CursorIcon) {
    ctx.set_cursor_icon(egui_icon(icon));
}

/// Sets the cursor image of the running pass, which the integration shows instead of the system cursor; `None` clears
/// it.
pub(crate) fn set_cursor_image(ctx: &Context, image: Option<&CursorBitmap>) {
    ctx.set_cursor_image(image.map(|image| egui::CustomCursorImage { rgba: image.rgba.clone(), size: image.size, hotspot: image.hotspot }));
}

/// Asks the platform to open `url`, in a new tab where the platform has tabs and `new_tab` is set. On the web a new tab
/// opens through the platform's one web opener, which gives the page no handle on the app (`noopener,noreferrer`).
pub(crate) fn open_url(ctx: &Context, url: &str, new_tab: bool) {
    if !crate::runtime::measure::effect(ctx, "open_url") {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    if new_tab {
        if let Err(error) = craft_platform::open::open_external(url) {
            log::warn!("could not open {url:?} in a new tab: {error}");
        }
        return;
    }
    ctx.open_url(if new_tab { egui::OpenUrl::new_tab(url) } else { egui::OpenUrl::same_tab(url) });
}

fn egui_icon(icon: CursorIcon) -> egui::CursorIcon {
    use egui::CursorIcon as E;
    match icon {
        CursorIcon::Default => E::Default,
        CursorIcon::None => E::None,
        CursorIcon::ContextMenu => E::ContextMenu,
        CursorIcon::Help => E::Help,
        CursorIcon::PointingHand => E::PointingHand,
        CursorIcon::Progress => E::Progress,
        CursorIcon::Wait => E::Wait,
        CursorIcon::Cell => E::Cell,
        CursorIcon::Crosshair => E::Crosshair,
        CursorIcon::Text => E::Text,
        CursorIcon::VerticalText => E::VerticalText,
        CursorIcon::Alias => E::Alias,
        CursorIcon::Copy => E::Copy,
        CursorIcon::Move => E::Move,
        CursorIcon::NoDrop => E::NoDrop,
        CursorIcon::NotAllowed => E::NotAllowed,
        CursorIcon::Grab => E::Grab,
        CursorIcon::Grabbing => E::Grabbing,
        CursorIcon::AllScroll => E::AllScroll,
        CursorIcon::ResizeHorizontal => E::ResizeHorizontal,
        CursorIcon::ResizeNeSw => E::ResizeNeSw,
        CursorIcon::ResizeNwSe => E::ResizeNwSe,
        CursorIcon::ResizeVertical => E::ResizeVertical,
        CursorIcon::ResizeColumn => E::ResizeColumn,
        CursorIcon::ResizeEast => E::ResizeEast,
        CursorIcon::ResizeSouthEast => E::ResizeSouthEast,
        CursorIcon::ResizeSouth => E::ResizeSouth,
        CursorIcon::ResizeSouthWest => E::ResizeSouthWest,
        CursorIcon::ResizeWest => E::ResizeWest,
        CursorIcon::ResizeNorthWest => E::ResizeNorthWest,
        CursorIcon::ResizeNorth => E::ResizeNorth,
        CursorIcon::ResizeNorthEast => E::ResizeNorthEast,
        CursorIcon::ZoomIn => E::ZoomIn,
        CursorIcon::ZoomOut => E::ZoomOut,
        _ => E::Default,
    }
}
