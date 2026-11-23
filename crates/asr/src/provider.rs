//! The speech-recognition protocol between a host and an engine.
//!
//! A [`Recognizer`] owns its model and its worker. The host calls [`Recognizer::start`], feeds
//! captured audio with [`Recognizer::feed`], ends the stream with [`Recognizer::finish`], and
//! takes whatever lines arrived with [`Recognizer::poll`], which never blocks. A [`Segment`]
//! marked `final` replaces the partial line it grew from; a partial line is still changing.
//!
//! [`FakeRecognizer`] plays a script one line at a time, so a UI, a layout test or a host
//! without a microphone can exercise the protocol without a model or a capture device.

use std::collections::VecDeque;

use crate::audio::{AudioBuffer, AudioError, Limits};

/// The longest line a transcript may carry: a runaway decode cannot grow a caption
/// indefinitely.
pub const MAX_SEGMENT_CHARS: usize = 500;

/// One recognised line. A partial line (`r#final == false`) is still changing; a final line
/// stays until the next final line replaces it.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// Start time in seconds from the beginning of the stream.
    pub start: f64,
    /// End time in seconds; never before `start`.
    pub end: f64,
    /// The recognised text, stripped of control characters and bounded by [`MAX_SEGMENT_CHARS`].
    pub text: String,
    /// Whether this line is final rather than a partial line still being recognised.
    pub r#final: bool,
}

impl Segment {
    /// A line with sanitised times and text. Times that are not finite become zero, `end` is
    /// raised to at least `start`, and an overlong line keeps its newest words.
    pub fn new(start: f64, end: f64, text: impl Into<String>, is_final: bool) -> Self {
        let start = seconds(start);
        let end = seconds(end).max(start);
        Segment { start, end, text: clip(text.into()), r#final: is_final }
    }

    /// A partial line.
    pub fn partial(start: f64, end: f64, text: impl Into<String>) -> Self {
        Segment::new(start, end, text, false)
    }

    /// A final line.
    pub fn final_line(start: f64, end: f64, text: impl Into<String>) -> Self {
        Segment::new(start, end, text, true)
    }
}

fn seconds(value: f64) -> f64 {
    if value.is_finite() { value.max(0.0) } else { 0.0 }
}

/// Keep the newest characters and make the line safe to draw. A model can echo control
/// characters, a bidirectional override or a zero-width joiner from the audio, and drawing them
/// reorders the line or hides text. Line breaks and tabs become an ordinary space; other
/// control characters and invisible format characters are dropped.
fn clip(text: String) -> String {
    let text: String =
        text.chars().map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c }).filter(|c| !c.is_control() && !is_invisible_format(*c)).collect();
    let count = text.chars().count();
    if count > MAX_SEGMENT_CHARS { text.chars().skip(count - MAX_SEGMENT_CHARS).collect() } else { text }
}

/// Bidi controls and zero-width characters a transcript must never contain. The zero-width
/// non-joiner and joiner (U+200C/U+200D) are kept: they are part of Persian, Hindi and emoji
/// spelling, and removing them changes the words.
fn is_invisible_format(c: char) -> bool {
    matches!(c as u32, 0x00AD | 0x061C | 0x200B | 0x200E | 0x200F | 0x2028..=0x202E | 0x2060..=0x2064 | 0x2066..=0x2069 | 0xFEFF)
}

/// Why a recognizer could not start, accept audio or report a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsrError {
    /// The audio format or length is outside what a recognizer accepts.
    Audio(AudioError),
    /// Audio was fed before [`Recognizer::start`] or after [`Recognizer::finish`].
    NotRunning,
    /// The session reached its segment cap; a runaway decode must not grow the transcript.
    SegmentLimit { max: usize },
    /// The engine failed: a missing or damaged model, a device, or inference.
    Provider(String),
}

impl AsrError {
    /// A provider failure whose message is capped so a hostile engine cannot flood a status line.
    pub fn provider(message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.chars().count() > 400 {
            message = message.chars().take(400).collect();
        }
        AsrError::Provider(message)
    }
}

