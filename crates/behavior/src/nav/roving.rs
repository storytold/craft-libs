//! Roving focus groups and type-ahead matching.

use crate::intent::NavMove;

use super::*;

/// Whether a roving group matches typed letters to entries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Letters {
    /// Letters do nothing.
    #[default]
    None,
    /// Letters build a prefix that picks the next matching entry.
    TypeAhead,
}

/// A roving group's rules: one Tab stop whose members take moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Roving {
    /// The navigation shape.
    pub shape: NavShape,
    /// Whether a move past an end continues at the other end.
    pub wrap: bool,
    /// Whether Home and End move to the ends.
    pub home_end: bool,
    /// Rows a Page key covers, `None` when Page keys are not the group's.
    pub page: Option<usize>,
    /// Whether letters pick entries.
    pub letters: Letters,
    /// The arrows the group takes; the others reach the focused member.
    pub arrows: Arrows,
    /// `WrappedGrid`: the column of the first member when blank cells come before it, `None` when members fill the
    /// rows from the first column.
    pub first_column: Option<usize>,
    /// Whether a move the group cannot make (a step past its first or last row, say) reaches the owning component
    /// instead of being dropped.
    pub hands_back: bool,
    /// Whether the focused member is no lead, so the next move is a first move: forward moves land on the first
    /// enabled member, backward moves on the last.
    pub no_lead: bool,
}

/// Which arrow keys a roving group moves its lead with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Arrows {
    /// Every arrow.
    #[default]
    All,
    /// Left and Right only: Up and Down reach the focused member, as a menu bar title opens its menu with Down.
    Horizontal,
    /// Up and Down only: Left and Right reach the focused member, as a menu moves to its neighbour with them.
    Vertical,
}

impl Roving {
    /// A group of `shape` with Home and End, no wrap, no paging and no letters.
    pub const fn new(shape: NavShape) -> Roving {
        Roving {
            shape,
            wrap: false,
            home_end: true,
            page: None,
            letters: Letters::None,
            arrows: Arrows::All,
            first_column: None,
            hands_back: false,
            no_lead: false,
        }
    }

    /// Whether moves wrap.
    pub const fn wrap(mut self, on: bool) -> Self {
        self.wrap = on;
        self
    }

    /// Whether Home and End move to the ends.
    pub const fn home_end(mut self, on: bool) -> Self {
        self.home_end = on;
        self
    }

    /// Rows a Page key covers.
    pub const fn page(mut self, rows: Option<usize>) -> Self {
        self.page = rows;
        self
    }

    /// Whether letters pick entries.
    pub const fn letters(mut self, letters: Letters) -> Self {
        self.letters = letters;
        self
    }

    /// The arrows the group takes.
    pub const fn arrows(mut self, arrows: Arrows) -> Self {
        self.arrows = arrows;
        self
    }

    /// The column of the first member of a wrapped grid with `column` blank cells before it: Up and Down keep the
    /// column, Home and End go to the ends of the row.
    pub const fn first_column(mut self, column: usize) -> Self {
        self.first_column = Some(column);
        self
    }

    /// A move the group cannot make reaches the owning component, which may page or change what the group shows.
    pub const fn hands_back(mut self) -> Self {
        self.hands_back = true;
        self
    }

    /// Whether the focused member is no lead: the next move starts at an end, as from a group with no lead.
    pub const fn no_lead(mut self, on: bool) -> Self {
        self.no_lead = on;
        self
    }

    /// The state a step over this group starts from, at `lead`, or from no lead when the group has none.
    pub const fn state(&self, lead: Option<usize>) -> NavState {
        let lead = if self.no_lead { None } else { lead };
        let page = match self.page {
            Some(rows) if rows > 0 => rows,
            _ => 10,
        };
        NavState { lead, anchor: None, page, wrap: self.wrap, column: 0 }
    }

    /// Whether the group handles `mv`: Page moves only with a page size, Start and End only with Home and End.
    pub const fn takes(&self, mv: NavMove) -> bool {
        match mv {
            NavMove::PageUp | NavMove::PageDown => self.page.is_some(),
            NavMove::Start | NavMove::End => self.home_end,
            NavMove::Back | NavMove::Forward => !matches!(self.arrows, Arrows::Vertical),
            NavMove::Up | NavMove::Down => !matches!(self.arrows, Arrows::Horizontal),
        }
    }
}

/// Typed letters of the last second as one prefix.
#[derive(Clone, Debug, Default)]
pub struct TypeAhead {
    buffer: String,
    last: f64,
}

impl TypeAhead {
    /// The seconds after which the next letter starts a new prefix.
    const RESET_AFTER: f64 = 1.0;

    /// An empty buffer.
    pub const fn new() -> Self {
        TypeAhead { buffer: String::new(), last: f64::NEG_INFINITY }
    }

    /// Adds `letter` typed at `now` seconds and returns the prefix.
    pub fn push(&mut self, letter: char, now: f64) -> &str {
        if now - self.last > Self::RESET_AFTER {
            self.buffer.clear();
        }
        self.last = now;
        self.buffer.extend(letter.to_lowercase());
        &self.buffer
    }

    /// The entry a prefix picks: the first match from the position after `from` (from `from` itself when the
    /// prefix is one repeated letter, so repeating a letter cycles), wrapping at the end.
    pub fn pick<'a>(prefix: &str, from: Option<usize>, len: usize, label: impl Fn(usize) -> std::borrow::Cow<'a, str>) -> Option<usize> {
        if len == 0 || prefix.is_empty() {
            return None;
        }
        let mut chars = prefix.chars();
        let first = chars.next();
        let repeated = first.is_some_and(|c| prefix.chars().all(|p| p == c)) && prefix.chars().count() > 1;
        let needle: String = if repeated { first.into_iter().collect() } else { prefix.to_owned() };
        let start = match from {
            Some(at) if repeated || prefix.chars().count() == 1 => at + 1,
            Some(at) => at,
            None => 0,
        };
        (0..len).map(|offset| (start + offset) % len).find(|&at| label(at).to_lowercase().starts_with(&needle))
    }
}
