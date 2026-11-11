//! `ContourField`: a contour picker over `PresetPicker`, with an Anti-aliased check box; `ContourEditorDialog` edits
//! the contour as a curve.

mod editor;
#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::{EditEvents, Mixed};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, EditResponse, NoValue, Outcome, Pattern, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::ThumbAspect;
use craft_ui::patterns::presets::{PickerField, PresetEvent, PresetFooterEvent, PresetPicker, PresetRow, PresetState};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{Corners, SurfaceKind};
use egui::{Rect, Response, Shape, Stroke, Ui, pos2};

pub use editor::ContourEditorDialog;

use crate::layer_style::{Contour, ContourPoint};
use crate::ui::curve::Curve;

craft_ui::words! {
    /// The words of the contour field and the Contour Editor, and the names of the built-in contours.
    #[non_exhaustive]
    pub struct ContourWords {
        editor: "contour.editor" = "Contour Editor";
        edit: "contour.edit" = "Edit Contour…";
        presets: "contour.presets" = "Contour presets";
        anti_aliased: "contour.antiAliased" = "Anti-aliased";
        preset: "contour.preset" = "Preset";
        input: "contour.input" = "Input";
        output: "contour.output" = "Output";
        corner: "contour.corner" = "Corner";
        new: "contour.new" = "New…";
        load: "contour.load" = "Load…";
        save: "contour.save" = "Save…";
        name: "contour.name" = "Name";
        /// The name a New contour starts with.
        suggested_name: "contour.suggestedName" = "Contour";
        preset_linear: "contour.presetLinear" = "Linear";
        preset_cone: "contour.presetCone" = "Cone";
        preset_cone_inverted: "contour.presetConeInverted" = "Cone - Inverted";
        preset_cove_deep: "contour.presetCoveDeep" = "Cove - Deep";
        preset_cove_shallow: "contour.presetCoveShallow" = "Cove - Shallow";
        preset_gaussian: "contour.presetGaussian" = "Gaussian";
        preset_half_round: "contour.presetHalfRound" = "Half Round";
        preset_ring: "contour.presetRing" = "Ring";
        preset_ring_double: "contour.presetRingDouble" = "Ring - Double";
        preset_rolling_slope: "contour.presetRollingSlope" = "Rolling Slope - Descending";
        preset_rounded_steps: "contour.presetRoundedSteps" = "Rounded Steps";
        preset_sawtooth: "contour.presetSawtooth" = "Sawtooth 1";
        preset_custom: "contour.presetCustom" = "Custom";
    }
}

impl ContourWords {
    /// The shown name of the contour called `name`: the built-in names translate, others show as they are.
    pub(crate) fn contour_name<'a>(&'a self, name: &'a str) -> &'a str {
        match name {
            "Linear" => self.preset_linear.as_str(),
            "Cone" => self.preset_cone.as_str(),
            "Cone - Inverted" => self.preset_cone_inverted.as_str(),
            "Cove - Deep" => self.preset_cove_deep.as_str(),
            "Cove - Shallow" => self.preset_cove_shallow.as_str(),
            "Gaussian" => self.preset_gaussian.as_str(),
            "Half Round" => self.preset_half_round.as_str(),
            "Ring" => self.preset_ring.as_str(),
            "Ring - Double" => self.preset_ring_double.as_str(),
            "Rolling Slope - Descending" => self.preset_rolling_slope.as_str(),
            "Rounded Steps" => self.preset_rounded_steps.as_str(),
            "Sawtooth 1" => self.preset_sawtooth.as_str(),
            "Custom" => self.preset_custom.as_str(),
            other => other,
        }
    }
}

/// A host's interpolation of a contour's points, shared with the thumbnails' painters.
pub(crate) type SharedSampler = Arc<dyn Fn(&[[f32; 2]]) -> Vec<f32> + Send + Sync>;

/// The levels of a contour as a curve: input and output run `0..=255`.
const LEVELS: f32 = 255.0;
/// The line of a thumbnail's curve, as a share of its side.
const LINE_SHARE: f32 = 0.05;
/// The room around a thumbnail's curve, as a share of its side.
const INSET_SHARE: f32 = 0.08;

/// A contour a picker lists: one of the built-in contours or one of the user's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContourKey {
    /// The built-in contour at this place of [`Contour::presets`].
    Preset(usize),
    /// The user's contour at this place of the list given to [`ContourField::user`].
    User(usize),
}