impl From<AudioError> for AsrError {
    fn from(error: AudioError) -> Self {
        AsrError::Audio(error)
    }
}

impl std::fmt::Display for AsrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsrError::Audio(error) => write!(f, "unusable audio: {error}"),
            AsrError::NotRunning => f.write_str("the recognizer is not running"),
            AsrError::SegmentLimit { max } => write!(f, "the transcript reached its limit of {max} segments"),
            AsrError::Provider(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for AsrError {}

/// A recognizer's lifecycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Starting,
    Running,
    Stopped,
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Starting => "starting",
            State::Running => "running",
            State::Stopped => "stopped",
            State::Failed => "failed",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, State::Starting | State::Running)
    }
}

/// What a host reports about the engine in a status line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// Why the engine is not running: a missing model, no microphone, a load error.
    pub detail: Option<String>,
    /// The model id in use, when one loaded.
    pub model: Option<String>,
    /// The language hint handed to the engine.
    pub language: Option<String>,
}

impl Status {
    pub fn idle() -> Self {
        Status::default()
    }

    pub fn starting(model: impl Into<String>, language: Option<String>) -> Self {
        Status { state: State::Starting, detail: None, model: Some(model.into()), language }
    }

    pub fn running(model: impl Into<String>, language: Option<String>) -> Self {
        Status { state: State::Running, detail: None, model: Some(model.into()), language }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        let message = AsrError::provider(message);
        Status { state: State::Failed, detail: Some(message.to_string()), model: None, language: None }
    }

    pub fn stopped() -> Self {
        Status { state: State::Stopped, detail: None, model: None, language: None }
    }

    /// A one-line description a status bar can show.
    pub fn summary(&self) -> String {
        match (&self.state, &self.detail) {
            (State::Failed, Some(detail)) => format!("Speech recognition unavailable: {detail}"),
            (State::Failed, None) => "Speech recognition unavailable".to_string(),
            (State::Idle, _) => "Speech recognition idle".to_string(),
            (State::Starting, _) => "Starting speech recognition…".to_string(),
            (State::Running, _) => "Speech recognition running".to_string(),
            (State::Stopped, _) => "Speech recognition stopped".to_string(),
        }
    }
}

/// A speech engine a host can start, feed, finish and poll.
pub trait Recognizer: Send {
    /// Begin listening. Return `Err` when the engine cannot start at all (no model, no
    /// microphone); failures after this are reported by `feed` or `finish`.
    fn start(&mut self) -> Result<(), AsrError>;
    /// Accept a block of captured audio. The buffer is validated against the engine's
    /// [`Limits`] before it is converted or stored.
    fn feed(&mut self, audio: &AudioBuffer) -> Result<(), AsrError>;
    /// End the stream, emitting any final line for a partial one.
    fn finish(&mut self) -> Result<(), AsrError>;
    /// Take the lines that arrived since the last call. Never blocks.
    fn poll(&mut self) -> Vec<Segment>;
    /// The current status, cheap enough for a UI frame.
    fn status(&self) -> Status;
}

/// A deterministic recognizer for tests and for hosts without a model or a microphone.
///
/// [`FakeRecognizer::scripted`] gives one partial and then one final segment per line, each pair
/// advanced by one non-empty [`Recognizer::feed`]. Silence (an empty buffer or one with no
/// frames) is accepted and produces nothing.
#[derive(Debug)]
pub struct FakeRecognizer {
    script: VecDeque<String>,
    current: Option<String>,
    showing_partial: bool,
    queue: VecDeque<Segment>,
    status: Status,
    start_error: Option<String>,
    started: bool,
    limits: Limits,
    emitted: usize,
    polls: usize,
}

impl FakeRecognizer {
    /// A recognizer that plays `lines`, one partial-then-final pair per line.
    pub fn scripted(lines: &[&str]) -> Self {
        let mut fake = FakeRecognizer::new();
        fake.script = lines.iter().map(|line| (*line).to_string()).collect();
        fake
    }

