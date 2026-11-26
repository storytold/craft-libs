//! `MarqueeSelect`: the rectangle a drag over list items sweeps and the selection it makes (8.7).

use craft_keymap::Mods;
use emath::{Pos2, Rect};

/// How the swept items combine with the selection the drag started from. Items follow the list rule (Cmd or Ctrl
/// toggles); pixels follow [`SelectionOperation`](crate::geometry::SelectionOperation) (Shift adds, Alt
/// subtracts, both intersect).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarqueeMode {
    /// The swept items become the selection.
    Replace,
    /// The swept items join it.
    Add,
    /// The swept items flip: selected ones leave, the others join.
    Toggle,
    /// The swept items leave it.
    Subtract,
}

impl MarqueeMode {
    /// The mode a press with `mods` starts: Shift adds, Cmd or Ctrl toggles, Alt subtracts, none replaces.
    pub const fn of(mods: Mods) -> MarqueeMode {
        if mods.alt {
            MarqueeMode::Subtract
        } else if mods.cmd || mods.ctrl {
            MarqueeMode::Toggle
        } else if mods.shift {
            MarqueeMode::Add
        } else {
            MarqueeMode::Replace
        }
    }
}

/// A marquee drag in progress. It holds the selection the drag started from, so a cancel restores it.
#[derive(Clone, Debug, PartialEq)]
pub struct MarqueeSelect<K> {
    mode: MarqueeMode,
    base: Vec<K>,
    origin: Pos2,
    pos: Pos2,
}

impl<K: Clone + PartialEq> MarqueeSelect<K> {
    /// A marquee that began at `origin` over `base`, in the mode `mods` pick.
    pub fn begin(origin: Pos2, mods: Mods, base: Vec<K>) -> Self {
        MarqueeSelect { mode: MarqueeMode::of(mods), base, origin, pos: origin }
    }

    /// The mode the drag runs in.
    pub fn mode(&self) -> MarqueeMode {
        self.mode
    }

    /// Moves the free corner to `pos`.
    pub fn update(&mut self, pos: Pos2) {
        self.pos = pos;
    }

    /// The swept rectangle.
    pub fn rect(&self) -> Rect {
        Rect::from_two_pos(self.origin, self.pos)
    }

    /// The selection while the marquee is live, from the items `items` (key and rect) it sweeps.
    pub fn selection(&self, items: impl IntoIterator<Item = (K, Rect)>) -> Vec<K> {
        let marquee = self.rect();
        let swept: Vec<K> = items.into_iter().filter(|(_, rect)| marquee.intersects(*rect)).map(|(key, _)| key).collect();
        match self.mode {
            MarqueeMode::Replace => swept,
            MarqueeMode::Add => {
                let mut keys = self.base.clone();
                keys.extend(swept.into_iter().filter(|key| !self.base.contains(key)));
                keys
            }
            MarqueeMode::Toggle => {
                let mut keys: Vec<K> = self.base.iter().filter(|key| !swept.contains(key)).cloned().collect();
                keys.extend(swept.into_iter().filter(|key| !self.base.contains(key)));
                keys
            }
            MarqueeMode::Subtract => self.base.iter().filter(|key| !swept.contains(key)).cloned().collect(),
        }
    }

    /// Ends the drag with Escape: the selection it started from.
    pub fn cancel(self) -> Vec<K> {
        self.base
    }
}
