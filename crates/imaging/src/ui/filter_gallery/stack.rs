//! [`EffectStack`]: the effect-layer stack under the settings.

use std::borrow::Cow;

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, IconId, ItemSource, Selection};
use craft_ui::contract::{
    CollectionEvent, ColumnKind, ColumnSpec, NoValue, Outcome, Remember as _, ReorderMove, ReorderRule, SelectMode, ShowCollection as _,
    ShowContainer as _, SweepApply, SweepStep, ToggleColumn,
};
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow, RowInk, ToggleCell};
use craft_ui::runtime::env::Env;
use craft_ui::sections::{FooterAction, FooterBar};
use egui::{Response, Ui};

use super::catalog::FilterIndex;
use super::session::{EffectChange, EffectLayer};
use super::words::FilterGalleryWords;

const EYE: ColumnKey = ColumnKey::new("eye");
const NAME: ColumnKey = ColumnKey::new("name");

/// The effect layers in display order: the top row is the layer applied last, so row `at` is effect
/// `count - 1 - at`, and a row's key is its effect's index.
struct Layers<'a> {
    effects: &'a [EffectLayer],
    index: &'a FilterIndex,
}

impl Layers<'_> {
    fn effect_at(&self, at: usize) -> usize {
        self.effects.len() - 1 - at
    }
}

impl DisplayOrder<usize> for Layers<'_> {
    fn len(&self) -> usize {
        self.effects.len()
    }

    fn key(&self, at: usize) -> usize {
        self.effect_at(at)
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.effects.len()).then(|| self.effects.len() - 1 - *key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.index.label(&self.effects[self.effect_at(at)].filter).as_str().to_owned()))
    }
}

impl ItemSource<usize> for Layers<'_> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == EYE && self.effects[self.effect_at(at)].visible
    }
}

/// What [`EffectStack::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct EffectStackOutput {
    /// The stack's response.
    pub response: Response,
    /// The layer picked (click, Up, Down).
    pub selected: Option<usize>,
    /// An eye, a move, New or Delete.
    pub change: Option<EffectChange>,
}

impl Outcome for EffectStackOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The stack: a `List` named "Effect Layers" that takes the height of its rows, top row last applied (so the
/// rows show `effects` in reverse), each row with a sweepable eye column and the filter's name, a hidden
/// layer's name dimmed; the selected row is the list's selection. Under it a footer bar with Move up, Move
/// down, New effect layer and Delete effect layer (disabled with one layer; the moves disabled at the ends).
/// Rows reorder by drag and by the collection's move keys.
///
/// Keyboard: Up and Down select, Space toggles the eye of the lead row. AccessKit: a list named "Effect
/// Layers"; rows named by their filter with the eye as a toggle.
#[derive(craft_ui::contract::Pattern)]
#[must_use = "show the stack with `show`"]
pub struct EffectStack<'a> {
    effects: &'a [EffectLayer],
    selected: usize,
    index: &'a FilterIndex,
}

impl<'a> EffectStack<'a> {
    /// The stack over `effects` with the layer at index `selected` selected; `index` names the filters.
    pub fn new(effects: &'a [EffectLayer], selected: usize, index: &'a FilterIndex) -> Self {
        Self { effects, selected, index }
    }

    /// Draws the stack and its buttons and reports any selection or change.
    pub fn show(self, ui: &mut Ui) -> EffectStackOutput {
        let Self { effects, selected, index } = self;
        let words = Env::of(ui).words::<FilterGalleryWords>();
        let count = effects.len();
        let current = selected.min(count.saturating_sub(1));
        let layers = Layers { effects, index };
        let mut selection = Selection::single(current);
        let columns = vec![
            ColumnSpec::new(EYE, words.visibility_of.clone(), ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())),
            ColumnSpec::new(NAME, words.filter.clone(), ColumnWidth::fill(1.0)),
        ];
        let mut change = None;
        let mut picked = None;
        let shown = Stack::new().show(ui, |ui, cx| {
            let out = List::<usize>::new(words.effect_layers.clone())
                .select(SelectMode::Single)
                .allow_empty(false)
                .header(false)
                .columns(columns.clone())
                .reorder(ReorderRule::new())
                .natural_height(true)
                .selection(craft_ui::contract::Bind::bound(&mut selection))
                .show_items(ui, &layers, &mut |key, icx| {
                    let effect = &effects[*key];
                    let ink = if effect.visible { RowInk::Normal } else { RowInk::Dimmed };
                    ListRow::new(index.label(&effect.filter))
                        .cells(icx.cells())
                        .ink(ink)
                        .toggle(ToggleCell::eye(EYE, effect.visible).name(words.visibility_of.clone()))
                });
            for event in &out.events {
                match event {
                    CollectionEvent::Toggle(edit) => {
                        if let Some(step) = cx.keep::<SweepApply>().feed(edit)
                            && let SweepStep::Flip(toggle) | SweepStep::Restore(toggle) = step
                            && effects.get(toggle.key).is_some_and(|effect| effect.visible != toggle.on)
                        {
                            change = Some(EffectChange::set_visible(toggle.key, toggle.on));
                        }
                    }
                    CollectionEvent::Reorder(edit) => {
                        if let Some(mv) = finished(edit)
                            && let Some((from, to)) = effect_move(count, mv)
                        {
                            change = Some(EffectChange::move_to(from, to));
                        }
                    }
                    _ => {}
                }
            }
            let actions = [
                FooterAction::new(IconId::new("chevron_up"), words.move_up.clone()).enabled(current + 1 < count),
                FooterAction::new(IconId::new("chevron_down"), words.move_down.clone()).enabled(current > 0),
                FooterAction::new(IconId::new("new_layer"), words.new_layer.clone()),
                FooterAction::new(IconId::new("trash"), words.delete_layer.clone()).enabled(count > 1),
            ];
            let footer = FooterBar::actions(actions).show_footer(ui);
            let pressed = match footer.picked {
                Some(0) => Some(EffectChange::move_to(current, current + 1)),
                Some(1) => Some(EffectChange::move_to(current, current.saturating_sub(1))),
                Some(2) => Some(EffectChange::Add),
                Some(3) => Some(EffectChange::Delete),
                _ => None,
            };
            if pressed.is_some() {
                change = pressed;
            }
            picked = selection.lead.filter(|&lead| lead != selected && lead < count);
        });
        EffectStackOutput { response: shown.output, selected: picked, change }
    }
}

/// The move a finished drag or move key makes.
fn finished(edit: &EditEvent<ReorderMove<usize>>) -> Option<&ReorderMove<usize>> {
    match edit {
        EditEvent::Set { after, .. } | EditEvent::Commit { after, .. } => Some(after),
        _ => None,
    }
}

/// `mv` of display rows as the effect indices to move from and to: the display order runs from the last effect
/// to the first, and `mv.to` counts display rows once the moved row is taken out.
fn effect_move(count: usize, mv: &ReorderMove<usize>) -> Option<(usize, usize)> {
    let &[from] = mv.keys.as_slice() else { return None };
    let to = (count - 1).checked_sub(mv.to)?;
    (to != from).then_some((from, to))
}
