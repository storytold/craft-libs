//! The Properties panel's story: Color Balance edited, then Reset.

use crate::ui::adjust::{AdjustContext, AdjustSession, ColorBalance, ColorBalanceEditor, ColorBalanceTone};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, clicked};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The layer the story's panel shows.
const TARGET: u64 = 1;

/// What the story's panel keeps: the layer's settings, the session around them and the pass count.
#[derive(Clone, Default, PartialEq)]
struct Panel {
    value: ColorBalance,
    session: AdjustSession<ColorBalance>,
    pass: u64,
    tone: Option<ColorBalanceTone>,
}

/// Reset returns to the settings the panel found while edited, and to the defaults the next time.
#[story(ColorBalanceEditor<'static>, covers = [], drive = Drive::Never)]
fn color_balance_properties(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let panel = data.value("panel", Panel { value: ColorBalance { shadows: [10.0, 0.0, -10.0], ..ColorBalance::default() }, ..Panel::default() });
    panel.pass += 1;
    panel.session.track(TARGET, panel.pass, &panel.value);
    let mut editor = ColorBalanceEditor::new("Color Balance", Bind::bound(&mut panel.value)).context(AdjustContext::properties(true));
    if let Some(tone) = panel.tone.take() {
        editor = editor.tone(tone);
    }
    let output = editor.show(ui);
    if !output.edit.events.is_empty() {
        panel.session.edited();
    }
    if clicked(&Button::new("Reset").show(ui)) {
        panel.value = panel.session.reset(ColorBalance::default);
        panel.tone = Some(ColorBalanceTone::Midtones);
    }
    output.into()
}
