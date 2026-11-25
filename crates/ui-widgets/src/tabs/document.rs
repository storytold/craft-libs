//! `DocumentTabs`: the tab strip of one document group. Tabs reorder within the strip, drop into another group's
//! strip through the drag session, and tear off when released over no strip; the host areas at both ends of the strip
//! are the caller's closures. Files from the OS drop between the tabs when the host takes them.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::{Tab, TabStrip};
use crate::contract::{
    CollectionEvent, ContextMenuRequest, DropAt, DropKind, DropPlace, ListDrag, OsFiles, Props as _, ReorderRule, ShowCollection as _,
    ShowContainer as _, Sizing,
};
use crate::layout_components::Row;
use crate::runtime::drag::OsFile;
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
    /// The length in bytes of the label's name part (the document's name before its zoom and colour mode): the
    /// modified mark follows the name, not the whole label. `None`: the label is all name.
    pub name_len: Option<usize>,
    /// The tab shows a close button.
    pub closable: bool,
    /// How far the document has opened, between 0 and 1; `None` for a document that is open.
    pub progress: Option<f32>,
    /// A second line of text the tab shows, such as the opening step.
    pub detail: Option<Label>,
    /// The accessible name of the close button, for a tab whose close cancels the opening.
    pub close_label: Option<Label>,
    /// Hover text; `DocumentTabs::show` passes it as the tab's tooltip. A tab whose label is cut shows the full label, then this text on the next line.
    pub tooltip: Option<Label>,
    /// The tab can be dragged and key-moved, and other tabs can be dropped past it (default true).
    pub movable: bool,
}

impl<D> DocumentTab<D> {
    /// A closable, unmodified tab for document `id` named `label`.
    pub fn new(id: D, label: impl Into<Label>) -> Self {
        DocumentTab {
            id,
            label: label.into(),
            modified: false,
            name_len: None,
            closable: true,
            progress: None,
            detail: None,
            close_label: None,
            tooltip: None,
            movable: true,
        }
    }

    /// Unsaved changes.
    #[must_use]
    pub fn modified(mut self, on: bool) -> Self {
        self.modified = on;
        self
    }

    /// The label's name part: its first `len` bytes, which the modified mark follows.
    #[must_use]
    pub fn name_len(mut self, len: usize) -> Self {
        self.name_len = Some(len);
        self
    }

    /// Whether the tab can be closed.
    #[must_use]
    pub fn closable(mut self, on: bool) -> Self {
        self.closable = on;
        self
    }

    /// How far the document has opened; a tab with progress is a document still opening, and its close cancels.
    #[must_use]
    pub fn progress(mut self, fraction: Option<f32>) -> Self {
        self.progress = fraction;
        self
    }

    /// A second line of text the tab shows.
    #[must_use]
    pub fn detail(mut self, text: impl Into<Label>) -> Self {
        self.detail = Some(text.into());
        self
    }

    /// The accessible name of the close button.
    #[must_use]
    pub fn close_label(mut self, label: impl Into<Label>) -> Self {
        self.close_label = Some(label.into());
        self
    }

    /// Whether the tab can be moved: dragged or key-moved, with no other tab dropped past it.
    #[must_use]
    pub fn movable(mut self, on: bool) -> Self {
        self.movable = on;
        self
    }

    /// Hover text for the tab.
    #[must_use]
    pub fn tooltip(mut self, text: impl Into<Label>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
}

/// Whether moving `moved` to `to`, a position among the tabs left once the moved ones are taken out, leaves every tab
/// that does not move at the place it holds: no unmovable tab is moved, and none is passed by a moved one.
fn keeps_fixed<D: PartialEq>(tabs: &[DocumentTab<D>], moved: &[D], to: usize) -> bool {
    if tabs.iter().all(|tab| tab.movable) {
        return true;
    }
    if tabs.iter().any(|tab| !tab.movable && moved.contains(&tab.id)) {
        return false;
    }
    let mut order: Vec<&D> = tabs.iter().map(|tab| &tab.id).filter(|id| !moved.contains(id)).collect();
    let at = to.min(order.len());
    for (n, id) in moved.iter().enumerate() {
        order.insert(at + n, id);
    }
    tabs.iter().enumerate().filter(|(_, tab)| !tab.movable).all(|(place, tab)| order.get(place) == Some(&&tab.id))
}

/// What the tabs of a group did this pass, in the order they happened.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DocumentTabsEvent<D> {
    /// The tab was selected: a click, Space or Enter, or a drag of another payload resting over the tab.
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
    /// Tabs of another group dropped on this strip, on the tab `over` or on none.
    #[non_exhaustive]
    DroppedInto {
        /// The dropped documents in the order they had.
        documents: Vec<D>,
        /// The tab the drop landed on; `None` in the room after the tabs, or on a strip with no tabs.
        over: Option<D>,
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
    /// Files dropped from the operating system on the strip, before the tab `before` (last when `None`).
    #[non_exhaustive]
    FilesDropped {
        /// The dropped files.
        files: Vec<OsFile>,
        /// The tab they go before.
        before: Option<D>,
    },
    /// The strip's empty room after the tabs was double-clicked: the host opens a new document there.
    EmptyDoubleClick,
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
    /// The tab under the pointer this pass, also while another component holds the press: the Move tool's transfer reads it.
    pub hovered: Option<D>,
}

type Area<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

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
    large: bool,
    takes_files: bool,
    leading: Option<Area<'a>>,
    trailing: Option<Area<'a>>,
}

