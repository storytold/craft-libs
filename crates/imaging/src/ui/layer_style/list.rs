//! The effect list on the dialog's left.

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, IconId, ItemSource, Selection, SelectionChange};
use craft_ui::contract::{
    ActivateOn, Bind, BodyCx, CollectionEvent, ColumnKind, ColumnSpec, CommandSet, Commands as _, Remember as _, SelectMode, ShowCollection as _,
    Sizing, SpaceAction, SweepApply, SweepStep, ToggleColumn,
};
use craft_ui::list::{List, ListRow, RowEvent, RowInk, ToggleCell, ToggleState, TrailingFace, TrailingPart};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Label, Units};
use egui::Ui;

use super::labels::LayerStyleWords;
use crate::layer_style::{EffectId, EffectKind, EffectSpec, LayerStyleDraft, LayerStylePage};

/// The keys of the effect list for what only the pointer reaches otherwise: `+` and `−` on the lead row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "layerStyle", scope = "layerStyleList")]
#[non_exhaustive]
pub enum LayerStyleCommand {
    /// Another instance of the lead row's effect.
    #[command(label = "Add Another Instance", default = "+")]
    AddInstance,
    /// Removes the lead row's instance while its effect has several.
    #[command(label = "Remove This Instance", default = "-")]
    RemoveInstance,
}

/// What the list asked for in one pass; the dialog applies it to a copy of the draft.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ListAction {
    /// Show this page (a row click, Enter on the lead row).
    Select(LayerStylePage),
    /// An instance's check box: [`LayerStyleDraft::toggle`].
    Toggle(EffectId),
    /// A click on an absent effect's row, or `+`: [`LayerStyleDraft::add_instance`].
    Add(EffectKind),
    /// `−`: [`LayerStyleDraft::remove_instance`].
    Remove(EffectId),
}

/// A row of the list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum EffectRow {
    Styles,
    Blending,
    /// Space between the two page rows and the effects.
    Gap,
    Instance(EffectId),
    /// An effect the layer lacks.
    Absent(EffectKind),
}

impl EffectRow {
    fn page(&self) -> Option<LayerStylePage> {
        match self {
            EffectRow::Styles => Some(LayerStylePage::Styles),
            EffectRow::Blending => Some(LayerStylePage::Blending),
            EffectRow::Instance(id) => Some(LayerStylePage::Effect(*id)),
            EffectRow::Gap | EffectRow::Absent(_) => None,
        }
    }
}

/// The row that holds the lead, kept in the list column's slot.
#[derive(Clone, Default)]
struct Lead(Option<EffectRow>);

const CHECK: ColumnKey = ColumnKey::new("check");
const ADD: TrailingPart = TrailingPart::new(0);
const REMOVE: TrailingPart = TrailingPart::new(1);

/// The rows in display order, with the checks' states.
struct Rows<'a> {
    rows: &'a [EffectRow],
    draft: &'a LayerStyleDraft,
}

impl DisplayOrder<EffectRow> for Rows<'_> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> EffectRow {
        self.rows[at].clone()
    }

    fn is_section(&self, at: usize) -> bool {
        self.rows[at] == EffectRow::Gap
    }
}

impl ItemSource<EffectRow> for Rows<'_> {
    fn toggled(&self, at: usize, _: &ColumnKey) -> bool {
        matches!(&self.rows[at], EffectRow::Instance(id) if self.draft.style.effect(*id).is_some_and(|instance| instance.on))
    }
}

/// The name of instance `id` of `kind`: the effect's name, numbered when the style has several instances of the kind.
fn instance_name(draft: &LayerStyleDraft, words: &LayerStyleWords, units: &Units, kind: EffectKind, id: EffectId) -> Label {
    let name = words.effect(kind);
    let place = draft.style.instances(kind).position(|other| other == id);
    match place {
        Some(place) if draft.style.instances(kind).count() > 1 => {
            let number = units.number((place + 1) as f64, 0..=0);
            Label::from(words.instance_name.fill(&[("effect", name.as_str()), ("number", number.as_str())]))
        }
        _ => name.clone(),
    }
}

