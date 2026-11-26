//! The theme styles' `right_to_left` and font coverage transfer function.

use super::memory;
use craft_theme::Mode;
use egui::epaint::FontColorTransferFunction;
use egui::{Context, Theme, Ui};
use std::sync::Arc;

/// Turns off egui's own caret blink in `ui`, and hides the caret stroke unless `visible`: the runtime blinks it.
pub(crate) fn set_text_cursor(ui: &mut Ui, visible: bool) {
    ui.visuals_mut().text_cursor.blink = false;
    if !visible {
        ui.visuals_mut().text_cursor.stroke.color = egui::Color32::TRANSPARENT;
    }
}

/// The width of the caret an editable text draws past its last letter: the one value measuring and styling both read.
pub(crate) fn caret_width(ctx: &Context) -> f32 {
    ctx.global_style().visuals.text_cursor.stroke.width
}

/// Paints the selected text of `ui`'s text edits in `ink` on `fill`.
pub(crate) fn set_text_selection(ui: &mut Ui, fill: egui::Color32, ink: egui::Color32) {
    let selection = &mut ui.visuals_mut().selection;
    selection.bg_fill = fill;
    selection.stroke.color = ink;
}

/// Paints the hint of `ui`'s text edits in `color`: egui draws it in the visuals' weak text colour.
pub(crate) fn set_hint_color(ui: &mut Ui, color: egui::Color32) {
    ui.visuals_mut().weak_text_color = Some(color);
}

/// Lets the rect `ui` was given set the height of its text edits: egui raises a text edit to the host's
/// `interact_size` and adds its extra line spacing, which an app's own egui theme sets for its egui widgets.
pub(crate) fn set_text_edit_unfloored(ui: &mut Ui) {
    let spacing = ui.spacing_mut();
    spacing.interact_size.y = 0.0;
    spacing.extra_text_line_spacing = 0.0;
}

/// Disables `ui` without fading it: the contract already inks a disabled part in its disabled colour, so egui's own
/// disabled alpha would dim it a second time.
pub(crate) fn disable_without_fade(ui: &mut Ui) {
    ui.visuals_mut().disabled_alpha = 1.0;
    ui.disable();
}

/// Whether egui considers `ui` enabled.
pub(crate) fn is_enabled(ui: &Ui) -> bool {
    ui.is_enabled()
}

/// Writes the direction into both theme styles and into `root`, whose style was copied before the begin hook ran.
pub(crate) fn set_theme_direction(ctx: &Context, root: &mut Ui, right_to_left: bool) {
    ctx.all_styles_mut(|style| {
        if style.right_to_left != right_to_left {
            style.right_to_left = right_to_left;
        }
    });
    if root.style().right_to_left != right_to_left {
        root.style_mut().right_to_left = right_to_left;
    }
}

/// Sets the spacing between the children `ui` lays out.
pub(crate) fn set_item_spacing(ui: &mut Ui, gap: egui::Vec2) {
    ui.spacing_mut().item_spacing = gap;
}

/// The egui style `ui` has now, for a framework part that shows an app's own egui code in a `Ui` of its own
/// (`primitives::HostStyle`).
pub(crate) fn style_of(ui: &Ui) -> Arc<egui::Style> {
    Arc::clone(ui.style())
}

/// Gives `ui` the style `style`: an app's own egui style kept by `primitives::HostStyle`, or `ui`'s own back after it.
pub(crate) fn set_style(ui: &mut Ui, style: &Arc<egui::Style>) {
    ui.set_style(Arc::clone(style));
}

/// Each egui theme style's own font coverage transfer function, and the one [`set_text_color_transfer`] last wrote.
#[derive(Clone, Copy)]
struct TextCurves {
    light: FontColorTransferFunction,
    dark: FontColorTransferFunction,
    written: FontColorTransferFunction,
}

/// Matches egui's font coverage transfer function to `mode` across both egui theme styles, so dark-on-light text
/// never renders with the dark-mode curve (or vice versa) when the app's mode differs from the OS's.
///
/// The curve for `mode` is the one the app set on that mode's egui style (egui's default until it sets one): a
/// style whose curve is not the one written here last has been set by the app since, and that curve becomes the
/// style's own.
pub(crate) fn set_text_color_transfer(ctx: &Context, mode: Mode) {
    let id = memory::unique("craft-ui-contract text color transfer");
    let held = |theme: Theme| ctx.style_of(theme).visuals.text_options.color_transfer_function;
    let (light, dark) = (held(Theme::Light), held(Theme::Dark));
    let (own_light, own_dark) = match memory::peek_copy::<TextCurves>(ctx, id) {
        Some(last) => (if light == last.written { last.light } else { light }, if dark == last.written { last.dark } else { dark }),
        None => (light, dark),
    };
    let transfer = match mode {
        Mode::Light => own_light,
        Mode::Dark => own_dark,
    };
    if light != transfer || dark != transfer {
        ctx.all_styles_mut(|style| {
            style.visuals.text_options.color_transfer_function = transfer;
        });
    }
    memory::put(ctx, id, TextCurves { light: own_light, dark: own_dark, written: transfer });
}