impl<'a, D: Clone + Eq + Hash + Send + Sync + std::fmt::Debug + 'static> DocumentTabs<'a, D> {
    /// The tabs of a group named `label`, in the host's order.
    pub fn new(label: impl Into<Label>, tabs: &'a [DocumentTab<D>]) -> Self {
        DocumentTabs {
            label: label.into(),
            tabs,
            selected: None,
            reorderable: false,
            tear_off: false,
            large: false,
            takes_files: false,
            leading: None,
            trailing: None,
        }
    }

    /// The document the group shows.
    pub fn selected(mut self, document: D) -> Self {
        self.selected = Some(document);
        self
    }

    /// Tabs reorder by drag and by Cmd+] and Cmd+[ (Ctrl off macOS; Shift to the ends).
    pub fn reorderable(mut self, on: bool) -> Self {
        self.reorderable = on;
        self
    }

    /// A tab released over no strip is reported as `TornOff`.
    pub fn tear_off(mut self, on: bool) -> Self {
        self.tear_off = on;
        self
    }

    /// Every tab takes the strip's taller form.
    pub fn large(mut self, on: bool) -> Self {
        self.large = on;
        self
    }

    /// The strip takes files dragged from the operating system and reports them as `FilesDropped`.
    pub fn takes_files(mut self, on: bool) -> Self {
        self.takes_files = on;
        self
    }

    /// Host content before the tabs, at the reading start, as wide as it measures.
    pub fn leading(mut self, area: impl FnMut(&mut Ui) + 'a) -> Self {
        self.leading = Some(Box::new(area));
        self
    }

    /// Host content after the tabs, at the reading end, as wide as it measures.
    pub fn trailing(mut self, area: impl FnMut(&mut Ui) + 'a) -> Self {
        self.trailing = Some(Box::new(area));
        self
    }

    /// Shows the group's strip.
    pub fn show(self, ui: &mut Ui) -> DocumentTabsOutput<D> {
        let DocumentTabs { label, tabs, selected, reorderable, tear_off, leading, trailing, large, takes_files } = self;
        let source = keyed(tabs, |tab: &DocumentTab<D>| tab.id.clone()).with_text(|tab| Cow::Borrowed(tab.label.as_str()));
        let make_strip = || {
            let mut strip = TabStrip::new(label.clone())
                .takes_drop(DropKind::new(DropPlace::Between, |_: &ListDrag<D>, _| true))
                .can_close(move |key: &D| tabs.iter().any(|tab| &tab.id == key && tab.closable));
            if takes_files {
                strip = strip.takes_drop(DropKind::new(DropPlace::Between, |_: &OsFiles, _| true));
            }
            if tear_off {
                strip = strip.tear_off();
            }
            if large {
                strip = strip.large();
            }
            if let Some(document) = selected.clone() {
                strip = strip.shown(document);
            }
            if reorderable {
                strip = strip
                    .reorder(ReorderRule::new())
                    // The held tab is the preview; production shows no filename badge beside the pointer.
                    .drag_preview(crate::contract::RowPreview::None)
                    .can_drag(move |key: &D| tabs.iter().find(|tab| &tab.id == key).is_none_or(|tab| tab.movable))
                    .accepts(move |drag: &ListDrag<D>, _, insert| {
                        // `insert` counts every tab; `keeps_fixed` takes the place among the tabs left without the dragged ones.
                        let passed = tabs.iter().take(insert).filter(|tab| drag.keys.contains(&tab.id)).count();
                        keeps_fixed(tabs, &drag.keys, insert - passed)
                    });
            }
            strip
        };
        let hovered = std::cell::RefCell::new(None);
        let show_strip = |ui: &mut Ui, strip: TabStrip<'_, D>| {
            strip.show_items(ui, &source, &mut |key, cx| {
                if cx.row.pointer_over {
                    *hovered.borrow_mut() = Some(key.clone());
                }
                let Some(document) = tabs.iter().find(|tab| &tab.id == key) else {
                    return Tab::new(Label::default()).closable(false);
                };
                let mut tab = Tab::new(document.label.clone()).closable(document.closable);
                if document.modified {
                    tab = tab.modified();
                }
                if let Some(len) = document.name_len {
                    tab = tab.name_len(len);
                }
                if let Some(fraction) = document.progress {
                    tab = tab.progress(fraction);
                }
                if let Some(detail) = &document.detail {
                    tab = tab.detail(detail.clone());
                }
                if let Some(close) = &document.close_label {
                    tab = tab.close_label(close.clone());
                }
                if let Some(tooltip) = &document.tooltip {
                    tab = tab.tooltip(tooltip.clone());
                }
                tab
            })
        };
        let shown = if leading.is_none() && trailing.is_none() {
            Ok(show_strip(ui, make_strip()))
        } else {
            let (mut leading, mut trailing) = (leading, trailing);
            let mut shown = None;
            // The row measures more than once to reserve non-filling children before sharing the
            // remaining width. Recreate the strip and show both areas on every sweep.
            let row = Row::new().sizing(Sizing::FillWidth).show(ui, |ui, _| {
                if let Some(area) = leading.as_mut() {
                    area(ui);
                }
                let strip = show_strip(ui, make_strip());
                if let Some(area) = trailing.as_mut() {
                    // The strip fills the row less the room the control took last pass; the control follows the last tab
                    // when every tab shows, and is the row's last item when they overflow.
                    let memory = ui.scope_id().with("document-tabs-trailing-width");
                    let held = crate::runtime::memory::with_session(ui.ctx(), memory, |held: &mut f32| *held);
                    // design: allow[contract/allocate](the tabs are a plain `show`, not a component: the slot keeps last pass's room for the trailing area)
                    let slot = ui.allocate_exact_size(egui::vec2(held, strip.response.response.rect.height()), egui::Sense::hover()).0;
                    let all_show = strip.tab_rects.len() == tabs.len();
                    let after = match strip.tab_rects.last() {
                        Some((_, rect)) if all_show => Some(rect.right() + ui.spacing().item_spacing.x),
                        None => Some(strip.response.response.rect.left()),
                        _ => None,
                    };
                    // Overflowing, the control ends at the row's end edge; it never runs past it.
                    let (room, layout) = match after {
                        Some(at) if !ui.layout().prefer_right_to_left() => {
                            let end = slot.right().min(ui.clip_rect().right().max(slot.left()));
                            let left = at.min(slot.left()).min(end - held).max(strip.response.response.rect.left());
                            (
                                egui::Rect::from_min_max(egui::pos2(left, slot.top()), egui::pos2(slot.right().max(left + 400.0), slot.bottom())),
                                egui::Layout::left_to_right(egui::Align::Center),
                            )
                        }
                        _ => {
                            // The row's own edge can lie past what the bar leaves it: end at the visible edge.
                            let end = slot.right().min(ui.clip_rect().right().max(slot.left()));
                            (
                                egui::Rect::from_min_max(egui::pos2(end - slot.width(), slot.top()), egui::pos2(end, slot.bottom())),
                                egui::Layout::right_to_left(egui::Align::Center),
                            )
                        }
                    };
                    let builder = egui::UiBuilder::new().max_rect(room).layout(layout);
                    let drawn = ui.scope_builder(builder, |ui| area(ui)).response.rect.width();
                    if (drawn - held).abs() > 0.5 {
                        crate::runtime::memory::with_session(ui.ctx(), memory, |held: &mut f32| *held = drawn);
                        ui.ctx().request_repaint();
                    }
                }
                shown = Some(strip);
            });
            shown.ok_or(row.output)
        };
        let shown = match shown {
            Ok(shown) => shown,
            Err(response) => return DocumentTabsOutput { response, events: Vec::new(), hovered: None },
        };
        let mut events = Vec::new();
        let response = shown.response.response.clone();
        for event in shown.response.events {
            match event {
                CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                    events.extend(keys.last().cloned().map(DocumentTabsEvent::Selected))
                }
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
                CollectionEvent::Dropped { at: DropAt::Gap { key, insert, .. }, payload, .. } => {
                    // The gap's `key` is the tab under the pointer; `insert` is the display position the files land
                    // before, so the tab at `insert` is the one they go before.
                    if let Some(files) = payload.get::<OsFiles>() {
                        events
                            .push(DocumentTabsEvent::FilesDropped { files: files.files.clone(), before: tabs.get(insert).map(|tab| tab.id.clone()) });
                    } else if let Some(drag) = payload.get::<ListDrag<D>>() {
                        events.push(DocumentTabsEvent::DroppedInto { documents: drag.keys.clone(), over: key });
                    }
                }
                _ => {}
            }
        }
        events.extend(shown.closed.into_iter().map(DocumentTabsEvent::CloseRequested));
        events.extend(shown.menu.map(DocumentTabsEvent::ContextMenu));
        events.extend(shown.picked.map(DocumentTabsEvent::Picked));
        events.extend(shown.torn_off.map(|torn| DocumentTabsEvent::TornOff { documents: torn.keys, at: torn.pointer }));
        if shown.empty_double {
            events.push(DocumentTabsEvent::EmptyDoubleClick);
        }
        DocumentTabsOutput { response, events, hovered: hovered.into_inner() }
    }
}
