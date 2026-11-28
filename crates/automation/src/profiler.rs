//! Where each frame's time goes, gathered every frame, and the slow ones kept.
//!
//! [`FrameStages`] is one frame's record; each stage has one writer (the raw input hook starts the
//! frame, the command runner adds commands, the canvas adds damage, composite and upload, the UI
//! pass closes it through [`SlowFrames::finish`]). A frame over [`SLOW_FRAME_MS`] goes into
//! [`SlowFrames`], the last [`SLOW_FRAMES`] of them, which the control channel's `perf` output
//! lists as `slowFrames`; one `info` log line per second names them. [`crate::trace`] reads the
//! same record, so an app times a frame in one place.

use std::collections::VecDeque;

/// At most one slow-frame log line per this many ms.
const LOG_EVERY_MS: f64 = 1000.0;

/// A frame slower than this is kept.
pub const SLOW_FRAME_MS: f64 = 50.0;
/// Slow frames kept.
pub const SLOW_FRAMES: usize = 64;

/// One frame's stage times in milliseconds. Field names are the `perf` output's JSON keys.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStages {
    /// Frame start on the app clock, ms.
    pub t: f64,
    /// Whole frame time; never less than the sum of its stages.
    pub frame_ms: f64,
    /// Time from frame start to the UI pass, minus command time.
    pub input_ms: f64,
    /// Time running the command named in `command`.
    pub command_ms: f64,
    /// The command id that ran this frame, if any.
    pub command: String,
    /// Time computing the changed canvas region.
    pub damage_ms: f64,
    /// Time compositing the canvas on the CPU.
    pub composite_ms: f64,
    /// Time uploading canvas pixels to a texture.
    pub upload_ms: f64,
    /// The GPU composite time; `None` off the GPU path.
    pub gpu_ms: Option<f64>,
    /// Presentation time of the previous frame, if known.
    pub present_ms: Option<f64>,
    /// What the canvas refreshed (`"full"`, `"rect"`, `"gpu-full"`, …; `""` none; a kind starting with `gpu` is the GPU path).
    pub refresh: &'static str,
    /// Pixels the canvas refreshed this frame.
    pub refresh_px: u64,
    /// The render path name (`"gpu"`, `"cpu"`).
    pub path: &'static str,
    /// Time in the UI pass.
    #[serde(skip)]
    pub ui_ms: f64,
    /// The live gesture's own render time (a brush stroke).
    #[serde(skip)]
    pub gesture_ms: f64,
    /// Pointer moves delivered this frame.
    #[serde(skip)]
    pub moves: usize,
    /// eframe's CPU time for the previous frame.
    #[serde(skip)]
    pub cpu_prev_ms: Option<f64>,
    /// The raw input hook ran for this frame.
    #[serde(skip)]
    started: bool,
}

impl FrameStages {
    /// Begin the frame at `now` with `moves` pointer moves.
    pub fn start(&mut self, now: f64, moves: usize) {
        self.t = now;
        self.moves = moves;
        self.started = true;
    }

    /// The UI pass begins: input time is `now - t`.
    pub fn ui_start(&mut self, now: f64, cpu_prev: Option<f32>) {
        if !self.started {
            self.start(now, 0);
        }
        self.input_ms = (now - self.t - self.command_ms).max(0.0);
        self.cpu_prev_ms = cpu_prev.map(|s| f64::from(s) * 1000.0);
    }

    /// A command `id` took `ms`.
    pub fn command(&mut self, id: &str, ms: f64) {
        self.command_ms += ms;
        id.clone_into(&mut self.command);
    }

    /// The canvas refreshed `px` pixels of `kind`.
    pub fn refresh(&mut self, kind: &'static str, px: u64, composite_ms: f64, upload_ms: f64) {
        self.composite_ms += composite_ms;
        self.upload_ms += upload_ms;
        self.refresh = kind;
        self.refresh_px += px;
    }

    /// The stages that make up `frame_ms` (`present_ms` belongs to the previous frame).
    pub fn stage_ms(&self) -> f64 {
        self.input_ms + self.command_ms + self.damage_ms + self.composite_ms + self.upload_ms
    }

