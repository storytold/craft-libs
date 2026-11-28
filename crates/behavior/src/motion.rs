//! The motion level an animation runs at.

/// How much motion the interface shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MotionMode {
    /// Every animation runs.
    #[default]
    Full,
    /// Travel and loops are replaced by short fades.
    Reduced,
    /// Nothing animates.
    Off,
}

/// What an animation is: the kind picks its row of the motion table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MotionKind {
    /// Colour and opacity.
    Fade,
    /// Position, size, scroll and zoom.
    Travel,
    /// A spinner, marching ants or a pulse.
    Loop,
    /// A text caret.
    Blink,
}

/// What an animation does under a mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Playback {
    /// Runs for its token's duration, or for ever when it loops.
    Run,
    /// Jumps to its end value.
    Snap,
    /// A looping animation paints its still frame.
    Still,
    /// A blinking caret stays visible.
    Solid,
}

impl MotionMode {
    /// What an animation of `kind` does at this mode (the motion table, 13.2).
    pub const fn playback(self, kind: MotionKind) -> Playback {
        match (kind, self) {
            (MotionKind::Fade, MotionMode::Off) | (MotionKind::Travel, MotionMode::Reduced | MotionMode::Off) => Playback::Snap,
            (MotionKind::Fade | MotionKind::Travel, _) => Playback::Run,
            (MotionKind::Loop, MotionMode::Full) | (MotionKind::Blink, MotionMode::Full | MotionMode::Reduced) => Playback::Run,
            (MotionKind::Loop, _) => Playback::Still,
            (MotionKind::Blink, MotionMode::Off) => Playback::Solid,
        }
    }

    /// The lower of two modes: a scope can lower the motion mode, never raise it.
    pub const fn lowered_by(self, scope: MotionMode) -> MotionMode {
        if scope.level() > self.level() { scope } else { self }
    }

    const fn level(self) -> u8 {
        match self {
            MotionMode::Full => 0,
            MotionMode::Reduced => 1,
            MotionMode::Off => 2,
        }
    }
}