/// Draws the list: the Styles and Blending Options rows, a spacer, then one row per instance in `specs` order (several
/// instances of a kind in the style's order) and one dimmed row per kind the layer lacks (a click or Space adds it,
/// turned on).
///
/// Each effect row has a check box in a toggle column, and its name is muted while the effect is off. A kind that
/// allows several has a `+` trailing part and, while it has more than one instance, a `−` before it; both keep the
/// selection, and `+` and `−` on the lead row do the same through [`LayerStyleCommand`]. The list is one Tab stop; ↑, ↓,
/// Home and End move the lead and show the page of a page row, Enter shows the page of any row and Space flips the
/// lead's check box. A kind with several instances numbers them ([`LayerStyleWords::instance_name`]), so no two rows,
/// check boxes or buttons share a name.
pub(super) fn effect_list(
    ui: &mut Ui,
    cx: &mut BodyCx<'_>,
    draft: &LayerStyleDraft,
    specs: &[EffectSpec],
    words: &LayerStyleWords,
    units: &Units,
) -> Option<ListAction> {
    let mut rows = vec![EffectRow::Styles, EffectRow::Blending, EffectRow::Gap];
    for spec in specs {
        let before = rows.len();
        rows.extend(draft.style.instances(spec.kind).map(EffectRow::Instance));
        if rows.len() == before {
            rows.push(EffectRow::Absent(spec.kind));
        }
    }
    let kind_of = |row: &EffectRow| match row {
        EffectRow::Instance(id) => draft.style.effect(*id).map(|instance| instance.kind),
        EffectRow::Absent(kind) => Some(*kind),
        _ => None,
    };
    let shown = rows.iter().find(|row| row.page() == Some(draft.page)).cloned();
    let selection = shown.clone().map_or_else(Selection::default, Selection::single);
    let source = Rows { rows: &rows, draft };
    let columns = vec![ColumnSpec::new(CHECK, Label::Empty, ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::default()))];

    let out = List::<EffectRow>::new(words.effects.clone())
        .select(SelectMode::Single)
        .activate(ActivateOn::Click)
        .allow_empty(false)
        .header(false)
        .space(SpaceAction::ToggleCell)
        .sizing(Sizing::Content)
        .columns(columns)
        .selection(Bind::given(selection))
        .show_items(ui, &source, &mut |key, icx| match key {
            EffectRow::Gap => ListRow::spacer(),
            EffectRow::Styles => ListRow::new(words.styles.clone()).cells(icx.cells()),
            EffectRow::Blending => ListRow::new(words.blending_options.clone()).cells(icx.cells()),
            EffectRow::Absent(kind) => ListRow::new(words.effect(*kind).clone())
                .cells(icx.cells())
                .toggle(ToggleCell::check(CHECK, ToggleState::of(false), words.effect(*kind).clone()))
                .ink(RowInk::Dimmed),
            EffectRow::Instance(id) => {
                let Some(instance) = draft.style.effect(*id) else { return ListRow::new(Label::Empty) };
                let name = instance_name(draft, words, units, instance.kind, *id);
                let mut row = ListRow::new(words.effect(instance.kind).clone())
                    .accessible_name(name.as_str())
                    .cells(icx.cells())
                    .toggle(ToggleCell::check(CHECK, ToggleState::of(instance.on), name.clone()));
                if !instance.on {
                    row = row.ink(RowInk::Muted);
                }
                if instance.kind.allows_several() {
                    if draft.style.instances(instance.kind).count() > 1 {
                        let remove = Label::from(words.remove_instance.fill(&[("effect", name.as_str())]));
                        row = row.trailing_part(REMOVE, TrailingFace::Icon(IconSpec::svg(IconId::new("minus"))), remove);
                    }
                    let add = Label::from(words.add_instance.fill(&[("effect", name.as_str())]));
                    row = row.trailing_part(ADD, TrailingFace::Icon(IconSpec::svg(IconId::new("plus"))), add);
                }
                row
            }
        });

    for event in &out.events {
        if let CollectionEvent::LeadMoved { key, .. } = event {
            cx.keep::<Lead>().0 = Some(key.clone());
        }
    }
    let lead = cx.keep::<Lead>().0.clone().filter(|row| rows.contains(row)).or(shown);

    let mut action = None;
    let mut first = |next: Option<ListAction>| {
        if action.is_none() {
            action = next;
        }
    };
    for event in &out.events {
        match event {
            CollectionEvent::Toggle(edit) => {
                // This column does not sweep: each toggle edit is one press.
                if let Some(SweepStep::Flip(toggle) | SweepStep::Restore(toggle)) = cx.keep::<SweepApply>().feed(edit) {
                    first(match &toggle.key {
                        EffectRow::Instance(id) => Some(ListAction::Toggle(*id)),
                        EffectRow::Absent(kind) => Some(ListAction::Add(*kind)),
                        _ => None,
                    });
                }
            }
            CollectionEvent::Item(row, RowEvent::Trailing { part, .. }) => first(match (row, *part) {
                (EffectRow::Instance(_), ADD) => kind_of(row).map(ListAction::Add),
                (EffectRow::Instance(id), REMOVE) => Some(ListAction::Remove(*id)),
                _ => None,
            }),
            CollectionEvent::Activate(row) => first(match row {
                EffectRow::Absent(kind) => Some(ListAction::Add(*kind)),
                other => other.page().map(ListAction::Select),
            }),
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                first(keys.last().and_then(EffectRow::page).filter(|page| *page != draft.page).map(ListAction::Select));
            }
            _ => {}
        }
    }
    while let Some((command, _)) = cx.take_command::<LayerStyleCommand>() {
        let kind = lead.as_ref().and_then(kind_of).filter(|kind| kind.allows_several());
        first(match (command, &lead) {
            (LayerStyleCommand::AddInstance, _) => kind.map(ListAction::Add),
            (LayerStyleCommand::RemoveInstance, Some(EffectRow::Instance(id))) => {
                kind.filter(|kind| draft.style.instances(*kind).count() > 1).map(|_| ListAction::Remove(*id))
            }
            _ => None,
        });
    }
    action
}