/// What the contour field's library asks of the app.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ContourAction {
    /// Load contours from a file, adding them to the user's.
    Load,
    /// Load contours from a file, replacing the user's.
    Replace,
    /// Save the user's contours to a file.
    Save,
    /// Delete the user's contour at this place.
    Delete(usize),
    /// Add the contour to the user's.
    New(Contour),
    /// Edit the contour.
    Edit(Contour),
}

/// A request to open the Contour Editor on a contour: `key` names what the contour belongs to (the field).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ContourRequest {
    /// What the contour belongs to.
    pub key: String,
    /// The contour to edit.
    pub contour: Contour,
}

impl ContourRequest {
    /// A request to edit `contour`, which belongs to `key`.
    #[must_use]
    pub fn new(key: impl Into<String>, contour: Contour) -> Self {
        Self { key: key.into(), contour }
    }
}

/// What [`ContourField::show`] returns.
#[derive(Debug)]
#[must_use = "the app applies the field's edits and requests"]
#[non_exhaustive]
pub struct ContourFieldOutput {
    /// The contour picked from the presets.
    pub edit: EditResponse<Contour>,
    /// The Anti-aliased check box's edit.
    pub anti_alias: EditEvents<Option<bool>>,
    /// The field asks for the Contour Editor.
    pub edit_contour: Option<ContourRequest>,
    /// The library asks for an action.
    pub action: Option<ContourAction>,
}

impl Outcome for ContourFieldOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

/// The rows of the contour picker: the built-in contours, then the user's, named by `words`, with a thumbnail of each
/// curve. `sampler` interpolates the points as the engine does; without it the thumbnails use a monotone cubic.
pub fn contour_rows(presets: &[Contour], user: &[Contour], words: &ContourWords, sampler: Option<SharedSampler>) -> Vec<PresetRow<ContourKey>> {
    let row = |key: ContourKey, contour: &Contour| {
        let name = words.contour_name(&contour.name).to_owned();
        let curve = Arc::new(to_curve(contour));
        let sampler = sampler.clone();
        PresetRow::item(key, name, 0).preview(ThumbAspect::SQUARE, move |painter, rect| paint_thumbnail(painter, rect, &curve, sampler.as_deref()))
    };
    let presets = presets.iter().enumerate().map(|(i, contour)| row(ContourKey::Preset(i), contour));
    let user = user.iter().enumerate().map(|(i, contour)| row(ContourKey::User(i), contour));
    presets.chain(user).collect()
}

type SamplerFn = dyn Fn(&[[f32; 2]]) -> Vec<f32> + Send + Sync;

/// A thumbnail of a contour: its curve over a well.
fn paint_thumbnail(painter: &ContentPainterCx<'_>, rect: Rect, curve: &Curve, sampler: Option<&SamplerFn>) {
    painter.surface(rect, SurfaceKind::Sunken, Corners::None);
    let Curve::Points { points, corners } = curve else { return };
    let outputs = Curve::through(points, corners, |points| match sampler {
        Some(sampler) => sampler(points),
        None => Curve::monotone(points, LEVELS),
    });
    let side = rect.width().min(rect.height());
    let area = rect.shrink(side * INSET_SHARE);
    let line: Vec<_> = outputs
        .iter()
        .enumerate()
        .map(|(input, output)| pos2(area.left() + input as f32 / LEVELS * area.width(), area.bottom() - output / LEVELS * area.height()))
        .collect();
    painter.painter.add(Shape::line(line, Stroke::new(side * LINE_SHARE, painter.ink_on(SurfaceKind::Sunken))));
}

/// The contour as a curve in levels.
pub fn to_curve(contour: &Contour) -> Curve {
    let points = contour.points.iter().map(|p| [p.input * LEVELS, p.output * LEVELS]).collect();
    Curve::from_parts(points, contour.points.iter().map(|p| p.corner).collect())
}

/// The contour a curve describes, named `name`; `None` for a drawn curve or too few or many points.
pub fn from_curve(curve: &Curve, name: &str) -> Option<Contour> {
    let points = curve.points()?;
    let points =
        points.iter().enumerate().map(|(i, p)| ContourPoint { input: p[0] / LEVELS, output: p[1] / LEVELS, corner: curve.is_corner(i) }).collect();
    Contour::from_points(name, points)
}

