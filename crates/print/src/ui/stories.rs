//! Stories of `PrintPanel`.

use super::{Duplex, PrintPanel, Printer, Settings};
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn printers() -> Vec<Printer> {
    vec![
        Printer::new("_192_0_2_15").label("192.0.2.15").is_default(true),
        Printer::new("Office_1").label("Office"),
        Printer::new("Office_2").label("Office"),
    ]
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Settings { printer: "Office_1".to_owned(), duplex: Some(Duplex::LongEdge), ..Settings::default() };
    let printers = printers();
    PrintPanel::new(Bind::bound(data.value("settings", start)), &printers).show(ui).into()
}

#[story(PrintPanel<'static>, covers = [], drive = Drive::Never)]
fn print_panel_discovering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PrintPanel::new(Bind::bound(data.value("settings", Settings::default())), &[]).discovering(true).show(ui).into()
}
