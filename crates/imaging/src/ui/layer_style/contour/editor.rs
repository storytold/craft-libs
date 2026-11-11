//! `ContourEditorDialog`: the Contour Editor, a stack dialog over the curve editor.

use std::cell::Cell;

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, BodyCx, Entry, Show as _, ShowContainer as _, clicked};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::text::TextField;
use egui::Ui;

use super::{ContourAction, ContourRequest, ContourWords, SharedSampler, from_curve, to_curve};
use crate::layer_style::Contour;
use crate::ui::curve::CurveEditor;

/// What the dialog edits: the contour, and the name of a New contour while its name row shows.
#[derive(Clone, Debug, PartialEq)]
struct Draft {
    contour: Contour,
    naming: Option<String>,
}

impl Default for Draft {
    fn default() -> Self {
        Draft { contour: Contour::linear(), naming: None }
    }
}

#[derive(Clone, Copy)]
enum Extra {
    Load,
    Save,
    New,
}

/// The Contour Editor: a Preset select, the contour's curve (corner points, at most [`Contour::MAX_POINTS`], 256
/// levels, in percent) with Input, Output and Corner fields, and Load, Save and New buttons. OK answers the contour,
/// Cancel nothing. New asks for a name in a row above the fields; Load, Save and a named New are reported through
/// [`take_action`](ContourEditorDialog::take_action). The values are live: [`current`](ContourEditorDialog::current)
/// holds the contour as edited.
pub struct ContourEditorDialog {
    id: DialogId,
    key: String,
    initial: Contour,
    draft: Draft,
    sampler: Option<SharedSampler>,
    changed: bool,
    action: Option<ContourAction>,
    focus_name: bool,
}

impl ContourEditorDialog {
    /// The editor on the contour of `request`.
    pub fn new(request: ContourRequest) -> Self {
        ContourEditorDialog {
            id: dialog_id("contourEditor"),
            key: request.key,
            initial: request.contour.clone(),
            draft: Draft { contour: request.contour, naming: None },
            sampler: None,
            changed: false,
            action: None,
            focus_name: false,
        }
    }

    /// The interpolation of the points, as the engine does it. Without it the curve is a monotone cubic.
    #[must_use]
    pub fn sampler(mut self, sampler: impl Fn(&[[f32; 2]]) -> Vec<f32> + Send + Sync + 'static) -> Self {
        self.sampler = Some(std::sync::Arc::new(sampler));
        self
    }

    /// What the request named.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The contour the dialog opened with.
    pub fn initial(&self) -> &Contour {
        &self.initial
    }

    /// The contour as edited.
    pub fn current(&self) -> &Contour {
        &self.draft.contour
    }

    /// The contour changed in the last pass.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// The contour is the one the dialog opened with.
    pub fn unchanged(&self) -> bool {
        self.draft.contour == self.initial
    }

    /// The library action the last pass asked for, once.
    pub fn take_action(&mut self) -> Option<ContourAction> {
        self.action.take()
    }
}

impl StackDialog for ContourEditorDialog {
    type Result = Contour;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Contour>> {
        let words = Env::of(ui).words::<ContourWords>();
        let before = self.draft.contour.clone();
        let extra = Cell::new(None::<Extra>);
        let named = Cell::new(false);
        let focus_name = std::mem::take(&mut self.focus_name);
        let sampler = self.sampler.clone();
        let shown = FormDialog::new(words.editor.clone(), Bind::bound(&mut self.draft))
            .outer_gesture(true)
            .footer_extra(|ui| {
                for (which, label) in [(Extra::Load, &words.load), (Extra::Save, &words.save), (Extra::New, &words.new)] {
                    if clicked(&Button::new(label.clone()).show(ui)) {
                        extra.set(Some(which));
                    }
                }
            })
            .show(ui, |ui, _, draft| {
                FieldGroup::new(words.preset.clone()).show(ui, |ui, _| {
                    Field::new(words.preset.clone()).show(ui, |ui, _| {
                        let entries: Vec<Entry<String>> = Contour::presets()
                            .into_iter()
                            .map(|preset| Entry::new(preset.name.clone(), words.contour_name(&preset.name).to_owned()))
                            .collect();
                        let mut name = draft.contour.name.clone();
                        let picked = Select::new(words.preset.clone(), Bind::bound(&mut name), &entries)
                            .placeholder(words.contour_name(&draft.contour.name).to_owned())
                            .show(ui);
                        if let Some(preset) = picked.value().and_then(|name| Contour::preset(name)) {
                            draft.contour = preset;
                        }
                    });
                });
                let mut curve = to_curve(&draft.contour);
                let graph_name = match draft.contour.name.is_empty() {
                    true => words.editor.as_str().to_owned(),
                    false => words.contour_name(&draft.contour.name).to_owned(),
                };
                let naming = &mut draft.naming;
                let between = |ui: &mut Ui, _: &mut BodyCx<'_>| {
                    let Some(text) = naming.as_mut() else { return };
                    Row::new().show(ui, |ui, _| {
                        Field::new(words.name.clone()).show(ui, |ui, _| {
                            let field = TextField::new(words.name.clone(), Bind::bound(text)).show(ui);
                            if focus_name {
                                focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
                            }
                            named.set(named.get() || field.submitted);
                        });
                        if clicked(&Button::new(words.new.clone()).show(ui)) {
                            named.set(true);
                        }
                    });
                };
                let mut editor = CurveEditor::new(graph_name, Bind::bound(&mut curve))
                    .levels(255)
                    .max_points(Contour::MAX_POINTS)
                    .bars(false)
                    .percent()
                    .corners()
                    .between(between);
                if let Some(sampler) = sampler.clone() {
                    editor = editor.sampler(move |points| sampler(points));
                }
                let edit = editor.show(ui);
                if let Some(next) = edit.events.value().and_then(|curve| from_curve(curve, &draft.contour.name)) {
                    draft.contour = next.edited();
                }
                true
            });

        if named.get()
            && let Some(typed) = self.draft.naming.take()
        {
            let name = if typed.trim().is_empty() { self.draft.contour.name.clone() } else { typed };
            let new = Contour { name, points: self.draft.contour.points.clone() };
            self.draft.contour.clone_from(&new);
            self.action = Some(ContourAction::New(new));
        }
        match extra.get() {
            Some(Extra::Load) => self.action = Some(ContourAction::Load),
            Some(Extra::Save) => self.action = Some(ContourAction::Save),
            Some(Extra::New) if self.draft.naming.is_none() => {
                self.draft.naming = Some(words.suggested_name.as_str().to_owned());
                self.focus_name = true;
            }
            _ => {}
        }
        self.changed = self.draft.contour != before;
        match (shown.accepted, shown.closed) {
            (Some(draft), _) => Some(DialogResult::Ok(draft.contour)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}
