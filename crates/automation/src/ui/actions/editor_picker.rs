//! Metadata pickers: explicit choices only; an unavailable identifier is never replaced on display.

use craft_behavior::IconId;
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::Row;
use craft_ui::primitives::IconSpec;
use egui::Ui;

use super::editor::{ActionEditorHost, ActionEditorState, ActionEditorValue};
use super::words::ActionEditorWords;
use crate::actions::ActionColor;

fn glyph(host: Option<&dyn ActionEditorHost>, name: &str) -> IconSpec<'static> {
    host.and_then(|host| host.icon(name)).unwrap_or_else(|| IconSpec::svg(IconId::new("play")))
}

pub(super) fn show(
    ui: &mut Ui,
    value: &mut ActionEditorValue,
    state: &mut ActionEditorState,
    host: Option<&dyn ActionEditorHost>,
    words: &ActionEditorWords,
    editable: bool,
) {
    let sizing = ui.is_sizing_pass();
    let trigger = Button::new(words.icon.clone())
        .icon(glyph(host, &value.action.icon))
        .selected(state.icons_open)
        .expanded(state.icons_open)
        .enabled(editable)
        .show(ui);
    if trigger.clicked() && !sizing {
        state.icons_open = !state.icons_open;
    }
    let mut picked = false;
    if state.icons_open {
        if Button::new(words.default_icon.clone()).enabled(editable).selected(value.action.icon.is_empty()).show(ui).clicked() && !sizing {
            value.action.icon.clear();
            state.icons_open = false;
            picked = true;
        }
        let icons = [
            ("ClickAction", words.icon_click.clone()),
            ("ActionsCrop", words.icon_crop.clone()),
            ("LayerMerge", words.icon_merge.clone()),
            ("AdjustmentGeneral", words.icon_adjustment.clone()),
            ("HueAndSat", words.icon_hue.clone()),
            ("ActionsExposure", words.icon_exposure.clone()),
            ("ColorLookUpTables", words.icon_lookup.clone()),
            ("BlackAndWhite", words.icon_monochrome.clone()),
            ("SmartRemove", words.icon_remove.clone()),
            ("ActionsDodge", words.icon_dodge.clone()),
            ("ActionsBurn", words.icon_burn.clone()),
            ("ActionsErase", words.icon_erase.clone()),
            ("LayerEffects", words.icon_effects.clone()),
            ("AIGenEffects", words.icon_generated_effects.clone()),
            ("Sharpen", words.icon_sharpen.clone()),
            ("ObjectSelectionLasso", words.icon_object_selection.clone()),
            ("MagicWand", words.icon_wand.clone()),
            ("AIGenerate", words.icon_generate.clone()),
            ("TextColor", words.icon_text_color.clone()),
            ("ActionsText", words.icon_text.clone()),
            ("ActionsDownload", words.icon_download.clone()),
            ("ActionsLayers", words.icon_layers.clone()),
            ("Brush", words.icon_brush.clone()),
            ("SelectSubject", words.icon_subject.clone()),
            ("RemoveBackground", words.icon_background.clone()),
            ("GaussianBlur", words.icon_blur.clone()),
            ("LayerHarmonize", words.icon_harmonize.clone()),
            ("P_AdjustClarity_32x32", words.icon_clarity.clone()),
            ("AIStyles", words.icon_styles.clone()),
            ("ShowGrid", words.icon_grid.clone()),
        ];
        for choices in icons.chunks(6) {
            Row::new().show(ui, |ui, _| {
                for (name, label) in choices {
                    if IconButton::new(glyph(host, name))
                        .label(label.clone())
                        .tooltip(label.clone())
                        .selected(value.action.icon == *name)
                        .id_salt(name)
                        .enabled(editable)
                        .show(ui)
                        .clicked()
                        && !sizing
                    {
                        value.action.icon = (*name).to_owned();
                        state.icons_open = false;
                        picked = true;
                    }
                }
            });
        }
    }
    if picked {
        trigger.response.request_focus();
    }
    Text::new(words.color.clone()).show(ui);
    // Display order is measured separately from the ATN header's index order.
    let colors = [
        (ActionColor::None, words.none.clone()),
        (ActionColor::Red, words.red.clone()),
        (ActionColor::Orange, words.orange.clone()),
        (ActionColor::Yellow, words.yellow.clone()),
        (ActionColor::Green, words.green.clone()),
        (ActionColor::Teal, words.teal.clone()),
        (ActionColor::Blue, words.light_blue.clone()),
        (ActionColor::Blue2, words.blue.clone()),
        (ActionColor::Magenta, words.magenta.clone()),
        (ActionColor::Purple, words.purple.clone()),
        (ActionColor::Violet, words.lavender.clone()),
        (ActionColor::Gray, words.gray.clone()),
    ];
    for choices in colors.chunks(6) {
        Row::new().show(ui, |ui, _| {
            for (color, label) in choices {
                let mut button = Button::new(label.clone()).selected(value.action.color == *color).enabled(editable).id_salt(color.atn_index());
                if let Some(tint) = color.label_color() {
                    button = button.tint(tint.fill());
                }
                if button.show(ui).clicked() && !sizing {
                    value.action.color = *color;
                }
            }
        });
    }
}