    /// An idle recognizer with no script.
    pub fn new() -> Self {
        FakeRecognizer {
            script: VecDeque::new(),
            current: None,
            showing_partial: false,
            queue: VecDeque::new(),
            status: Status::idle(),
            start_error: None,
            started: false,
            limits: Limits::default(),
            emitted: 0,
            polls: 0,
        }
    }

    /// A recognizer that refuses to start with `message`.
    pub fn refusing(message: &str) -> Self {
        FakeRecognizer { start_error: Some(message.to_string()), ..FakeRecognizer::new() }
    }

    /// Apply a different limit set; a session may not report more than `max_segments`.
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// How many times the host polled.
    pub fn polls(&self) -> usize {
        self.polls
    }

    fn push(&mut self, segment: Segment) -> Result<(), AsrError> {
        if self.emitted >= self.limits.max_segments {
            return Err(AsrError::SegmentLimit { max: self.limits.max_segments });
        }
        self.emitted += 1;
        self.queue.push_back(segment);
        Ok(())
    }
}

impl Default for FakeRecognizer {
    fn default() -> Self {
        FakeRecognizer::new()
    }
}

impl Recognizer for FakeRecognizer {
    fn start(&mut self) -> Result<(), AsrError> {
        if let Some(message) = self.start_error.clone() {
            self.status = Status::failed(message.clone());
            return Err(AsrError::provider(message));
        }
        self.started = true;
        self.status = Status::running("fake", Some("en".into()));
        Ok(())
    }

