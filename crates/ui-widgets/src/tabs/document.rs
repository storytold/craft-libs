//! `DocumentTabs`: the tab strip of one document group. Tabs reorder within the strip, drop into another group's
//! strip through the drag session, and tear off when released over no strip; the host areas at both ends of the strip
//! are the caller's closures.

#[cfg(feature = "testing")]
mod stories;

use super::{Tab, TabStrip};
use crate::contract::{CollectionEvent, ContextMenuRequest, ReorderRule, ShowCollection as _, ShowContainer as _};
use crate::layout_components::Row;
use crate::runtime::env::Label;
use craft_behavior::{EditEvent, SelectionChange, keyed};
use egui::{Pos2, Response, Ui};
use std::borrow::Cow;
use std::hash::Hash;

/// A document as its tab shows it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DocumentTab<D> {
    /// Identifies the document in events.
    pub id: D,
    /// The tab's text.
    pub label: Label,
    /// Unsaved changes: the tab shows the modified mark.
    pub modified: bool,
    /// The tab shows a close button.
    pub closable: bool,
}

impl<D> DocumentTab<D> {
    /// A closable, unmodified tab for document `id` named `label`.
    pub fn new(id: D, label: impl Into<Label>) -> Self {
        DocumentTab { id, label: label.into(), modified: false, closable: true }
    }

    /// Unsaved changes.
    #[must_use]
    pub fn modified(mut self, on: bool) -> Self {
        self.modified = on;
        self
    }

    /// Whether the tab can be closed.
    #[must_use]
    pub fn closable(mut self, on: bool) -> Self {
        self.closable = on;
        self
    }
}

/// What the tabs of a group did this pass, in the order they happened.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DocumentTabsEvent<D> {
    /// The tab was selected: a click or an arrow key.
    Selected(D),
    /// The tab was activated: Enter, or a double click.
    Activated(D),
    /// The tab's close button was pressed, or the tab was middle-clicked.
    CloseRequested(D),
    /// The tab was picked in the overflow menu: the host shows it.
    Picked(D),
    /// The tabs moved within the strip: now before `before`, or last when `None`.
    #[non_exhaustive]
    Reordered {
        /// The moved documents in display order.
        documents: Vec<D>,
        /// The tab they now sit before.
        before: Option<D>,
    },
    /// Tabs of another group dropped on this strip, on the tab `over`.
    #[non_exhaustive]
    DroppedInto {
        /// The dropped documents in the order they had.
        documents: Vec<D>,
        /// The tab the drop landed on.
        over: D,
    },
    /// The tab was dragged out and released over no strip: float it at `at`.
    #[non_exhaustive]
    TornOff {
        /// The documents the drag carried.
        documents: Vec<D>,
        /// The pointer when the drag was released, in screen points.
        at: Pos2,
    },
    /// The tab's context menu is asked for.
    ContextMenu(ContextMenuRequest<D>),
}

/// What `DocumentTabs::show` returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the group's events"]
pub struct DocumentTabsOutput<D> {
    /// The strip's response.
    pub response: Response,
    /// What happened, in order.
    pub events: Vec<DocumentTabsEvent<D>>,
}

type Area<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// The tab strip of one document group. Give the strips of all groups the same `accepts` rule (the default takes
/// every group) and a tab dropped from one reaches the other as `DroppedInto`.
#[must_use = "show the tabs with `show`"]
#[derive(crate::contract::Pattern)]
#[pattern(probe(DocumentTabs<'static, usize>))]
pub struct DocumentTabs<'a, D: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> {
    label: Label,
    tabs: &'a [DocumentTab<D>],
    selected: Option<D>,
    reorderable: bool,
    tear_off: bool,
    leading: Option<Area<'a>>,
    trailing: Option<Area<'a>>,
}

impl<'a, D: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> DocumentTabs<'a, D> {
    /// The tabs of a group named `label`, in the host's order.
    pub fn new(label: impl Into<Label>, tabs: &'a [DocumentTab<D>]) -> Self {
        DocumentTabs { label: label.into(), tabs, selected: None, reorderable: false, tear_off: false, leading: None, trailing: None }
    }

