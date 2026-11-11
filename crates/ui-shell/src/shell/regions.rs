//! The slot API of the window skeleton: one bar or rail, its key, size and reveal.

use crate::contract::PaneEdge;
use crate::runtime::env::{Label, ShellRegions, SlotKey};
use crate::style::{StyleCx, UiTokens};
use egui::{Rect, pos2};

/// The size of a slot along its edge: the height of a top or bottom slot, the width of a start or end slot. Each names a
/// size of the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SlotSize {
    /// `size.bar.title`.
    TitleBar,
    /// `size.bar.tab`.
    TabBar,
    /// `size.bar.status`.
    StatusBar,
    /// `size.bar.options`.
    OptionsBar,
    /// `size.bar.options_narrow`: the options bar of a window too small for the full bar.
    OptionsBarNarrow,
    /// `size.rail.width`: one column of a rail; [`Slot::columns`] says how many columns the slot lays out.
    RailWidth,
}

impl SlotSize {
    /// The size in logical pixels under the tokens of `cx`.
    pub fn px(self, cx: &StyleCx<'_>) -> f32 {
        self.resolve(cx.tokens)
    }

    /// The size in logical pixels under `tokens`.
    pub(crate) fn resolve(self, tokens: &UiTokens) -> f32 {
        let size = &tokens.size;
        match self {
            SlotSize::TitleBar => size.bar.title,
            SlotSize::TabBar => size.bar.tab,
            SlotSize::StatusBar => size.bar.status,
            SlotSize::OptionsBar => size.bar.options,
            SlotSize::OptionsBarNarrow => size.bar.options_narrow,
            SlotSize::RailWidth => size.rail.width,
        }
    }
}

/// One bar or rail of the skeleton: its key, whether it shows, whether it shows in a reveal, its size and its AccessKit
/// name. The slot's body draws its own surface and edge.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Slot {
    key: SlotKey,
    size: Option<SlotSize>,
    columns: usize,
    visible: bool,
    reveal: bool,
    label: Option<Label>,
}

impl Slot {
    /// A slot with a key that is stable and unique in the window. It is visible and does not show in a reveal. Without a
    /// [`size`](Self::size) it takes the default of its edge: the title bar, options bar, status bar or rail width.
    pub fn new(key: SlotKey) -> Self {
        Self { key, size: None, columns: 1, visible: true, reveal: false, label: None }
    }

    /// The size along the slot's edge, in place of the default of the edge.
    pub fn size(mut self, size: SlotSize) -> Self {
        self.size = Some(size);
        self
    }

    /// The number of rail columns of a start or end slot, clamped to 1 and 2. The slot keeps [`SlotSize::RailWidth`] per
    /// column and the shell lays the columns out side by side, so the slot's extent is its columns wide.
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns.clamp(1, 2);
        self
    }

    /// Whether the slot shows in normal mode: the host's flag, passed every frame. A slot that is not visible is not
    /// built.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// The slot shows during a reveal in body-only mode, whatever its `visible` flag.
    pub fn reveal(self) -> Self {
        self.reveal_if(true)
    }

    /// [`reveal`](Self::reveal) when `reveal` is true. A title slot passes the host's menu bar flag, so it reveals only
    /// when the menu bar is on.
    pub fn reveal_if(mut self, reveal: bool) -> Self {
        self.reveal = reveal;
        self
    }

    /// The name of the slot's AccessKit region ("Options", "Tools", "Status").
    pub fn label(mut self, label: impl Into<Label>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The slot's key, as visibility flags and [`ShellCx::slot_visible`](crate::runtime::env::ShellCx::slot_visible) name it.
    pub fn key(&self) -> SlotKey {
        self.key
    }

    /// The size set by [`size`](Self::size), if any.
    pub fn slot_size(&self) -> Option<SlotSize> {
        self.size
    }

    /// The number of rail columns, 1 or 2.
    pub fn column_count(&self) -> usize {
        self.columns
    }

    /// The host's visibility flag for normal mode.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Whether the slot shows during a reveal in body-only mode.
    pub fn reveals(&self) -> bool {
        self.reveal
    }

    /// The name of the slot's AccessKit region, if set.
    pub fn name(&self) -> Option<&Label> {
        self.label.as_ref()
    }
}

/// One slot's claim on the window's room: where it sits, how far it reaches inward, and whether it covers the body
/// instead of taking room from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Claim {
    pub key: SlotKey,
    pub edge: PaneEdge,
    pub extent: f32,
    pub overlay: bool,
}

/// Lays out `claims` in order around the body in `window`: a top claim from the top edge inward, a bottom claim from the
/// bottom edge inward, a start or end claim from its side inward (mirrored when `rtl`). A claim takes its extent, or the
/// room that is left if that is less. An overlay claim stacks inward from the edge like the others but leaves the body
/// its room, so the body's rect is the same with and without it.
pub(super) fn layout(window: Rect, claims: &[Claim], rtl: bool) -> ShellRegions {
    // `flow` is what the body keeps; `cover` is what overlay claims place themselves in, which every claim shrinks.
    let mut flow = window;
    let mut cover = window;
    let mut slots = Vec::with_capacity(claims.len());
    for claim in claims {
        let (rect, left) = if claim.overlay { split(cover, claim, rtl) } else { split(flow, claim, rtl) };
        if claim.overlay {
            cover = left;
        } else {
            flow = left;
            cover = split(cover, claim, rtl).1;
        }
        slots.push((claim.key, rect));
    }
    ShellRegions { body: flow, slots }
}

/// The strip `claim` takes at its edge of `room`, and what is left.
fn split(room: Rect, claim: &Claim, rtl: bool) -> (Rect, Rect) {
    let Rect { min, max } = room;
    let extent = claim.extent.max(0.0);
    match claim.edge {
        PaneEdge::Top => {
            let y = (min.y + extent).min(max.y);
            (Rect::from_min_max(min, pos2(max.x, y)), Rect::from_min_max(pos2(min.x, y), max))
        }
        PaneEdge::Bottom => {
            let y = (max.y - extent).max(min.y);
            (Rect::from_min_max(pos2(min.x, y), max), Rect::from_min_max(min, pos2(max.x, y)))
        }
        PaneEdge::Start | PaneEdge::End => {
            if (claim.edge == PaneEdge::Start) != rtl {
                let x = (min.x + extent).min(max.x);
                (Rect::from_min_max(min, pos2(x, max.y)), Rect::from_min_max(pos2(x, min.y), max))
            } else {
                let x = (max.x - extent).max(min.x);
                (Rect::from_min_max(pos2(x, min.y), max), Rect::from_min_max(min, pos2(x, max.y)))
            }
        }
        _ => (room, room),
    }
}