/// A contour field: the current contour's thumbnail opening the preset browser (a `PresetPicker`), an Edit Contour…
/// button that asks for the Contour Editor, and, with [`anti_alias`](ContourField::anti_alias), the Anti-aliased
/// check box.
///
/// A pick reports an edit of the contour. A built-in contour keeps its name; the one the editor made is "Custom".
/// Deleting a user's contour from the browser, or New in its footer, reports a [`ContourAction`].
#[must_use = "show the field with `show`"]
#[derive(Pattern)]
pub struct ContourField<'a> {
    label: Label,
    contour: Bind<'a, Contour>,
    rows: &'a [PresetRow<ContourKey>],
    state: Bind<'a, PresetState<ContourKey>>,
    user: &'a [Contour],
    anti_alias: Option<Bind<'a, Option<bool>>>,
}

impl<'a> ContourField<'a> {
    /// A field named `label` over `contour`, choosing among `rows` ([`contour_rows`]); `state` is the browser's view.
    pub fn new(
        label: impl Into<Label>,
        contour: Bind<'a, Contour>,
        rows: &'a [PresetRow<ContourKey>],
        state: Bind<'a, PresetState<ContourKey>>,
    ) -> Self {
        Self { label: label.into(), contour, rows, state, user: &[], anti_alias: None }
    }

    /// The user's contours, the targets of [`ContourKey::User`].
    pub fn user(mut self, user: &'a [Contour]) -> Self {
        self.user = user;
        self
    }

    /// Shows the Anti-aliased check box over `anti_alias`; `None` in the binding hides it.
    pub fn anti_alias(mut self, anti_alias: Bind<'a, Option<bool>>) -> Self {
        self.anti_alias = Some(anti_alias);
        self
    }

    /// Shows the field and returns this pass's edit and requests.
    pub fn show(self, ui: &mut Ui) -> ContourFieldOutput {
        let Self { label, mut contour, rows, mut state, user, mut anti_alias } = self;
        let env = Env::of(ui);
        let words = env.words::<ContourWords>();
        let presets = Contour::presets();
        let mut result: Option<ContourFieldOutput> = None;
        let shown = Stack::new().label(label.clone()).show(ui, |ui, root| {
            let held = contour.reborrow().read(root).and_then(|held| held.same().cloned()).unwrap_or_else(Contour::linear);
            let mut key = presets
                .iter()
                .position(|candidate| *candidate == held)
                .map(ContourKey::Preset)
                .or_else(|| user.iter().position(|candidate| *candidate == held).map(ContourKey::User));
            let mut action = None;
            let mut edit_contour = None;
            let mut anti_events = EditEvents::new();
            let mut picked = None;
            Row::new().show(ui, |ui, root| {
                let picker = PresetPicker::new(label.clone(), Bind::bound(&mut key), rows, state.reborrowed())
                    .field(PickerField::Thumbnail)
                    .can_delete(|key| matches!(key, ContourKey::User(_)))
                    .show(ui);
                for event in &picker.events {
                    match event {
                        PresetEvent::Delete(ContourKey::User(at)) => action = Some(ContourAction::Delete(*at)),
                        PresetEvent::Footer(PresetFooterEvent::New) => action = Some(ContourAction::New(held.clone())),
                        _ => {}
                    }
                }
                picked = Some(picker.edit);
                if clicked(&Button::new(words.edit.clone()).show(ui)) {
                    edit_contour = Some(ContourRequest { key: label.as_str().to_owned(), contour: held.clone() });
                }
                if let Some(bind) = anti_alias.as_mut()
                    && let Some(Mixed::Same(Some(on))) = bind.reborrow().read(root)
                {
                    let mut value = on;
                    let check = Checkbox::new(Bind::bound(&mut value)).label(words.anti_aliased.clone()).show(ui);
                    anti_events = check.events.map(Some);
                    for event in &anti_events {
                        bind.reborrow().write(root, event);
                    }
                }
            });
            let from_key = |key: Option<ContourKey>| match key {
                Some(ContourKey::Preset(at)) => presets.get(at).cloned().unwrap_or_else(|| held.clone()),
                Some(ContourKey::User(at)) => user.get(at).cloned().unwrap_or_else(|| held.clone()),
                None => held.clone(),
            };
            let edit = picked.expect("the row shows the picker").map(from_key);
            for event in &edit.events {
                contour.reborrow().write(root, event);
            }
            result = Some(ContourFieldOutput { edit, anti_alias: anti_events, edit_contour, action });
        });
        result.unwrap_or_else(|| ContourFieldOutput {
            edit: EditResponse::lifted(shown.output, EditEvents::new(), None),
            anti_alias: EditEvents::new(),
            edit_contour: None,
            action: None,
        })
    }
}
