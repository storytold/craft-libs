//! Stories of `AppShell` and `ShellRegion`.

use super::{AppShell, ShellRegion};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::dialog::Dialog;
use crate::display::Text;
use crate::layout_components::{Grid, Stack};
use crate::runtime::env::WindowKey;
use crate::shell::{CanvasPanel, Slot};
use crate::testing::{Drive, StoryData, StoryOutput, stage, story};
use craft_behavior::ColumnWidth;
use egui::Ui;

#[story(AppShell<'static, ()>, covers = [Default], drive = Drive::Never)]
fn app_shell(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    AppShell::new(WindowKey::main())
        .title_bar(|ui, _, _| {
            Text::new("Title").show(ui);
        })
        .top(Slot::new("options").reveal().label("Options"), |ui, _, _| {
            Text::new("Options").show(ui);
        })
        .bottom(Slot::new("status").label("Status"), |ui, _, _| {
            Text::new("Status").show(ui);
        })
        .start(Slot::new("tools").reveal().label("Tools"), |ui, _, _| {
            Text::new("Tools").show(ui);
        })
        .body(|ui, _, _| {
            Text::new("Document").show(ui);
        })
        .show_at(ui, room, &mut ())
        .into()
}

#[story(AppShell<'static, ()>, covers = [], drive = Drive::Never)]
fn app_shell_canvas(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    AppShell::new(WindowKey::main())
        .title_bar_if(true, |ui, _, _| {
            Text::new("Title").show(ui);
        })
        .options_bar(true, |ui, _, _| {
            Text::new("Options").show(ui);
        })
        .tools(true, |ui, _, _| {
            Text::new("Tools").show(ui);
        })
        .status(true, |ui, _, _| {
            Text::new("Status").show(ui);
        })
        .body(|ui, _, _| {
            CanvasPanel::new()
                .tabs(|ui, _| {
                    Text::new("Tabs").show(ui);
                })
                .show(ui, |ui, _| {
                    Text::new("Document").show(ui);
                });
        })
        .show_at(ui, room, &mut ())
        .into()
}

#[story(ShellRegion, covers = [Default], drive = Drive::Never)]
fn shell_region(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ShellRegion::window()
        .label("Window")
        .show(ui, |ui, _| {
            Text::new("Window").show(ui);
        })
        .into()
}

/// What the story dialogs hold: two check boxes whose values the story keeps between passes.
fn dialog_body(ui: &mut Ui, checks: &mut [bool; 2]) {
    let [snap, rulers] = checks;
    Stack::new().show(ui, |ui, _| {
        Checkbox::new(Bind::bound(snap)).label("Snap to grid").show(ui);
        Checkbox::new(Bind::bound(rulers)).label("Show rulers").show(ui);
    });
}

/// Two embedded regions side by side, each with its own open dialog: a dialog opened in a region is placed in it, its
/// scrim covers that region only and the other region stays in use. `open` opens both; once both are closed, `open` is
/// off and turning it on opens both again.
#[story(AppShell<'static, ()>, covers = [], drive = Drive::Never)]
fn app_shell_dialogs_in_regions(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = [*data.value("checks_a", [true, false]), *data.value("checks_b", [false, true])];
    let shows = *data.value("open", true);
    let mut open = if shows { [*data.value("open_a", true), *data.value("open_b", true)] } else { [false; 2] };
    let mut shown = Vec::new();
    let [checks_a, checks_b] = &mut checks;
    let [open_a, open_b] = &mut open;
    let region = |ui: &mut Ui, name: &str, checks: &mut [bool; 2], open: &mut bool| {
        AppShell::embedded(WindowKey::new(format!("dialog-region-{name}")))
            .body(|ui, _, _| {
                Dialog::new(format!("Region {name}")).opened(Bind::bound(&mut *open)).show(ui, |ui, _| dialog_body(ui, checks));
            })
            .show(ui, &mut ())
    };
    const HALVES: &[ColumnWidth] = &[ColumnWidth::fill(1.0), ColumnWidth::fill(1.0)];
    let room = stage(ui);
    Grid::new(HALVES).fill().show_at(ui, room, |ui, _| {
        shown.push(region(ui, "A", checks_a, open_a));
        shown.push(region(ui, "B", checks_b, open_b));
    });
    *data.value("checks_a", [true, false]) = checks[0];
    *data.value("checks_b", [false, true]) = checks[1];
    if shows {
        let closed = open == [false; 2];
        *data.value("open", true) = !closed;
        *data.value("open_a", true) = open[0] || closed;
        *data.value("open_b", true) = open[1] || closed;
    }
    shown.pop().map_or_else(StoryOutput::default, StoryOutput::from)
}
