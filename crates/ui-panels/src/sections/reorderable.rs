//! `ReorderableSections` (24.4): a sidebar of sections the user folds, hides, pins and drags, over a `SectionLayout`.

use super::layout::{SectionEvent, SectionKey, SectionLayout};
use super::words::ReorderableWords;
use super::{Section, SectionStack};
use crate::contract::{Bind, BodyCx, Entry, NoValue, Outcome, Pattern, Politeness, Props as _, Show as _, ShowContainer as _, announce};
use crate::display::Divider;
use crate::layout_components::Stack;
use crate::menu::{MenuItem, entry_rows};
use crate::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::{Env, Label};
use craft_behavior::{EditEvent, Mixed};
use egui::{Response, Ui};
use std::cell::RefCell;
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// The commands of a section, run on the section that holds the focus: its header menu rows and any chord the keymap
/// binds. Showing a hidden section is a row of the menu, one per section, because a hidden section cannot hold focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "section", scope = "section")]
#[non_exhaustive]
pub enum SectionCommands {
    /// Hide the section from the sidebar.
    #[command(label = "Hide Section")]
    Hide,
    /// Fold the section shut.
    #[command(label = "Fold Section")]
    Fold,
    /// Open a folded section.
    #[command(label = "Unfold Section")]
    Unfold,
    /// Keep the section above the sections that are not pinned.
    #[command(label = "Pin Section")]
    Pin,
    /// Release a pinned section.
    #[command(label = "Unpin Section")]
    Unpin,
    /// Move the section one place up.
    #[command(label = "Move Section Up")]
    MoveUp,
    /// Move the section one place down.
    #[command(label = "Move Section Down")]
    MoveDown,
}

/// A section the app registers: its key and its title.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SectionSpec {
    /// The section's key.
    pub key: SectionKey,
    /// The title in the section's header and in the menu's list of sections.
    pub title: Label,
}

impl SectionSpec {
    /// The section `key`, titled `title`.
    pub fn new(key: impl Into<SectionKey>, title: impl Into<Label>) -> Self {
        SectionSpec { key: key.into(), title: title.into() }
    }
}

/// What the sections report: the response and what the user did, in order.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the sections' events"]
pub struct ReorderableSectionsOutput {
    /// The sections' response.
    pub response: Response,
    /// What happened this pass. A bound layout already holds them; a given one is the host's to change
    /// ([`SectionLayout::apply`]).
    pub events: Vec<SectionEvent>,
}

impl Outcome for ReorderableSectionsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Sidebar sections the user folds, hides, pins and reorders, over the host's [`SectionLayout`].
///
/// - Layout: the shown sections of the layout, pinned first, as the rows of a `SectionStack`; the layout is made to
///   hold exactly the registered sections first. Folding a section writes the layout; so does every event below.
/// - Header: dragging it, or Alt with Up and Down in the section, moves the section among the others of its group.
///   The context request on a header opens the section menu: Hide, Fold or Unfold, Pin or Unpin, Move Up and Move Down as
///   `SectionCommands` rows that run on this section, then a check item per registered section that shows or hides it.
/// - Body: `body` shows the content of each open section.
/// - Announcement: a move announces the section's new place politely.
#[derive(Pattern)]
pub struct ReorderableSections<'a> {
    label: Label,
    layout: Bind<'a, SectionLayout>,
    sections: &'a [SectionSpec],
}

impl<'a> ReorderableSections<'a> {
    /// The sections `sections`, named `label`, arranged as `layout` says.
    pub fn new(label: impl Into<Label>, layout: Bind<'a, SectionLayout>, sections: &'a [SectionSpec]) -> Self {
        ReorderableSections { label: label.into(), layout, sections }
    }

