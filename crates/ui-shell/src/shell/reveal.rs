//! What a window hides and reveals: the chrome toggles, the Tab reveal and the edge reveal of body-only mode.

use crate::contract::PaneEdge;
use crate::runtime::env::RevealEdge;
use crate::runtime::frame::ChromeToggle;
use crate::shell::Slot;
use egui::{Pos2, Rect, pos2};

/// The width of the strip along a window edge where the pointer reveals the edge's hidden slots.
pub(crate) const EDGE_REVEAL_STRIP: f32 = 8.0;

/// The reveal state of one window.
///
/// - `hidden`: the user hid every panel, tool and bar in normal mode (Tab); the window then behaves as in body-only
///   mode, and Tab shows them again.
/// - `panels_hidden`: the panels are hidden and the tools and bars stay (Shift+Tab).
/// - `tab`: in body-only mode, Tab shows the slots built to reveal until the next Tab.
/// - `edges`: in body-only mode the pointer at the top, start or end edge shows the slots of that edge.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Reveal {
    hidden: bool,
    panels_hidden: bool,
    tab: bool,
    edges: [bool; 3],
    shown: Vec<(RevealEdge, Rect)>,
}

/// What the reveal depends on in this pass.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tracking {
    /// The window shows the body only.
    pub(crate) only: bool,
    /// The reveal works at all; off for a presentation.
    pub(crate) allow_reveal: bool,
    /// The pointer at an edge reveals (the Auto-Show Hidden Panels preference).
    pub(crate) edge_reveal: bool,
}

/// Where the pointer is; the strips it reveals from are [`EDGE_REVEAL_STRIP`] wide.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Aim {
    /// The pointer position in the window.
    pub(crate) pointer: Option<Pos2>,
    /// A button is held or a popup is open.
    pub(crate) frozen: bool,
    /// The shell's rect.
    pub(crate) window: Rect,
    /// Start and end are mirrored.
    pub(crate) rtl: bool,
}

const fn at(edge: RevealEdge) -> usize {
    match edge {
        RevealEdge::Top => 0,
        RevealEdge::Start => 1,
        _ => 2,
    }
}

impl Tracking {
    /// The reveal's inputs for one pass.
    pub(crate) const fn new(only: bool, allow_reveal: bool, edge_reveal: bool) -> Self {
        Tracking { only, allow_reveal, edge_reveal }
    }
}

impl Aim {
    /// The pointer and the strips for one pass.
    pub(crate) const fn new(pointer: Option<Pos2>, frozen: bool, window: Rect, rtl: bool) -> Self {
        Aim { pointer, frozen, window, rtl }
    }
}

const EDGES: [RevealEdge; 3] = [RevealEdge::Top, RevealEdge::Start, RevealEdge::End];

impl Reveal {
    /// Applies the toggles the window's shell took from the frame state in this pass. `host_only` says the screen mode
    /// shows the body only; `allow_reveal` is off for a presentation.
    pub(crate) fn apply(&mut self, toggles: &[ChromeToggle], host_only: bool, allow_reveal: bool) {
        for toggle in toggles {
            self.toggle(*toggle, host_only, allow_reveal);
        }
    }

    fn toggle(&mut self, toggle: ChromeToggle, host_only: bool, allow_reveal: bool) {
        match toggle {
            ChromeToggle::Panels if host_only => {
                if allow_reveal {
                    self.tab = !self.tab;
                    if self.tab {
                        self.edges = [false; 3];
                    }
                }
            }
            ChromeToggle::Panels => {
                self.hidden = !(self.hidden || self.panels_hidden);
                self.panels_hidden = false;
                self.end_reveal();
            }
            ChromeToggle::PanelsKeepTools if !host_only && !self.hidden => self.panels_hidden = !self.panels_hidden,
            _ => {}
        }
    }

    /// Ends the Tab and edge reveal and the user's hiding: every screen-mode change does.
    pub(crate) fn clear(&mut self) {
        self.hidden = false;
        self.panels_hidden = false;
        self.end_reveal();
    }

    fn end_reveal(&mut self) {
        self.tab = false;
        self.edges = [false; 3];
    }

    /// Whether the window shows the body only: the screen mode says so or the user hid the chrome.
    pub(crate) fn only(&self, host_only: bool) -> bool {
        host_only || self.hidden
    }

    /// Whether the panels are hidden and the tools stay.
    pub(crate) fn panels_hidden(&self) -> bool {
        self.panels_hidden && !self.hidden
    }

    /// Whether the Tab reveal is on.
    pub(crate) fn tab(&self) -> bool {
        self.tab
    }

    /// Whether the edge reveal of `edge` is on.
    pub(crate) fn edge(&self, edge: RevealEdge) -> bool {
        self.edges[at(edge)]
    }

    /// Updates the reveal at the start of a pass. A held button or an open popup freezes the edge reveal: it neither
    /// starts nor ends.
    pub(crate) fn track(&mut self, mode: Tracking, aim: Aim) {
        if !mode.only || !mode.allow_reveal {
            self.end_reveal();
            return;
        }
        if self.tab || !mode.edge_reveal {
            self.edges = [false; 3];
            return;
        }
        if aim.frozen {
            return;
        }
        let pointer = aim.pointer.filter(|point| aim.window.contains(*point));
        for edge in EDGES {
            let in_zone = pointer.is_some_and(|point| strip(aim.window, edge, aim.rtl).contains(point));
            let over_slot =
                self.edges[at(edge)] && pointer.is_some_and(|point| self.shown.iter().any(|(shown, rect)| *shown == edge && rect.contains(point)));
            self.edges[at(edge)] = in_zone || over_slot;
        }
    }

    /// Whether `slot` on `edge` shows: its `visible` flag in normal mode; in body-only mode, if it reveals and a Tab
    /// or edge reveal shows it.
    pub(crate) fn shows(&self, slot: &Slot, edge: PaneEdge, only: bool, allow_reveal: bool) -> bool {
        if !only {
            return slot.is_visible();
        }
        allow_reveal && slot.reveals() && (self.tab || edge.reveal_edge().is_some_and(|edge| self.edges[at(edge)]))
    }

    /// Records the rects of the shown slots and the revealed panels the body registered, by the edge they sit on, for
    /// the next pass's hit test.
    pub(crate) fn record(&mut self, shown: Vec<(RevealEdge, Rect)>) {
        self.shown = shown;
    }
}

/// The strip of [`EDGE_REVEAL_STRIP`] along `edge` of `window`.
fn strip(window: Rect, edge: RevealEdge, rtl: bool) -> Rect {
    let zone = EDGE_REVEAL_STRIP;
    match edge {
        RevealEdge::Top => Rect::from_min_max(window.min, pos2(window.max.x, window.min.y + zone)),
        _ => {
            let at_left = (edge == RevealEdge::Start) != rtl;
            if at_left {
                Rect::from_min_max(window.min, pos2(window.min.x + zone, window.max.y))
            } else {
                Rect::from_min_max(pos2(window.max.x - zone, window.min.y), window.max)
            }
        }
    }
}
