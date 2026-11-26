//! Frame timing trace: every frame of a gesture (a brush stroke, a drag) as a row, so a tester on
//! any platform reports a per-stage breakdown instead of one input-to-pixel number.
//!
//! Set the app's trace variable before launching ([`FrameTrace::from_env`]; PhotoCraft keeps
//! `PHOTOCRAFT_STROKE_TRACE`): `1` prints a [`Summary`] to stderr when each gesture ends; a file
//! path also appends every frame of every gesture to that CSV. The last summary is in the control
//! channel's `perf` output under `frameTrace` (PhotoCraft's `strokeTrace`, renamed with
//! `gesture_ms`; spec Q-S3T-5). The trace keeps no clock: each row is the
//! frame's [`FrameStages`], so the slow-frame ring and the trace read one record.

use std::fmt::Write as _;

use crate::profiler::FrameStages;

/// One frame of a gesture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// Frame start on the app clock, ms.
    pub t: f64,
    /// Time since the previous frame started, ms; `0` for the first frame of a gesture.
    pub interval: f64,
    /// Pointer moves delivered this frame.
    pub moves: usize,
    /// The live gesture's render time, ms.
    pub gesture: f64,
    /// Canvas composite plus upload time, ms.
    pub canvas: f64,
    /// UI pass time, ms.
    pub ui: f64,
    /// The windowing library's CPU time for the previous frame, ms.
    pub cpu_prev: Option<f64>,
}

/// `[p50, p95]` per stage over one gesture's frames.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Frames in the gesture.
    pub frames: usize,
    /// The render backend name.
    pub backend: String,
    /// Frame interval, `[p50, p95]` in ms.
    pub interval: [f64; 2],
    /// Pointer moves per frame, `[p50, p95]`.
    pub moves: [f64; 2],
    /// Gesture render time, `[p50, p95]` in ms.
    pub gesture: [f64; 2],
    /// Canvas time, `[p50, p95]` in ms.
    pub canvas: [f64; 2],
    /// UI time, `[p50, p95]` in ms.
    pub ui: [f64; 2],
    /// CPU time of the previous frame, `[p50, p95]` in ms.
    pub cpu: [f64; 2],
    /// Time between frames not spent in the CPU of the previous frame, `[p50, p95]` in ms.
    pub wait: [f64; 2],
}

impl Row {
    /// The trace's row for a frame's stages.
    fn from_stages(s: &FrameStages, interval: f64) -> Self {
        Self { t: s.t, interval, moves: s.moves, gesture: s.gesture_ms, canvas: s.composite_ms + s.upload_ms, ui: s.ui_ms, cpu_prev: s.cpu_prev_ms }
    }
}

impl Summary {
    /// One stderr line.
    pub fn line(&self) -> String {
        let f = |name: &str, [p50, p95]: [f64; 2]| format!("{name} {p50:.2}/{p95:.2}");
        [
            format!("frame trace: {} frames on {} (p50/p95 ms):", self.frames, self.backend),
            f("interval", self.interval),
            f("moves/frame", self.moves),
            f("gesture", self.gesture),
            f("canvas", self.canvas),
            f("ui", self.ui),
            f("cpu", self.cpu),
            f("wait", self.wait),
        ]
        .join(" ")
    }
}

/// Where the trace goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Output {
    /// No trace.
    #[default]
    Off,
    /// Print a summary to stderr when each gesture ends.
    Stderr,
    /// Also append every frame of every gesture to this CSV file.
    Csv(std::path::PathBuf),
}

/// The trace state: the current gesture's rows and the last summary.
#[derive(Debug, Default)]
pub struct FrameTrace {
    output: Output,
    last_start: Option<f64>,
    rows: Vec<Row>,
    gestures: usize,
    /// The last finished gesture's summary.
    pub last: Option<Summary>,
}

