//! Selective Color: its value and editor.

use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::{Bind, EditResponse, Entry, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::runtime::env::{Label, UnitKind};
use egui::Ui;

use super::Tables;
use crate::ui::adjust::sliders::{Run, lifted, read, words};
use crate::ui::adjust::{AdjustContext, AdjustField};

/// Selective Color's method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SelectiveMethod {
    /// Adjusts by a share of the colour already there; the default.
    #[default]
    Relative,
    /// Adjusts by the amount given.
    Absolute,
}

/// A Selective Color value: per range (Reds, Yellows, Greens, Cyans, Blues, Magentas, Whites, Neutrals, Blacks) the
/// Cyan, Magenta, Yellow and Black shifts, −100..=100 %, whole numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SelectiveColor {
    /// The nine ranges.
    pub ranges: [[f64; 4]; 9],
    /// Relative or Absolute.
    pub method: SelectiveMethod,
}

impl SelectiveColor {
    /// A value of the nine ranges and the method.
    pub fn new(ranges: [[f64; 4]; 9], method: SelectiveMethod) -> Self {
        Self { ranges, method }
    }
}

/// What a Selective Color editor keeps between passes: the range shown and the value its running gesture began with.
#[derive(Clone, Default)]
struct SelectiveMemory {
    range: usize,
    began: Option<SelectiveColor>,
}

/// The number of ranges.
const RANGES: usize = 9;

/// What a [`SelectiveColorEditor`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SelectiveColorOutput {
    /// The edit of the value.
    pub edit: EditResponse<SelectiveColor>,
    /// The range shown after the pass, 0 Reds … 8 Blacks.
    pub shown_range: usize,
}

impl Outcome for SelectiveColorOutput {
    type Value = SelectiveColor;

    fn response(&self) -> &egui::Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl std::ops::Deref for SelectiveColorOutput {
    type Target = EditResponse<SelectiveColor>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

/// Selective Color: the Colors select, the Cyan, Magenta, Yellow and Black rows of that range, and the Relative or
/// Absolute method.
///
/// The range shown is view state of the editor, Reds at first; a row edits the range shown only.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
pub struct SelectiveColorEditor<'a> {
    label: Label,
    value: Bind<'a, SelectiveColor>,
    context: AdjustContext,
}

impl<'a> SelectiveColorEditor<'a> {
    /// The editor named `label` of `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, SelectiveColor>) -> Self {
        Self { label: label.into(), value, context: AdjustContext::dialog(true) }
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the editor and returns this pass's edit of the `SelectiveColor` and the range shown.
    pub fn show(self, ui: &mut Ui) -> SelectiveColorOutput {
        let SelectiveColorEditor { label, mut value, context } = self;
        let words = words(ui);
        let mut out = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let now = read(&mut value, cx).unwrap_or_default();
            let memory = cx.memory::<SelectiveMemory>().clone();
            let range = memory.range.min(RANGES - 1);
            let names = [
                &words.reds,
                &words.yellows,
                &words.greens,
                &words.cyans,
                &words.blues,
                &words.magentas,
                &words.whites,
                &words.neutrals,
                &words.blacks,
            ];
            let entries: Vec<Entry<usize>> = names.iter().enumerate().map(|(i, name)| Entry::new(i, (*name).clone())).collect();
            let fields: Vec<AdjustField> = [("cyan", &words.cyan), ("magenta", &words.magenta), ("yellow", &words.yellow), ("black", &words.black)]
                .into_iter()
                .map(|(key, name)| AdjustField::new(key, name.clone(), -100.0..=100.0, 0.0).unit_kind(UnitKind::Percent))
                .collect();
            let mut run = Run::start(&mut value, context, now, memory.began);
            let tables = Tables { name: words.colors.clone(), entries: &entries, picked: range, fields: &fields, values: &now.ranges[range] };
            let picked = tables.show(ui, cx, &mut run, move |now: &SelectiveColor, i, x| {
                let mut next = *now;
                next.ranges[range][i] = x;
                next
            });
            let mut method = None;
            Field::new(words.relative_absolute.clone()).id_salt("method").show(ui, |ui, _| {
                let group = RadioGroup::new(words.relative_absolute.clone(), Bind::given(now.method)).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        Radio::new(SelectiveMethod::Relative).label(words.relative.clone()).show(ui);
                        Radio::new(SelectiveMethod::Absolute).label(words.absolute.clone()).show(ui);
                    });
                });
                method = Some(group.output);
            });
            if let Some(method) = method {
                run.push(cx, method, |now, method| SelectiveColor { method: *method, ..*now });
            }
            let memory = cx.memory::<SelectiveMemory>();
            memory.began = run.began();
            memory.range = picked.unwrap_or(range);
            out = Some((run.finish(), picked.unwrap_or(range)));
        });
        let (edit, shown_range) = out.map_or((None, 0), |(edit, range)| (Some(edit), range));
        SelectiveColorOutput { edit: lifted(edit, shown.output), shown_range }
    }
}
