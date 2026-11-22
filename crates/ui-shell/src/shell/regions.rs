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
    /// `size.rail.width`: the default extent of a start or end slot.
    RailWidth,
    /// As tall (a top or bottom slot) or as wide (a start or end slot) as its content. The window is laid out at the
    /// extent the content took the last time the slot showed (the edge's default until it first shows), while the
    /// content runs in the room the slot can grow into, from its outer edge to the far edge of what the slots before it
    /// leave: it measures at its own extent, and when that differs the shell lays the window out again in the same
    /// frame. The slot stacks its content from its outer edge instead of centring one line in it, so content of more
    /// than one line (a ribbon: a tab strip over a group row) gets its own height and the slot's full width.
    ///
    /// The slot never takes more than the room the window leaves it: content larger than that, unless it scrolls,
    /// is cut at the window's edge. Raw egui content allocates its own extent along the slot's edge; content that takes whatever room it is
    /// offered (`ui.available_size()`) takes the whole room the slot can grow into. Outside a shell it resolves to
    /// `size.bar.options`.
    Content,
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
            SlotSize::OptionsBar | SlotSize::Content => size.bar.options,
            SlotSize::OptionsBarNarrow => size.bar.options_narrow,
            SlotSize::RailWidth => size.rail.width,
        }
    }
}

/// One bar or rail of the skeleton: its key, whether it shows, whether it shows in a reveal, its size and its AccessKit
/// name. The shell paints the chrome surface over the whole slot and a divider
/// along its edge toward the body; the title slot's bar paints its own.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Slot {
    key: SlotKey,
    size: Option<SlotSize>,
    visible: bool,
    reveal: bool,
    label: Option<Label>,
    screen_bar: bool,
    drops: bool,
}

impl Slot {
    /// A slot with a key that is stable and unique in the window. It is visible and does not show in a reveal. Without a
    /// [`size`](Self::size) it takes the default of its edge: the title bar, options bar, status bar or rail width.
    pub fn new(key: SlotKey) -> Self {
        Self { key, size: None, visible: true, reveal: false, label: None, screen_bar: false, drops: false }
    }

    /// The size along the slot's edge, in place of the default of the edge.
    pub fn size(mut self, size: SlotSize) -> Self {
        self.size = Some(size);
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

    /// Marks the slot as the screen bar: it shows in Focus and Immersive, and only there, whatever its `visible` flag,
    /// and takes its room from the body although those modes show the body only. It needs no
    /// [`reveal`](Self::reveal).
    pub fn screen_bar(mut self) -> Self {
        self.screen_bar = true;
        self
    }

    /// Marks the slot as the drop-down slot of Focus: in Focus it shows while the shell's `dropped` flag is set and
    /// covers the body, leaving the body's rect as it is. A press in the body or a mode change clears the flag.
    pub fn drops(mut self) -> Self {
        self.drops = true;
        self
    }

    /// Whether the slot is the screen bar ([`screen_bar`](Self::screen_bar)).
    pub fn is_screen_bar(&self) -> bool {
        self.screen_bar
    }

    /// Whether the slot is the drop-down slot of Focus ([`drops`](Self::drops)).
    pub fn drops_down(&self) -> bool {
        self.drops
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