    /// Shows the sections and fills each open one with `body`.
    pub fn show(self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &SectionKey, &mut BodyCx<'_>)) -> ReorderableSectionsOutput {
        let ReorderableSections { label, layout: mut bind, sections } = self;
        let words = Env::of(ui).words::<ReorderableWords>();
        let keys: Vec<SectionKey> = sections.iter().map(|spec| spec.key.clone()).collect();
        let name: Arc<str> = Arc::from(label.as_str());
        let mut events: Vec<SectionEvent> = Vec::new();
        let shown = Stack::new().fill_width().label(label).show(ui, |ui, root| {
            let stored = bind.reborrow().read(root).and_then(|held| held.same().cloned());
            let mut held = stored.clone().unwrap_or_default();
            held.reconcile(&keys);
            let local: RefCell<Vec<SectionEvent>> = RefCell::new(Vec::new());
            let order: Vec<SectionKey> = held.shown().cloned().collect();
            let stack = SectionStack::new(&order);
            let rows = stack.rows();
            let moves = stack
                .show(ui, |ui, _| {
                    rows.each(ui, |ui, key, slot| {
                        let title = sections
                            .iter()
                            .find(|spec| spec.key == *key)
                            .map_or_else(|| Label::from(key.as_str().to_owned()), |spec| spec.title.clone());
                        let mut open = !held.is_folded(key);
                        let out = Section::new(title)
                            .open(Bind::bound(&mut open))
                            .in_stack(slot)
                            .command_scope(SectionCommands::SCOPE)
                            .controls(|_, cx| section_commands(cx, key, &held, &mut local.borrow_mut()))
                            .menu(|ui, _| section_menu(ui, key, &held, sections, &mut local.borrow_mut()))
                            .show(ui, |ui, cx| body(ui, key, cx))
                            .output;
                        if open == held.is_folded(key) {
                            local.borrow_mut().push(SectionEvent::Folded { key: key.clone(), folded: !open });
                        }
                        out
                    });
                })
                .output
                .moves;
            let mut local = local.into_inner();
            for edit in &moves {
                if let Some(moved) = edit.value()
                    && let Some(event) = held.moved_to(&moved.key, moved.to)
                {
                    local.push(event);
                }
            }
            let mut next = held.clone();
            let mut changed = false;
            for event in &local {
                if next.apply(event) {
                    changed = true;
                    if matches!(event, SectionEvent::Moved { .. }) {
                        announce_place(ui, &words, sections, &next, event);
                    }
                }
            }
            if changed {
                bind.reborrow().write(root, &EditEvent::set(Mixed::Same(stored.unwrap_or_default()), next, Arc::clone(&name), None));
            }
            events.extend(local);
        });
        ReorderableSectionsOutput { response: shown.output, events }
    }
}

/// The commands routed to the section: runs them and publishes their states.
fn section_commands(cx: &mut BodyCx<'_>, key: &SectionKey, held: &SectionLayout, events: &mut Vec<SectionEvent>) {
    while let Some((command, _)) = cx.take_command::<SectionCommands>() {
        events.extend(command_event(command, key, held));
    }
    for (command, enabled) in command_rows(key, held) {
        cx.publish(command, CommandState::new().enabled(enabled));
    }
}

/// The commands of a section's menu with whether each applies now.
fn command_rows(key: &SectionKey, held: &SectionLayout) -> [(SectionCommands, bool); 5] {
    let (folded, pinned) = (held.is_folded(key), held.is_pinned(key));
    [
        (SectionCommands::Hide, true),
        (if folded { SectionCommands::Unfold } else { SectionCommands::Fold }, true),
        (if pinned { SectionCommands::Unpin } else { SectionCommands::Pin }, true),
        (SectionCommands::MoveUp, held.can_move(key, true)),
        (SectionCommands::MoveDown, held.can_move(key, false)),
    ]
}

/// The rows of a section's menu: its commands, then a check item per registered section that shows or hides it.
fn section_menu(ui: &mut Ui, key: &SectionKey, held: &SectionLayout, sections: &[SectionSpec], events: &mut Vec<SectionEvent>) {
    let entries: Vec<Entry<SectionEvent>> = sections
        .iter()
        .map(|spec| {
            let hidden = held.is_hidden(&spec.key);
            let event = if hidden { SectionEvent::Shown(spec.key.clone()) } else { SectionEvent::Hidden(spec.key.clone()) };
            Entry::new(event, spec.title.clone()).checked(!hidden)
        })
        .collect();
    for (command, _) in command_rows(key, held) {
        let _ = MenuItem::command(command).show(ui);
    }
    Divider::horizontal().show(ui);
    if let Some(event) = entry_rows(ui, &entries).and_then(|at| entries.get(at)).and_then(Entry::value) {
        events.push(event.clone());
    }
}

/// The event a command of a section stands for, or `None` when it would change nothing.
fn command_event(command: SectionCommands, key: &SectionKey, held: &SectionLayout) -> Option<SectionEvent> {
    match command {
        SectionCommands::Hide => Some(SectionEvent::Hidden(key.clone())),
        SectionCommands::Fold => Some(SectionEvent::Folded { key: key.clone(), folded: true }),
        SectionCommands::Unfold => Some(SectionEvent::Folded { key: key.clone(), folded: false }),
        SectionCommands::Pin => Some(SectionEvent::Pinned(key.clone())),
        SectionCommands::Unpin => Some(SectionEvent::Unpinned(key.clone())),
        SectionCommands::MoveUp => held.moved_by(key, true, false),
        SectionCommands::MoveDown => held.moved_by(key, false, false),
    }
}

/// Announces the new place of the section a `Moved` event moved, in `layout` as it is after the move.
fn announce_place(ui: &Ui, words: &ReorderableWords, sections: &[SectionSpec], layout: &SectionLayout, event: &SectionEvent) {
    let SectionEvent::Moved { key, .. } = event else { return };
    let Some(position) = layout.shown().position(|shown| shown == key) else { return };
    let title = sections.iter().find(|spec| spec.key == *key).map_or_else(|| key.as_str().to_owned(), |spec| spec.title.as_str().to_owned());
    let (position, count) = (Env::of(ui).format().integer(position as u64 + 1), Env::of(ui).format().integer(layout.shown().count() as u64));
    announce(ui.ctx(), words.moved.fill(&[("title", &title), ("position", &position), ("count", &count)]), Politeness::Polite);
}
