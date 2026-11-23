//! [`EffectParameters`]: the selected effect layer's filter select and parameters.

use craft_ui::contract::{Bind, NoValue, Outcome, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::form_dialogs::ParamRows;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::style::TypeRole;
use egui::{Response, Ui};

use super::catalog::FilterIndex;
use super::session::{EffectChange, EffectLayer};
use super::words::FilterGalleryWords;

/// What [`EffectParameters::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct EffectParametersOutput {
    /// The column's response.
    pub response: Response,
    /// A filter picked in the select, or the fields changed.
    pub change: Option<EffectChange>,
    /// A colour row's swatch pressed: the host opens its picker for the request and answers into the
    /// selected effect's fields.
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for EffectParametersOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The settings column's parameters: the Filter select (every filter, grouped by category) and the selected
/// effect layer's parameters through `ParamRows`. A colour row asks the app through
/// `EffectParametersOutput::pick_color`. A filter that uses the foreground and background colours says so under
/// its rows.
///
/// AccessKit: the select is a combo box named "Filter"; the rows are the form's.
#[derive(craft_ui::contract::Pattern)]
#[must_use = "show the parameters with `show`"]
pub struct EffectParameters<'a> {
    at: usize,
    effect: &'a EffectLayer,
    index: &'a FilterIndex,
}

impl<'a> EffectParameters<'a> {
    /// The settings of `effect` (at index `at` in the stack); `index` holds the filters.
    pub fn new(at: usize, effect: &'a EffectLayer, index: &'a FilterIndex) -> Self {
        Self { at, effect, index }
    }

    /// Draws the filter select and the parameter rows, and reports any change.
    pub fn show(self, ui: &mut Ui) -> EffectParametersOutput {
        let Self { at, effect, index } = self;
        let words = Env::of(ui).words::<FilterGalleryWords>();
        let entries = index.entries();
        let filter = index.filter(&effect.filter);
        let mut change = None;
        let mut pick_color = None;
        let stack = Stack::new().show(ui, |ui, _| {
            let picked = Select::new(words.filter.clone(), Bind::given(effect.filter.clone()), &entries).show(ui);
            if let Some(key) = picked.value().filter(|key| **key != effect.filter) {
                change = Some(EffectChange::set_filter(at, key.clone()));
            }
            let Some(filter) = filter else { return };
            let rows = ParamRows::new(words.settings.clone(), &filter.params, Bind::given(effect.fields.clone())).show(ui);
            if rows.changed
                && change.is_none()
                && let Some(fields) = rows.values
            {
                change = Some(EffectChange::with_fields(at, fields));
            }
            pick_color = rows.pick_color;
            if filter.uses_colors {
                Text::new(words.uses_colors.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
            }
        });
        EffectParametersOutput { response: stack.output, change, pick_color }
    }
}
