//! The presets list and the popup slider of a number field.

use super::NumberField;
use super::value::{NumberPick, NumberValue};
use crate::contract::{
    Bind, CollectionEvent, Cx, Edit as _, Entry, Overlay, OverlayUi, Overlays as _, Props as _, Show as _, ShowCollection as _, WordSetCx as _,
};
use crate::list::{List, ListRow};
use crate::runtime::env::Label;
use crate::value_drag::Slider;
use craft_behavior::{DisplayOrder, ItemSource, Selection};
use std::borrow::Cow;
use std::ops::RangeInclusive;

impl<'a, V: NumberValue + 'static> NumberField<'a, V> {
    pub(super) fn show_presets(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>, rows: &[Entry<NumberPick>]) {
        let popup = cx.popup("presets");
        let labels: Vec<Label> =
            rows.iter().map(|row| if row.is_separator() { Label::Empty } else { cx.env.resolve(&row.label().clone()) }).collect();
        let current = cx.value().and_then(NumberValue::number);
        let named = cx.value().and_then(NumberValue::named);
        let held = |at: usize| match rows[at].value() {
            Some(NumberPick::Number(value)) => current.is_some_and(|current| (current - value).abs() <= 1e-4 * current.abs().max(1.0)),
            Some(NumberPick::Choice(id)) => rows[at].is_checked() || named == Some(*id),
            None => false,
        };
        let marked = (0..rows.len()).find(|at| held(*at));
        let name = cx.label();
        let events = ui.show(popup, Overlay::dropdown(), |ui, _| {
            let selection = marked.map_or_else(Selection::default, |at| Selection::new(vec![at], Some(at), Some(at)));
            let source = Presets { rows, labels: &labels };
            List::<usize>::options(name.clone())
                .selection(Bind::given(selection))
                .show_items(ui, &source, &mut |at, _| {
                    if rows[*at].is_separator() {
                        ListRow::spacer()
                    } else {
                        ListRow::new(labels[*at].clone()).enabled(rows[*at].is_enabled()).option()
                    }
                })
                .events
        });
        let picked = events.into_iter().flatten().find_map(|event| match event {
            CollectionEvent::Activate(at) => rows[at].value().copied(),
            _ => None,
        });
        if let Some(pick) = picked {
            match pick {
                NumberPick::Number(value) => cx.set(V::of_number(self.settle_stored(value))),
                NumberPick::Choice(id) => {
                    if let Some(name) = V::from_name(id) {
                        cx.set(name);
                    }
                }
            }
            self.picked = Some(pick);
        }
    }

    pub(super) fn show_slider(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>, range: &RangeInclusive<f64>) {
        let popup = cx.popup("slider");
        let start = cx.value().and_then(NumberValue::number).unwrap_or_else(|| self.settle_stored((range.start() + range.end()) / 2.0));
        let mut shown = start;
        let name = cx.label();
        let step = self.step.map(|step| self.to_stored(step));
        let (rule, scale) = (self.rule(), self.scale);
        let edit = ui.show(popup, Overlay::popover(), |ui, _| {
            let slider = Slider::new(Bind::bound(&mut shown), range.clone()).label(name.clone());
            match step {
                Some(step) => slider.step(step).show(ui),
                None => slider.show(ui),
            }
        });
        if let Some(edit) = edit {
            cx.adopt(&edit, &V::of_number(start), |value, mine| *mine = V::of_number(rule.settle(*value * scale) / scale));
        }
    }
}

/// The preset rows as a collection source, keyed by position.
struct Presets<'a> {
    rows: &'a [Entry<NumberPick>],
    labels: &'a [Label],
}

impl DisplayOrder<usize> for Presets<'_> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.rows.len()).then_some(*key)
    }

    fn is_section(&self, at: usize) -> bool {
        self.rows[at].is_separator()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.labels[at].as_str()))
    }
}

impl ItemSource<usize> for Presets<'_> {}
