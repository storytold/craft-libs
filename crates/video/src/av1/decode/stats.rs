//! Decode statistics: frame / tile counts and the busy time of every decoding stage.

/// A decoding stage (see [`DecodeStats::stage_secs`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Frame header follow-up: reference setup, motion field estimation, CDF initialisation.
    Setup,
    /// Tile decoding: mode info, coefficients, prediction and reconstruction.
    Tiles,
    /// Deblocking loop filter (7.14).
    LoopFilter,
    /// CDEF (7.15).
    Cdef,
    /// Super-resolution upscaling (7.16).
    Superres,
    /// Loop restoration (7.17).
    Restoration,
    /// Reference update, motion vector storage, output copy and film grain synthesis.
    Output,
}

impl Stage {
    pub const COUNT: usize = 7;
    pub const ALL: [Stage; Stage::COUNT] =
        [Stage::Setup, Stage::Tiles, Stage::LoopFilter, Stage::Cdef, Stage::Superres, Stage::Restoration, Stage::Output];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Setup => "setup",
            Stage::Tiles => "tiles",
            Stage::LoopFilter => "deblock",
            Stage::Cdef => "cdef",
            Stage::Superres => "superres",
            Stage::Restoration => "restoration",
            Stage::Output => "output",
        }
    }
}

/// Counters accumulated by a [`crate::av1::decode::Decoder`]. Stage times are busy time summed over every
/// thread that worked on the stage (wall-clock per piece of work), so with threads they can add
/// up to more than the elapsed time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DecodeStats {
    /// Frames decoded (excluding show_existing_frame repeats).
    pub frames: u64,
    /// Frames decoded in draft mode without in-loop filters.
    pub draft_frames: u64,
    /// Tiles decoded.
    pub tiles: u64,
    /// Busy seconds per [`Stage`] (index with `stage as usize`).
    pub stage_secs: [f64; Stage::COUNT],
}

impl DecodeStats {
    pub fn secs(&self, stage: Stage) -> f64 {
        self.stage_secs[stage as usize]
    }

    pub(crate) fn add(&mut self, stage: Stage, secs: f64) {
        self.stage_secs[stage as usize] += secs;
    }

    pub(crate) fn merge(&mut self, o: &DecodeStats) {
        self.frames += o.frames;
        self.draft_frames += o.draft_frames;
        self.tiles += o.tiles;
        for (a, b) in self.stage_secs.iter_mut().zip(o.stage_secs.iter()) {
            *a += b;
        }
    }

    /// Sum of all stage times.
    pub fn total_secs(&self) -> f64 {
        self.stage_secs.iter().sum()
    }
}

/// A wall-clock stopwatch (a no-op on wasm32, which has no clock in std).
pub(crate) struct Timer {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
}

impl Timer {
    #[inline]
    pub fn start() -> Timer {
        Timer {
            #[cfg(not(target_arch = "wasm32"))]
            start: std::time::Instant::now(),
        }
    }

    /// Seconds since `start` (0 on wasm32).
    #[inline]
    pub fn secs(&self) -> f64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_secs_f64()
        }
        #[cfg(target_arch = "wasm32")]
        {
            0.0
        }
    }

    /// Seconds since `start`, restarting the stopwatch.
    #[inline]
    pub fn lap(&mut self) -> f64 {
        let s = self.secs();
        *self = Timer::start();
        s
    }
}
