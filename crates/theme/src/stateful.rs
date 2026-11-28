//! The state set a style resolves against: flags, the blend of hover, press and on, and `Stateful`, the one
//! per-state type a style field holds. Egui-free; colours blend through `ecolor`.

use crate::parts::{State, States};
use bitflags::bitflags;
use smallvec::SmallVec;

bitflags! {
    /// Value flags of a component or part. `StateSet.flags` is the only source of AccessKit states.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct Flags: u32 {
        /// A selected item.
        const SELECTED = 1 << 0;
        /// A checked control.
        const CHECKED = 1 << 1;
        /// A mixed (indeterminate) control.
        const MIXED = 1 << 2;
        /// The component's own content is open (a disclosure).
        const EXPANDED = 1 << 3;
        /// One of the component's own popups is open.
        const OPEN = 1 << 4;
        /// The bound value fails its check.
        const INVALID = 1 << 5;
        /// The bound value passes with a warning.
        const WARN = 1 << 6;
        /// Work is in progress.
        const BUSY = 1 << 7;
        /// The value cannot be edited.
        const READ_ONLY = 1 << 8;
        /// Holds keyboard focus.
        const FOCUSED = 1 << 9;
        /// Holds keyboard focus and shows its ring.
        const FOCUS_VISIBLE = 1 << 10;
        /// Is being dragged.
        const DRAGGING = 1 << 11;
        /// Is the target of a drag.
        const DROP_TARGET = 1 << 12;
        /// Is drawn with extra weight.
        const EMPHASIZED = 1 << 13;
        /// The lead item of a selection.
        const LEAD = 1 << 14;
        /// The current item of a set: the page on screen, which is neither the lead nor selected.
        const CURRENT = 1 << 15;
    }
}

impl Flags {
    /// Selected, checked or mixed: the flags the `on` axis follows.
    pub const ON: Flags = Flags::SELECTED.union(Flags::CHECKED).union(Flags::MIXED);
}

/// The flags a `Stateful` look needs, all of them set at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlagMask(pub Flags);

impl FlagMask {
    /// A mask of `flags`.
    pub const fn new(flags: Flags) -> Self {
        FlagMask(flags)
    }
}

impl From<Flags> for FlagMask {
    fn from(flags: Flags) -> Self {
        FlagMask(flags)
    }
}

/// Animated 0..1 values. `on` covers selected, checked and mixed.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct StateMix {
    /// Pointer over the part.
    pub hover: f32,
    /// Pointer pressed on the part.
    pub press: f32,
    /// Selected, checked or mixed.
    pub on: f32,
}

impl StateMix {
    /// A mix with every value at rest.
    pub const REST: StateMix = StateMix { hover: 0.0, press: 0.0, on: 0.0 };

    /// A mix of `hover`, `press` and `on`.
    pub const fn new(hover: f32, press: f32, on: f32) -> Self {
        StateMix { hover, press, on }
    }
}

/// The only per-state type a style field may hold.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Stateful<T> {
    /// The looks while off.
    pub off: States<T>,
    /// The looks while on, blended in by `StateMix::on`.
    pub on: Option<States<T>>,
    /// Flag looks in priority order; the first whose flags are all set wins.
    pub when: SmallVec<[(FlagMask, T); 2]>,
}

impl<T: Copy> Stateful<T> {
    /// Every state is `base`.
    pub fn new(base: T) -> Self {
        Stateful { off: States::new(base, base, base, base), on: None, when: SmallVec::new() }
    }

    /// The off looks per state.
    pub fn off(mut self, states: States<T>) -> Self {
        self.off = states;
        self
    }

    /// The on looks per state.
    pub fn on(mut self, states: States<T>) -> Self {
        self.on = Some(states);
        self
    }

    /// Appends a flag look; earlier entries win.
    pub fn when(mut self, mask: impl Into<FlagMask>, value: T) -> Self {
        self.when.push((mask.into(), value));
        self
    }
}

/// Linear blend of two values; `t` is 0..1.
pub trait Mix: Copy {
    /// `self` at `t == 0`, `to` at `t == 1`.
    fn mix(self, to: Self, t: f32) -> Self;
}

impl Mix for f32 {
    fn mix(self, to: f32, t: f32) -> f32 {
        self + (to - self) * t
    }
}

impl Mix for ecolor::Color32 {
    fn mix(self, to: ecolor::Color32, t: f32) -> ecolor::Color32 {
        self.lerp_to_gamma(to, t)
    }
}

/// `None` is "no value of its own" (for ink, the ambient one): a blend with a value switches at the halfway point.
impl<T: Mix> Mix for Option<T> {
    fn mix(self, to: Option<T>, t: f32) -> Option<T> {
        match (self, to) {
            (Some(a), Some(b)) => Some(a.mix(b, t)),
            (a, b) => {
                if t < 0.5 {
                    a
                } else {
                    b
                }
            }
        }
    }
}

/// Picks the value for a state.
pub trait Resolve<T> {
    /// The value for `state` and `flags`, blended by `t`.
    fn resolve(&self, state: State, flags: Flags, t: StateMix) -> T;
}

impl<T: Mix> Resolve<T> for Stateful<T> {
    /// Priority: disabled, then the `when` list in order, then the on axis blended with rest, hover and pressed.
    fn resolve(&self, state: State, flags: Flags, t: StateMix) -> T {
        if state == State::Disabled {
            return match &self.on {
                Some(on) => self.off.disabled.mix(on.disabled, t.on),
                None => self.off.disabled,
            };
        }
        if let Some((_, value)) = self.when.iter().find(|(mask, _)| !mask.0.is_empty() && flags.contains(mask.0)) {
            return *value;
        }
        let pick = |s: &States<T>| s.rest.mix(s.hover, t.hover).mix(s.pressed, t.press);
        let off = pick(&self.off);
        match &self.on {
            Some(on) => off.mix(pick(on), t.on),
            None => off,
        }
    }
}