    /// The document the group shows.
    pub fn selected(mut self, document: D) -> Self {
        self.selected = Some(document);
        self
    }

    /// Tabs reorder by drag and by Alt+Left and Alt+Right.
    pub fn reorderable(mut self, on: bool) -> Self {
        self.reorderable = on;
        self
    }

    /// A tab released over no strip is reported as `TornOff`.
    pub fn tear_off(mut self, on: bool) -> Self {
        self.tear_off = on;
        self
    }

    /// Host content before the tabs, at the reading start, as wide as it measures.
    pub fn leading(mut self, area: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.leading = Some(Box::new(area));
        self
    }

    /// Host content after the tabs, at the reading end, as wide as it measures.
    pub fn trailing(mut self, area: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.trailing = Some(Box::new(area));
        self
    }

    /// Shows the group's strip.
    pub fn show(self, ui: &mut Ui) -> DocumentTabsOutput<D> {
        let DocumentTabs { label, tabs, selected, reorderable, tear_off, leading, trailing } = self;
        let source = keyed(tabs, |tab: &DocumentTab<D>| tab.id.clone()).with_text(|tab| Cow::Borrowed(tab.label.as_str()));
        let mut strip = TabStrip::new(label);
        if tear_off {
            strip = strip.tear_off();
        }
        if let Some(document) = selected {
            strip = strip.shown(document);
        }
        if reorderable {
            strip = strip.reorder(ReorderRule::new());
        }
        let show_strip = |ui: &mut Ui, strip: TabStrip<'_, D>| {
            strip.show_items(ui, &source, &mut |key, _| {
                let document = tabs.iter().find(|tab| &tab.id == key);
                let tab = Tab::new(document.map_or_else(Label::default, |tab| tab.label.clone())).closable(document.is_some_and(|tab| tab.closable));
                if document.is_some_and(|tab| tab.modified) { tab.modified() } else { tab }
            })
        };
        let shown = if leading.is_none() && trailing.is_none() {
            Ok(show_strip(ui, strip))
        } else {
            let (mut strip, mut leading, mut trailing) = (Some(strip), leading, trailing);
            let mut shown = None;
            let row = Row::new().show(ui, |ui, _| {
                if let Some(area) = leading.take() {
                    area(ui);
                }
                if let Some(strip) = strip.take() {
                    shown = Some(show_strip(ui, strip));
                }
                if let Some(area) = trailing.take() {
                    area(ui);
                }
            });
            shown.ok_or(row.output)
        };
        let shown = match shown {
            Ok(shown) => shown,
            Err(response) => return DocumentTabsOutput { response, events: Vec::new() },
        };
        let mut events = Vec::new();
        let response = shown.response.response.clone();
        for event in shown.response.events {
            match event {
                CollectionEvent::Selection(SelectionChange::Set(keys)) => events.extend(keys.last().cloned().map(DocumentTabsEvent::Selected)),
                CollectionEvent::Activate(key) => events.push(DocumentTabsEvent::Activated(key)),
                CollectionEvent::Reorder(edit) => {
                    if let (Some(moved), true) = (edit.value(), matches!(edit, EditEvent::Set { .. } | EditEvent::Commit { .. })) {
                        let remaining: Vec<&D> = tabs.iter().map(|tab| &tab.id).filter(|id| !moved.keys.contains(id)).collect();
                        events.push(DocumentTabsEvent::Reordered {
                            documents: moved.keys.clone(),
                            before: remaining.get(moved.to).map(|id| (*id).clone()),
                        });
                    }
                }
                CollectionEvent::Dropped { key: Some(key), keys, .. } => events.push(DocumentTabsEvent::DroppedInto { documents: keys, over: key }),
                _ => {}
            }
        }
        events.extend(shown.closed.into_iter().map(DocumentTabsEvent::CloseRequested));
        events.extend(shown.menu.map(DocumentTabsEvent::ContextMenu));
        events.extend(shown.picked.map(DocumentTabsEvent::Picked));
        events.extend(shown.torn_off.map(|torn| DocumentTabsEvent::TornOff { documents: torn.keys, at: torn.pointer }));
        DocumentTabsOutput { response, events }
    }
}
