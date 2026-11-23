//! `LayerCompsPanel`: the Layer Comps panel, a list of recorded layer states with sweepable
//! option toggles, rename, reorder and the footer actions.

mod body;
mod events;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod words;

pub use events::{LayerCompsEvent, LayerCompsOutput, LayerCompsRowCommands};
pub use words::LayerCompsWords;

use std::cell::RefCell;

use craft_ui::contract::{Bind, Show as _, ShowContainer as _, SweepApply};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::{Env, Label};
use egui::Ui;

/// What a layer comp records of a layer: the options a comp row toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CompOption {
    /// Whether the layers are shown.
    Visibility,
    /// Where the layers are.
    Position,
    /// The layers' appearance (styles, blending).
    Appearance,
}

/// One comp as the panel draws it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LayerCompRow {
    /// The comp's id.
    pub id: u64,
    /// The name.
    pub name: String,
    /// The comment, shown as the row's tooltip.
    pub comment: String,
    /// The comp the document state matches.
    pub applied: bool,
    /// The recorded layers deleted since capture; above 0 the warning shows.
    pub missing: usize,
    /// Whether the comp records visibility.
    pub visibility: bool,
    /// Whether the comp records position.
    pub position: bool,
    /// Whether the comp records appearance.
    pub appearance: bool,
}

impl LayerCompRow {
    /// A row for comp `id` named `name`, recording nothing, applied nowhere.
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self { id, name: name.into(), comment: String::new(), applied: false, missing: 0, visibility: false, position: false, appearance: false }
    }

    /// The comment, the row's tooltip.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = comment.into();
        self
    }

    /// Whether the document state matches this comp.
    pub fn applied(mut self, applied: bool) -> Self {
        self.applied = applied;
        self
    }

    /// The recorded layers deleted since capture.
    pub fn missing(mut self, missing: usize) -> Self {
        self.missing = missing;
        self
    }

    /// Whether the comp records visibility.
    pub fn visibility(mut self, visibility: bool) -> Self {
        self.visibility = visibility;
        self
    }

    /// Whether the comp records position.
    pub fn position(mut self, position: bool) -> Self {
        self.position = position;
        self
    }

    /// Whether the comp records appearance.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }
}

/// The sweeps in progress, one reader per option column: the host keeps one `SweepApply` per column.
#[derive(Clone)]
struct Sweeps {
    visibility: SweepApply,
    position: SweepApply,
    appearance: SweepApply,
}

impl Default for Sweeps {
    fn default() -> Self {
        Sweeps { visibility: SweepApply::new(), position: SweepApply::new(), appearance: SweepApply::new() }
    }
}

impl Sweeps {
    /// The sweep reader of `option`'s column.
    fn of(&mut self, option: CompOption) -> &mut SweepApply {
        match option {
            CompOption::Visibility => &mut self.visibility,
            CompOption::Position => &mut self.position,
            CompOption::Appearance => &mut self.appearance,
        }
    }
}

/// What the panel keeps between passes, in its root slot.
#[derive(Clone, Default)]
struct Session {
    /// The option sweeps in progress.
    sweeps: Sweeps,
    /// The comp the last context request named, whose row menu shows.
    menu: Option<u64>,
}

/// The Layer Comps panel: a list with three sweepable toggle columns, rename, reorder (Last
/// Document State fixed first), a warning trailing part and the footer actions.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct LayerCompsPanel<'a> {
    label: Label,
    rows: &'a [LayerCompRow],
    selected: Bind<'a, Option<u64>>,
    last_state_applied: bool,
    document: Option<u64>,
    empty: Option<Label>,
}

impl<'a> LayerCompsPanel<'a> {
    /// The panel named `label` showing `rows`, with `selected` the selected comp.
    pub fn new(label: impl Into<Label>, rows: &'a [LayerCompRow], selected: Bind<'a, Option<u64>>) -> Self {
        Self { label: label.into(), rows, selected, last_state_applied: false, document: None, empty: None }
    }

    /// Whether the document state is the last document state (the fixed row's mark).
    pub fn last_state_applied(mut self, applied: bool) -> Self {
        self.last_state_applied = applied;
        self
    }

    /// The document the panel shows, so the list's own state is per document.
    pub fn document(mut self, key: impl std::hash::Hash) -> Self {
        use std::hash::Hasher as _;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        self.document = Some(hasher.finish());
        self
    }

    /// The text the list shows with no document.
    pub fn empty(mut self, text: impl Into<Label>) -> Self {
        self.empty = Some(text.into());
        self
    }

    /// Draws the panel over the available rect: the rows, then the footer. Events are reported in that order.
    pub fn show(self, ui: &mut Ui) -> LayerCompsOutput {
        let Self { label, rows, selected, last_state_applied, document, empty } = self;
        if let Some(text) = empty {
            let response = Text::new(text).shade(Shade::Muted).show(ui);
            return LayerCompsOutput { response, events: Vec::new() };
        }
        let env = Env::of(ui);
        let words = env.words::<LayerCompsWords>();
        // Comp ids restart in every document, so the list's own state (rename, scroll) is keyed by the document.
        let list = match document {
            Some(document) => ui.scope_id().with(("layer-comps-rows", document)),
            None => ui.scope_id().with("layer-comps-rows"),
        };
        let cx = body::ListCx { list, label: &label, comps: rows, last_state_applied, words: &words, env: &env };
        let selected = RefCell::new(selected);
        let footer_events = RefCell::new(Vec::new());
        let shown = Stack::new()
            .fill()
            .footer(|ui, stack| {
                let out = body::show_footer(ui, stack, &selected, &cx);
                footer_events.borrow_mut().extend(out);
            })
            .show(ui, |ui, stack| body::show_list(ui, stack, &selected, &cx));
        let mut events = footer_events.into_inner();
        events.extend(shown.inner.unwrap_or_default());
        LayerCompsOutput { response: shown.output, events }
    }
}
