//! `ReorderableSections` (24.4): a sidebar of sections the user folds, hides, pins and drags, over a `SectionLayout`.

use super::layout::{SectionEvent, SectionKey, SectionLayout};
use super::words::ReorderableWords;
use super::{Section, SectionStack};
use crate::buttons::IconButton;
use crate::contract::{Bind, BodyCx, Entry, NoValue, Outcome, Pattern, Politeness, Props as _, Show as _, ShowContainer as _, announce, clicked};
use crate::display::{Divider, Shade, Text};
use crate::layout_components::Stack;
use crate::menu::{MenuItem, entry_rows};
use crate::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::{Env, Label};
use crate::style::TypeRole;
use craft_behavior::{EditEvent, IconId, Mixed};
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
    /// Put every section back in the usual arrangement: registration order, shown, open and not pinned.
    #[command(label = "Reset Sections")]
    Reset,
}

/// A button at the end of a section's header (Albums' "+" opens Create Album). A press on it never folds or drags
/// the section; it is reported as [`SectionEvent::Button`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SectionButton {
    /// The id reported in `SectionEvent::Button`.
    pub id: &'static str,
    /// The glyph.
    pub icon: IconId,
    /// The tooltip and accessible name.
    pub label: Label,
}

impl SectionButton {
    /// The header button `id`, showing `icon`, named `label`.
    pub fn new(id: &'static str, icon: IconId, label: impl Into<Label>) -> Self {
        SectionButton { id, icon, label: label.into() }
    }
}

/// A section the app registers: its key, its title and what its header and empty body show.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SectionSpec {
    /// The section's key.
    pub key: SectionKey,
    /// The title in the section's header and in the menu's list of sections.
    pub title: Label,
    /// The number of rows in the section: shown at the end of the header while the section is folded.
    pub count: Option<usize>,
    /// What an open section with no rows (`count` of 0) shows ("No albums yet").
    pub empty_hint: Option<Label>,
    /// The button at the end of the header.
    pub button: Option<SectionButton>,
}

impl SectionSpec {
    /// The section `key`, titled `title`, with no count, hint or button.
    pub fn new(key: impl Into<SectionKey>, title: impl Into<Label>) -> Self {
        SectionSpec { key: key.into(), title: title.into(), count: None, empty_hint: None, button: None }
    }

    /// The number of rows in the section.
    #[must_use]
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    /// What the open section shows while it has no rows.
    #[must_use]
    pub fn empty_hint(mut self, hint: impl Into<Label>) -> Self {
        self.empty_hint = Some(hint.into());
        self
    }

    /// The button at the end of the header.
    #[must_use]
    pub fn button(mut self, button: SectionButton) -> Self {
        self.button = Some(button);
        self
    }

    /// Whether the open section shows its empty hint: it has one and no rows.
    fn shows_hint(&self) -> bool {
        self.count == Some(0) && self.empty_hint.is_some()
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
    /// Each header button's response this pass, to anchor the host's popup after a `Button` event.
    pub buttons: Vec<(SectionKey, Response)>,
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
/// - Header: dragging it, or Cmd+] and Cmd+[ in the section (Ctrl off macOS; Shift to the ends), moves the section among
///   the others of its group. At its end, the section's row count in faint caption text while it is folded, then its
///   button ([`SectionSpec::button`]), whose press is reported as `SectionEvent::Button` and never folds or drags.
///   The context request on a header opens the section menu: Hide, Fold or Unfold, Pin or Unpin, Move Up and Move Down as
///   `SectionCommands` rows that run on this section, then a check item per registered section that shows or hides it,
///   then Reset Sections, enabled while the layout is not the usual one.
/// - Body: `body` shows the content of each open section; a section whose count is 0 shows its empty hint first.
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
        let mut buttons: Vec<(SectionKey, Response)> = Vec::new();
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
                        let spec = sections.iter().find(|spec| spec.key == *key);
                        let title = spec.map_or_else(|| Label::from(key.as_str().to_owned()), |spec| spec.title.clone());
                        let mut open = !held.is_folded(key);
                        let out = Section::new(title)
                            .open(Bind::bound(&mut open))
                            .in_stack(slot)
                            .command_scope(SectionCommands::SCOPE)
                            .controls(|ui, cx| {
                                section_commands(cx, key, &held, &keys, &mut local.borrow_mut());
                                if let Some(spec) = spec {
                                    header_end(ui, spec, held.is_folded(key), &mut local.borrow_mut(), &mut buttons);
                                }
                            })
                            .menu(|ui, _| section_menu(ui, key, &held, sections, &mut local.borrow_mut()))
                            .show(ui, |ui, cx| {
                                if let Some(hint) = spec.filter(|spec| spec.shows_hint()).and_then(|spec| spec.empty_hint.clone()) {
                                    Text::new(hint).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
                                }
                                body(ui, key, cx);
                            })
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
        ReorderableSectionsOutput { response: shown.output, events, buttons }
    }
}

/// The commands routed to the section: runs them and publishes their states.
fn section_commands(cx: &mut BodyCx<'_>, key: &SectionKey, held: &SectionLayout, registered: &[SectionKey], events: &mut Vec<SectionEvent>) {
    while let Some((command, _)) = cx.take_command::<SectionCommands>() {
        events.extend(command_event(command, key, held, registered));
    }
    for (command, enabled) in command_rows(key, held) {
        cx.publish(command, CommandState::new().enabled(enabled));
    }
    cx.publish(SectionCommands::Reset, CommandState::new().enabled(!held.is_default(registered)));
}

/// The end of a section's header: the row count while the section is folded, then its button. A container body may
/// run twice in a pass, so a press is reported once and the button's response replaces an earlier one.
fn header_end(ui: &mut Ui, spec: &SectionSpec, folded: bool, events: &mut Vec<SectionEvent>, buttons: &mut Vec<(SectionKey, Response)>) {
    if folded && let Some(count) = spec.count {
        Text::new(Env::of(ui).format().integer(count as u64)).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
    }
    let Some(button) = &spec.button else { return };
    let shown = IconButton::new(button.icon.clone()).id_salt(("section-button", button.id)).label(button.label.clone()).show(ui);
    if clicked(&shown) {
        let event = SectionEvent::Button { key: spec.key.clone(), id: button.id };
        if !events.contains(&event) {
            events.push(event);
        }
    }
    buttons.retain(|(key, _)| *key != spec.key);
    buttons.push((spec.key.clone(), shown.response.clone()));
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
    Divider::horizontal().show(ui);
    let _ = MenuItem::command(SectionCommands::Reset).show(ui);
}

/// The event a command of a section stands for, or `None` when it would change nothing.
fn command_event(command: SectionCommands, key: &SectionKey, held: &SectionLayout, registered: &[SectionKey]) -> Option<SectionEvent> {
    match command {
        SectionCommands::Hide => Some(SectionEvent::Hidden(key.clone())),
        SectionCommands::Fold => Some(SectionEvent::Folded { key: key.clone(), folded: true }),
        SectionCommands::Unfold => Some(SectionEvent::Folded { key: key.clone(), folded: false }),
        SectionCommands::Pin => Some(SectionEvent::Pinned(key.clone())),
        SectionCommands::Unpin => Some(SectionEvent::Unpinned(key.clone())),
        SectionCommands::MoveUp => held.moved_by(key, true, false),
        SectionCommands::MoveDown => held.moved_by(key, false, false),
        SectionCommands::Reset => (!held.is_default(registered)).then(|| SectionEvent::reset(registered.iter().cloned())),
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
