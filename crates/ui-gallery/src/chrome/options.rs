//! The options bar: the theme and mode, the rarer appearance options in More, the workspace's own controls, and Reset
//! layout and the workspace switch at the end.

use crate::viewer::{
    CONTRASTS, DENSITIES, DIRECTIONS, GalleryCommands, MODES, MOTIONS, Preview, SCALES, TEXT_SIZES, VariantAxis, Viewer, Workspace, theme_entries,
};
use craft_behavior::{IconId, Value};
use craft_ui::bars::{Bar, BarCell, BarKind};
use craft_ui::buttons::{IconButton, ToggleButton};
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Trigger};
use craft_ui::display::Divider;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::Menu;
use craft_ui::pickers::Select;
use craft_ui::popover::Popover;
use egui::Ui;

/// A select over `entries` in a cell with its caption.
fn pick<T: Value>(ui: &mut Ui, caption: &'static str, value: &mut T, entries: &[Entry<T>]) {
    BarCell::new(format!("{caption}:")).control(ui, |ui, label| {
        Select::new(label, Bind::bound(&mut *value), entries).show(ui);
    });
}

/// A segmented row over `entries` in a cell with its caption.
fn choose<T: Value>(ui: &mut Ui, caption: &'static str, value: &mut T, entries: &[Entry<T>]) {
    BarCell::new(format!("{caption}:")).control(ui, |ui, label| {
        Segmented::new(label, Bind::bound(&mut *value), entries).show(ui);
    });
}

/// One line of a form: what `add` shows, in a row.
fn line(ui: &mut Ui, mut add: impl FnMut(&mut Ui)) {
    Row::new().show(ui, |ui, _| add(ui));
}

/// The More popover's form: the root values the bar's own controls leave out.
fn more_form(ui: &mut Ui, viewer: &mut Viewer) {
    let env = &mut viewer.env;
    let (contrasts, densities, motions) = (CONTRASTS.entries(), DENSITIES.entries(), MOTIONS.entries());
    let (directions, text_sizes, scales) = (DIRECTIONS.entries(), TEXT_SIZES.entries(), SCALES.entries());
    Stack::new().show(ui, |ui, _| {
        line(ui, |ui| choose(ui, "Contrast", &mut env.contrast, &contrasts));
        line(ui, |ui| choose(ui, "Size", &mut env.density, &densities));
        line(ui, |ui| choose(ui, "Reduced motion", &mut env.motion, &motions));
        line(ui, |ui| choose(ui, "Direction", &mut env.direction, &directions));
        line(ui, |ui| choose(ui, "Text", &mut env.text_size, &text_sizes));
        line(ui, |ui| pick(ui, "Scale", &mut env.scale, &scales));
    });
}

/// The Variants toggle and its menu of axes: the toggle shows whether a Variants view is open, and a press opens the
/// menu. Returns the axis chosen.
fn variants_menu(ui: &mut Ui, shown: bool, has_target: bool) -> Option<VariantAxis> {
    let mut on = shown;
    let button =
        ToggleButton::new("Variants", Bind::bound(&mut on)).trailing_icon(craft_ui::controls::disclosure::DisclosureGlyph::menu_button()).show(ui);
    let entries: Vec<Entry<VariantAxis>> = VariantAxis::ALL.into_iter().map(|axis| Entry::new(axis, axis.menu_label()).enabled(has_target)).collect();
    Menu::new("Variants").values().opened_by(&button.response, Trigger::Press).pick(ui, &entries).picked
}

/// The theme and mode, the More popover, the workspace's own controls, then Reset layout and the workspace switch.
pub fn options_bar(ui: &mut Ui, viewer: &mut Viewer) {
    let (modes, themes) = (MODES.entries(), theme_entries());
    let (previews, workspaces) = (
        [Entry::new(Preview::App, "App"), Entry::new(Preview::Components, "Components")],
        [Entry::new(Workspace::Components, "Components"), Entry::new(Workspace::Theme, "Theme")],
    );
    let has_target = viewer.browser.target_story().is_some();
    let workspace = viewer.workspace;
    let mut more = None;
    let Viewer { env, more_open, variants, preview, workspace: shown, .. } = viewer;
    Bar::new("Appearance", BarKind::Options)
        .trailing(|ui| {
            IconButton::command(IconId::new("undo"), GalleryCommands::ResetLayout).show(ui);
            choose(ui, "Workspace", shown, &workspaces);
        })
        .show(ui, |ui, _| {
            pick(ui, "Theme", &mut env.theme, &themes);
            choose(ui, "Mode", &mut env.mode, &modes);
            let button = IconButton::new(IconId::new("more")).label("More appearance settings").selected(*more_open).show(ui);
            if button.clicks > 0 {
                *more_open = !*more_open;
            }
            more = Some(button);
            Divider::vertical().show(ui);
            match workspace {
                Workspace::Components => {
                    if let Some(axis) = variants_menu(ui, variants.is_some(), has_target) {
                        *variants = Some(axis);
                    }
                }
                Workspace::Theme => choose(ui, "Preview", preview, &previews),
            }
        });
    // The popover is a layer of its own: shown inside the bar it would take the bar's height.
    if let Some(more) = more {
        let mut open = viewer.more_open;
        Popover::new("More appearance settings", Bind::bound(&mut open)).opened_by(&more.response).show(ui, |ui, _| more_form(ui, viewer));
        viewer.more_open = open;
    }
}