    /// The frame ends at `now`; never shorter than its stages.
    fn end(&mut self, now: f64) {
        if !self.started {
            self.t = now;
        }
        self.frame_ms = (now - self.t).max(self.stage_ms());
    }

    /// One log line: `slow frame: 83.0 ms on the gpu path: input 2.0, command 60.0 (filter.blur), …`.
    pub fn line(&self) -> String {
        let opt = |v: Option<f64>| v.map_or("-".to_string(), |v| format!("{v:.1}"));
        let command = if self.command.is_empty() { String::new() } else { format!(" ({})", self.command) };
        format!(
            "slow frame: {:.1} ms on the {} path: input {:.1}, command {:.1}{command}, damage {:.1}, composite {:.1}, upload {:.1}, gpu {}, present {}; refresh {} {} px",
            self.frame_ms,
            self.path,
            self.input_ms,
            self.command_ms,
            self.damage_ms,
            self.composite_ms,
            self.upload_ms,
            opt(self.gpu_ms),
            opt(self.present_ms),
            if self.refresh.is_empty() { "none" } else { self.refresh },
            self.refresh_px,
        )
    }
}

/// The last [`SLOW_FRAMES`] frames over the threshold.
#[derive(Debug)]
pub struct SlowFrames {
    frames: VecDeque<FrameStages>,
    threshold_ms: f64,
    prev_ui_ms: Option<f64>,
    logged_at: Option<f64>,
    unlogged: usize,
}

impl Default for SlowFrames {
    fn default() -> Self {
        Self { frames: VecDeque::with_capacity(SLOW_FRAMES), threshold_ms: SLOW_FRAME_MS, prev_ui_ms: None, logged_at: None, unlogged: 0 }
    }
}

impl SlowFrames {
    /// The kept frames, oldest first.
    pub fn frames(&self) -> impl DoubleEndedIterator<Item = &FrameStages> {
        self.frames.iter()
    }

    /// Keep frames slower than `ms` instead of [`SLOW_FRAME_MS`].
    pub fn set_threshold_ms(&mut self, ms: f64) {
        self.threshold_ms = ms;
    }

    /// Close frame `s` at `now`: `gpu` says the canvas is on the GPU, `gpu_ms` is the GPU's
    /// latest composite time. A slow frame is kept and logged at most once a second.
    pub fn finish(&mut self, mut s: FrameStages, now: f64, gpu: bool, gpu_ms: Option<f64>) -> FrameStages {
        s.end(now);
        let on_gpu = if s.refresh.is_empty() { gpu } else { s.refresh.starts_with("gpu") };
        s.path = if on_gpu { "gpu" } else { "cpu" };
        s.gpu_ms = gpu_ms.filter(|_| on_gpu);
        s.present_ms = s.cpu_prev_ms.zip(self.prev_ui_ms).map(|(cpu, ui)| (cpu - ui).max(0.0));
        self.prev_ui_ms = Some(s.ui_ms);
        if let Some(line) = self.push(&s) {
            log::info!("{line}");
        }
        s
    }

    /// Keep `s` if it is slow; the log line to write, if one is due.
    fn push(&mut self, s: &FrameStages) -> Option<String> {
        if s.frame_ms.is_nan() || s.frame_ms <= self.threshold_ms {
            return None;
        }
        if self.frames.len() == SLOW_FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(s.clone());
        if self.logged_at.is_some_and(|at| s.t - at < LOG_EVERY_MS) {
            self.unlogged += 1;
            return None;
        }
        let mut line = s.line();
        if self.unlogged > 0 {
            line.push_str(&format!(" ({} more slow frames)", self.unlogged));
        }
        self.logged_at = Some(s.t);
        self.unlogged = 0;
        Some(line)
    }
}

impl serde::Serialize for SlowFrames {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.frames.iter())
    }
}

/// Monotonic milliseconds since the first call in this process: `craft_platform::clock::monotonic_ms`,
/// so profiler rows, traces, screenshot deadlines and the app's own timers share one origin.
pub fn now_ms() -> f64 {
    craft_platform::clock::monotonic_ms()
}
