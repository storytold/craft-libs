//! Stories of the transform bar: the Box phase and the Warp phase.

use super::{BarSlot, TransformBar};
use super::{TransformEdit, TransformSession, TransformSettings};
use craft_behavior::kurbo::Rect;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::display::Text;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The box before the transform, in document units.
const START: Rect = Rect::new(0.0, 0.0, 200.0, 100.0);

#[story(TransformBar<'static, ()>, covers = [], drive = Drive::Never)]
fn box_phase(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("settings", TransformSettings::default());
    let session = TransformSession::<()>::new(START);
    TransformBar::new("Free Transform", &session, Bind::bound(settings)).show(ui).into()
}

#[story(TransformBar<'static, ()>, covers = [], drive = Drive::Never)]
fn warp_phase(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("settings", TransformSettings::default());
    let mut session = TransformSession::<()>::new(START);
    let _ = session.apply(TransformEdit::Warp(()), 0.0);
    let fields = BarSlot::new(|ui: &mut Ui| {
        Text::new("Warp fields").show(ui);
    });
    TransformBar::new("Warp", &session, Bind::bound(settings)).warp_fields(fields).show(ui).into()
}