impl FrameTrace {
    /// Reads the environment variable `var` (off on wasm, which has no environment).
    pub fn from_env(var: &str) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(v) = std::env::var_os(var) {
            return Self::from_value(&v.to_string_lossy());
        }
        let _ = var;
        Self::default()
    }

    /// `v` trimmed: `""`, `"0"`, `"off"` or `"false"` is off; `"1"`, `"stderr"` or `"true"` prints
    /// summaries; anything else is a CSV path. Always off on wasm (no clock there).
    pub fn from_value(v: &str) -> Self {
        let v = v.trim();
        let output = match v {
            _ if cfg!(target_arch = "wasm32") => Output::Off,
            "" | "0" | "off" | "false" => Output::Off,
            "1" | "stderr" | "true" => Output::Stderr,
            path => Output::Csv(path.into()),
        };
        Self { output, ..Default::default() }
    }

    /// Whether the trace records frames.
    pub fn is_on(&self) -> bool {
        !matches!(self.output, Output::Off)
    }

    /// Turn summaries on or off (the control channel's `ui.set`); never sets a path.
    pub fn set_on(&mut self, on: bool) {
        let last = self.last.take();
        *self = Self { last, ..Self::from_value(if on { "1" } else { "off" }) };
    }

    /// Record `stages` when `in_gesture`; the first frame after a gesture ends summarises it,
    /// with `backend` named in the summary.
    pub fn frame(&mut self, stages: &FrameStages, in_gesture: bool, backend: impl FnOnce() -> String) {
        if !self.is_on() {
            return;
        }
        let interval = self.last_start.map_or(0.0, |t| stages.t - t);
        self.last_start = Some(stages.t);
        if in_gesture || stages.gesture_ms > 0.0 {
            // The first frame of a gesture does not follow a gesture frame: its interval is idle time.
            let interval = if self.rows.is_empty() { 0.0 } else { interval };
            self.rows.push(Row::from_stages(stages, interval));
        } else if !self.rows.is_empty() {
            self.finish(backend());
        }
    }

    fn finish(&mut self, backend: String) {
        let rows = std::mem::take(&mut self.rows);
        self.gestures += 1;
        let col = |g: fn(&Row) -> Option<f64>| percentiles(rows.iter().filter_map(g).collect());
        let summary = Summary {
            frames: rows.len(),
            backend,
            interval: col(|r| (r.interval > 0.0).then_some(r.interval)),
            moves: col(|r| Some(r.moves as f64)),
            gesture: col(|r| Some(r.gesture)),
            canvas: col(|r| Some(r.canvas)),
            ui: col(|r| Some(r.ui)),
            // A frame's cpu_prev belongs to the frame before it, so pair it with that interval.
            cpu: percentiles(rows.iter().skip(1).filter_map(|r| r.cpu_prev).collect()),
            wait: percentiles(rows.windows(2).filter_map(|w| Some((w[1].interval - w[1].cpu_prev?).max(0.0))).collect()),
        };
        eprintln!("{}", summary.line());
        #[cfg(not(target_arch = "wasm32"))]
        if let Output::Csv(path) = &self.output
            && let Err(e) = append_csv(path, self.gestures, &rows)
        {
            eprintln!("frame trace: can't write {}: {e}", path.display());
        }
        self.last = Some(summary);
    }
}

/// `[p50, p95]` of `v`, nearest rank, rounded to 0.01; zeros when empty.
pub fn percentiles(mut v: Vec<f64>) -> [f64; 2] {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return [0.0, 0.0];
    }
    v.sort_by(f64::total_cmp);
    let at = |p: f64| v.get(((p * (v.len() - 1) as f64).round() as usize).min(v.len() - 1)).copied().unwrap_or(0.0);
    let r = |x: f64| (x * 100.0).round() / 100.0;
    [r(at(0.5)), r(at(0.95))]
}

/// The CSV text for `rows` of gesture number `gesture` (with the header when `header`).
fn csv(gesture: usize, rows: &[Row], header: bool) -> String {
    let mut s = String::new();
    if header {
        s.push_str("gesture,t_ms,interval_ms,moves,gesture_ms,canvas_ms,ui_ms,cpu_prev_ms\n");
    }
    for r in rows {
        let cpu = r.cpu_prev.map_or(String::new(), |c| format!("{c:.3}"));
        let _ = writeln!(s, "{gesture},{:.3},{:.3},{},{:.3},{:.3},{:.3},{cpu}", r.t, r.interval, r.moves, r.gesture, r.canvas, r.ui);
    }
    s
}

#[cfg(not(target_arch = "wasm32"))]
fn append_csv(path: &std::path::Path, gesture: usize, rows: &[Row]) -> std::io::Result<()> {
    use std::io::Write as _;
    let header = std::fs::metadata(path).map_or(true, |m| m.len() == 0);
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(csv(gesture, rows, header).as_bytes())
}