    fn feed(&mut self, audio: &AudioBuffer) -> Result<(), AsrError> {
        if !self.started || self.status.state != State::Running {
            return Err(AsrError::NotRunning);
        }
        audio.validate(&self.limits)?;
        if audio.frames() == 0 {
            return Ok(());
        }
        if self.current.is_none() {
            self.current = self.script.pop_front();
            self.showing_partial = false;
        }
        let Some(line) = self.current.clone() else { return Ok(()) };
        if self.showing_partial {
            self.push(Segment::final_line(0.0, audio.duration_seconds(), line))?;
            self.current = None;
            self.showing_partial = false;
        } else {
            self.push(Segment::partial(0.0, audio.duration_seconds(), line))?;
            self.showing_partial = true;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), AsrError> {
        if self.started && self.showing_partial {
            if let Some(line) = self.current.take()
                && let Err(error) = self.push(Segment::final_line(0.0, 0.0, line))
            {
                self.status = Status::failed(error.to_string());
                return Err(error);
            }
            self.showing_partial = false;
        }
        self.started = false;
        if self.status.state != State::Failed {
            self.status = Status::stopped();
        }
        Ok(())
    }

    fn poll(&mut self) -> Vec<Segment> {
        self.polls += 1;
        if !self.started && self.queue.is_empty() {
            return Vec::new();
        }
        self.queue.drain(..).collect()
    }

    fn status(&self) -> Status {
        self.status.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::TARGET_RATE;

    fn one_second() -> AudioBuffer {
        AudioBuffer::new(vec![0.25; TARGET_RATE as usize], TARGET_RATE, 1)
    }

    #[test]
    fn a_scripted_recognizer_reports_a_partial_then_a_final_line() {
        let mut fake = FakeRecognizer::scripted(&["hello"]);
        assert_eq!(fake.status().state, State::Idle);
        fake.start().unwrap_or(());
        assert_eq!(fake.status().state, State::Running);
        fake.feed(&one_second()).unwrap_or(());
        let partial = fake.poll();
        assert_eq!(partial.len(), 1);
        assert!(!partial[0].r#final);
        assert_eq!(partial[0].text, "hello");
        fake.feed(&one_second()).unwrap_or(());
        let final_line = fake.poll();
        assert_eq!(final_line.len(), 1);
        assert!(final_line[0].r#final);
        assert_eq!(final_line[0].text, "hello");
        assert!(fake.poll().is_empty());
        fake.finish().unwrap_or(());
        assert_eq!(fake.status().state, State::Stopped);
    }

    #[test]
    fn empty_audio_produces_no_segment_and_does_not_advance_the_script() {
        let mut fake = FakeRecognizer::scripted(&["hello"]);
        fake.start().unwrap_or(());
        fake.feed(&AudioBuffer::new(vec![], TARGET_RATE, 1)).unwrap_or(());
        fake.feed(&AudioBuffer::new(vec![], TARGET_RATE, 2)).unwrap_or(());
        assert!(fake.poll().is_empty());
        fake.finish().unwrap_or(());
        assert!(fake.poll().is_empty());
        assert_eq!(fake.status().state, State::Stopped);
    }

    #[test]
    fn a_refusing_recognizer_reports_failure_and_audio_is_refused_while_idle() {
        let mut fake = FakeRecognizer::refusing("no microphone");
        assert!(matches!(fake.start(), Err(AsrError::Provider(_))));
        assert_eq!(fake.status().state, State::Failed);
        assert_eq!(fake.feed(&one_second()), Err(AsrError::NotRunning));
        fake.finish().unwrap_or(());
        assert_eq!(fake.status().state, State::Failed, "a failed stream stays failed after finish");
    }

    #[test]
    fn hostile_audio_is_refused_before_a_line_is_reported() {
        let mut fake = FakeRecognizer::scripted(&["hello"]).with_limits(Limits::tiny());
        fake.start().unwrap_or(());
        let hostile = AudioBuffer::new(vec![0.0; 3], TARGET_RATE, 2);
        assert!(matches!(fake.feed(&hostile), Err(AsrError::Audio(AudioError::PartialFrame { .. }))));
        assert!(fake.poll().is_empty(), "a refused buffer reports nothing");
        let too_long = AudioBuffer::new(vec![0.0; 8_000 * 2], 8_000, 1);
        assert!(matches!(fake.feed(&too_long), Err(AsrError::Audio(AudioError::TooLong { .. }))));
        let zero_rate = AudioBuffer::new(vec![0.0], 0, 1);
        assert!(matches!(fake.feed(&zero_rate), Err(AsrError::Audio(AudioError::ZeroSampleRate))));
    }

    #[test]
    fn the_segment_cap_stops_a_runaway_transcript() {
        let limits = Limits { max_segments: 1, ..Limits::default() };
        let mut fake = FakeRecognizer::scripted(&["one", "two"]).with_limits(limits);
        fake.start().unwrap_or(());
        fake.feed(&one_second()).unwrap_or(());
        assert_eq!(fake.poll().len(), 1);
        assert_eq!(fake.feed(&one_second()), Err(AsrError::SegmentLimit { max: 1 }));
        assert!(fake.poll().is_empty());
    }

    #[test]
    fn lines_are_safe_and_bounded_and_times_are_ordered() {
        let hostile = Segment::final_line(-5.0, f64::NAN, "he\u{202E}llo\u{0} wor\u{202B}ld\u{200B}!\n");
        assert_eq!(hostile.text, "hello world! ");
        assert_eq!((hostile.start, hostile.end), (0.0, 0.0));
        let long = Segment::partial(0.0, 0.0, "x".repeat(MAX_SEGMENT_CHARS * 3));
        assert_eq!(long.text.chars().count(), MAX_SEGMENT_CHARS);
        assert!(Segment::partial(0.0, 0.0, "old ".repeat(200) + "newest").text.ends_with("newest"));
        let backwards = Segment::partial(5.0, 1.0, "x");
        assert_eq!(backwards.end, 5.0, "end is never before start");
        let joiners = "می\u{200C}خواهم 👩\u{200D}💻";
        assert_eq!(Segment::partial(0.0, 0.0, joiners).text, joiners);
        assert_eq!(AsrError::provider("y".repeat(1_000)).to_string().chars().count(), 400);
    }

    #[test]
    fn states_and_status_summaries_are_distinguishable() {
        assert_eq!(State::Running.as_str(), "running");
        assert!(State::Starting.is_active() && !State::Idle.is_active());
        assert_eq!(Status::failed("x").summary(), "Speech recognition unavailable: x");
        assert_eq!(Status::running("fake", None).summary(), "Speech recognition running");
    }
}
