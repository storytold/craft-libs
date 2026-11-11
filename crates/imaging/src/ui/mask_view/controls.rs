//! [`MaskViewControls`]: the View Mode block of Select and Mask's Properties panel.

use super::{MaskViewChange, MaskViewMode, MaskViewState, MaskViewWords};
use craft_theme::TypeRole;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Pattern, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::form::{Field, SliderField, SliderFieldLayout};
use craft_ui::layout_components::Stack;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::Commands as _;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use egui::Ui;

/// What the View Mode controls report.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MaskViewControlsOutput {
    /// The change the user made this pass, already applied to the state.
    pub changed: Option<MaskViewChange>,
}

/// The View Mode controls: a "View" field with a Select named "View Mode" over the seven views (each row shows its
/// key), the current view's Opacity in percent (0 to 100, only for the four views that have one), the Show Edge and
/// Show Original check boxes, and the hint line in the caption role.
///
/// Remember Settings and the Refine Edge Brush controls are the workspace's. The keys are [`MaskViewKeys`]
/// (super::MaskViewKeys), which the workspace forwards to [`MaskViewState::run`].
#[must_use = "show the controls with `show`"]
#[derive(Pattern)]
pub struct MaskViewControls<'a> {
    state: &'a mut MaskViewState,
}

impl<'a> MaskViewControls<'a> {
    /// The controls of `state`.
    pub fn new(state: &'a mut MaskViewState) -> Self {
        Self { state }
    }

    /// Shows the controls.
    pub fn show(self, ui: &mut Ui) -> MaskViewControlsOutput {
        let Self { state } = self;
        let env = Env::of(ui);
        let words = env.words::<MaskViewWords>();
        let os = env.view().shell().os;
        let mut changed = None;
        Stack::new().show(ui, |ui, cx| {
            let keys: Vec<(MaskViewMode, String)> = MaskViewMode::ALL
                .into_iter()
                .map(|mode| {
                    (mode, cx.resolve_command(mode.command()).and_then(|resolved| resolved.chord).map(|chord| chord.display(os)).unwrap_or_default())
                })
                .collect();
            let entries: Vec<Entry<MaskViewMode>> =
                MaskViewMode::ALL.into_iter().map(|mode| Entry::new(mode, Label::from(mode.label(&words).to_owned()))).collect();
            Field::new(words.view.clone()).show(ui, |ui, _| {
                let key_of = |mode: &MaskViewMode| keys.iter().find(|(held, _)| held == mode).map(|(_, text)| text.clone()).unwrap_or_default();
                let select =
                    Select::new(words.view_mode.clone(), Bind::given(state.mode), &entries).decorate(move |mode, row| row.trailing(key_of(mode)));
                if let Some(&mode) = select.show(ui).value() {
                    let change = MaskViewChange::Mode(mode);
                    state.apply(change);
                    changed = Some(change);
                }
            });
            let mode = state.mode;
            if let Some(percent) = state.opacity(mode) {
                let opacity = SliderField::new(words.opacity.clone(), Bind::given(f64::from(percent)), 0.0..=100.0)
                    .step(1.0)
                    .unit_kind(UnitKind::Percent)
                    .layout(SliderFieldLayout::Inline)
                    .show(ui);
                if let Some(&value) = opacity.value() {
                    let value = (value as f32).round();
                    if Some(value) != state.opacity(mode) {
                        let change = MaskViewChange::Opacity(mode, value);
                        state.apply(change);
                        changed = Some(change);
                    }
                }
            }
            let edge = Checkbox::new(Bind::given(state.show_edge)).label(words.show_edge.clone()).show(ui);
            if let Some(&on) = edge.value() {
                let change = MaskViewChange::ShowEdge(on);
                state.apply(change);
                changed = Some(change);
            }
            let original = Checkbox::new(Bind::given(state.show_original)).label(words.show_original.clone()).show(ui);
            if let Some(&on) = original.value() {
                let change = MaskViewChange::ShowOriginal(on);
                state.apply(change);
                changed = Some(change);
            }
            Text::new(words.hint.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        });
        MaskViewControlsOutput { changed }
    }
}
