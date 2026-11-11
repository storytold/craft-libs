//! The Marker dialog: the draft it edits, its result and the dialog itself.

use std::cell::Cell;

use craft_ui::buttons::Button;
use craft_ui::choice::{RadioGroup, Swatch};
use craft_ui::contract::{Bind, Outcome as _, Show as _, ShowContainer as _, clicked};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::text::TextField;
use egui::{Color32, Ui};

use super::MarkerWords;
use crate::markers::{Marker, MarkerColor, MarkerId};
use crate::time::Tick;
use crate::timecode::TimeFormat;
use crate::ui::{TimecodeCodec, TimelineWords, label_palette};

/// The Comments field's height in lines (EffectCraft `markers_ui.rs:151`).
const COMMENT_ROWS: usize = 3;
/// The sixteen label colours.
const LABELS: u8 = 16;

/// The marker as the dialog edits it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MarkerDraft {
    /// The marker's name.
    pub name: String,
    /// The marker's comment.
    pub comment: String,
    /// The marker's start time.
    pub start: Tick,
    /// The marker's duration; zero for a point marker.
    pub duration: Tick,
    /// The marker's colour label.
    pub color: MarkerColor,
}

/// What the Marker dialog answers with.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MarkerDialogResult {
    /// The marker the dialog edited.
    pub marker: MarkerId,
    /// What the user chose.
    pub action: MarkerDialogAction,
}

/// OK with the edited values, Delete, or Previous / Next (Premiere's: the edits are kept and the app opens the
/// dialog again on the neighbouring marker).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarkerDialogAction {
    /// OK: keep the edited values.
    Save(MarkerDraft),
    /// Remove the marker.
    Delete,
    /// Keep the edits and open the dialog on the previous marker.
    SaveAndPrevious(MarkerDraft),
    /// Keep the edits and open the dialog on the next marker.
    SaveAndNext(MarkerDraft),
}

/// Premiere's Marker dialog on the dialog host: Name, Time and Duration (timecode fields), Comments (multi-line),
/// Marker Color (the label swatches, one radio group), Delete, Previous and Next, and OK / Cancel. Enter in a
/// single-line field is OK; Escape is Cancel; Tab walks the fields in the order they show. Result: a
/// [`MarkerDialogResult`].
#[derive(Clone, PartialEq, craft_ui::contract::Pattern)]
pub struct MarkerDialog {
    id: MarkerId,
    draft: MarkerDraft,
    format: TimeFormat,
}

impl MarkerDialog {
    /// A dialog editing `marker`, with times shown in `format`.
    pub fn new(marker: &Marker, format: TimeFormat) -> Self {
        let draft = MarkerDraft {
            name: marker.name.clone(),
            comment: marker.comment.clone(),
            start: marker.start,
            duration: marker.duration,
            color: marker.color,
        };
        MarkerDialog { id: marker.id, draft, format }
    }
}

/// The footer buttons beyond OK and Cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Extra {
    Delete,
    Previous,
    Next,
}

impl StackDialog for MarkerDialog {
    type Result = MarkerDialogResult;

    fn id(&self) -> DialogId {
        dialog_id("timeline-marker")
    }

    // effectcraft: crates/ui-egui/src/panels/markers_ui.rs:124 `dialog`, `:47` `MarkerDraft`; layout as Premiere's
    // Marker dialog.
    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let env = Env::of(ui);
        let words = env.words::<MarkerWords>();
        let format = self.format;
        let picked = Cell::new(None::<Extra>);
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut self.draft))
            .outer_gesture(true)
            .footer_extra(|ui| {
                for (extra, label) in [(Extra::Delete, &words.delete), (Extra::Previous, &words.previous), (Extra::Next, &words.next)] {
                    if clicked(&Button::new(label.clone()).show(ui)) {
                        picked.set(Some(extra));
                    }
                }
            })
            .show(ui, |ui, form, draft| {
                let first = form.first();
                let timeline_words = Env::of(ui).words::<TimelineWords>();
                let palette = label_palette(&form.env);
                FieldGroup::new(words.title.clone()).show(ui, |ui, _| {
                    Field::new(words.name.clone()).show(ui, |ui, _| {
                        let field = TextField::new(words.name.clone(), Bind::bound(&mut draft.name)).show(ui);
                        if first {
                            focus::request(ui.ctx(), FocusTarget::Id(field.response().id));
                        }
                    });
                    Field::new(words.time.clone()).show(ui, |ui, _| {
                        TextField::with_codec(words.time.clone(), Bind::bound(&mut draft.start), TimecodeCodec::new(format)).show(ui);
                    });
                    Field::new(words.duration.clone()).show(ui, |ui, _| {
                        TextField::with_codec(words.duration.clone(), Bind::bound(&mut draft.duration), TimecodeCodec::new(format)).show(ui);
                    });
                    Field::new(words.comment.clone()).show(ui, |ui, _| {
                        TextField::new(words.comment.clone(), Bind::bound(&mut draft.comment)).lines(COMMENT_ROWS).show(ui);
                    });
                    Field::new(words.color.clone()).show(ui, |ui, _| {
                        let mut chosen = draft.color.0;
                        RadioGroup::new(words.color.clone(), Bind::bound(&mut chosen)).show(ui, |ui, _| {
                            Row::new().show(ui, |ui, _| {
                                for n in 0..LABELS {
                                    let label = MarkerColor(n);
                                    Swatch::new(timeline_words.label_name(label).to_owned(), srgba(palette.0[usize::from(n)])).member(n).show(ui);
                                }
                            });
                        });
                        draft.color = MarkerColor(chosen);
                    });
                });
                true
            });
        let answer = |action| Some(DialogResult::Ok(MarkerDialogResult { marker: self.id, action }));
        if let Some(extra) = picked.get() {
            return match extra {
                Extra::Delete => answer(MarkerDialogAction::Delete),
                Extra::Previous => answer(MarkerDialogAction::SaveAndPrevious(self.draft.clone())),
                Extra::Next => answer(MarkerDialogAction::SaveAndNext(self.draft.clone())),
            };
        }
        match (shown.accepted, shown.closed) {
            (Some(draft), _) => answer(MarkerDialogAction::Save(draft)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// `color` as the theme's colour type.
fn srgba(color: Color32) -> craft_theme::Srgba {
    let channel = |value: u8| f32::from(value) / 255.0;
    craft_theme::Srgba { r: channel(color.r()), g: channel(color.g()), b: channel(color.b()), a: channel(color.a()) }
}
