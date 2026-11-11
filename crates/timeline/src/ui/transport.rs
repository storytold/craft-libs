//! A monitor's transport bar and the compact media controls.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{EditEvents, IconId};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::{Button, IconButton, ToggleButton};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Held, NoValue, Outcome, Pattern, PopupKind, Props, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Divider, Text};
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::style::TypeRole;
use craft_ui::value_drag::Slider;
use egui::{Response, Ui};

/// How far the arrows and the seek keys move the position, in seconds.
const NUDGE: f64 = 0.25;

craft_ui::words! {
    /// The buttons' tooltips and names, English by default (FilmCraft's, with Premiere's keys).
    #[non_exhaustive]
    pub struct TransportWords {
        add_marker: "transport.addMarker" = "Add Marker (M)";
        mark_in: "transport.markIn" = "Mark In (I)";
        mark_out: "transport.markOut" = "Mark Out (O)";
        go_to_in: "transport.goToIn" = "Go to In (Shift+I)";
        step_back: "transport.stepBack" = "Step Back 1 Frame (Left)";
        play_stop: "transport.playStop" = "Play-Stop Toggle (Space)";
        step_forward: "transport.stepForward" = "Step Forward 1 Frame (Right)";
        go_to_out: "transport.goToOut" = "Go to Out (Shift+O)";
        insert: "transport.insert" = "Insert (,)";
        overwrite: "transport.overwrite" = "Overwrite (.)";
        lift: "transport.lift" = "Lift (;)";
        extract: "transport.extract" = "Extract (')";
        export_frame: "transport.exportFrame" = "Export Frame";
        loop_playback: "transport.loop" = "Loop";
        button_editor: "transport.buttonEditor" = "Button Editor";
        reset: "transport.reset" = "Reset";
        play_pause_media: "transport.playPauseMedia" = "Play or pause media";
        back_quarter: "transport.backQuarter" = "Back 0.25 seconds";
        forward_quarter: "transport.forwardQuarter" = "Forward 0.25 seconds";
        seek: "transport.seek" = "Seek";
        toolbar: "transport.toolbar" = "Transport";
        media_controls: "transport.mediaControls" = "Media controls";
    }
}

/// A transport button. The app's own buttons (Toggle Proxies, a camera angle) are `Custom` with an icon and a label
/// in the app's words.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportButton {
    /// Add a marker at the playhead.
    AddMarker,
    /// Set the In mark at the playhead.
    MarkIn,
    /// Set the Out mark at the playhead.
    MarkOut,
    /// Move the playhead to the In mark.
    GoToIn,
    /// Step one frame back.
    StepBack,
    /// Shows Pause while playing.
    PlayStop,
    /// Step one frame forward.
    StepForward,
    /// Move the playhead to the Out mark.
    GoToOut,
    /// Insert the source into the sequence.
    Insert,
    /// Overwrite the sequence with the source.
    Overwrite,
    /// Lift the In to Out range, leaving a gap.
    Lift,
    /// Extract the In to Out range, closing the gap.
    Extract,
    /// Export the frame at the playhead.
    ExportFrame,
    /// A toggle; shown pressed while on.
    Loop,
    /// An app-defined button.
    #[non_exhaustive]
    Custom {
        /// The button's identity, to tell it from other custom buttons.
        id: String,
        /// The button's icon.
        icon: IconId,
        /// The tooltip and accessible name, in the app's words.
        label: String,
    },
}

impl TransportButton {
    /// Premiere's Program monitor row; FilmCraft adds Toggle Proxies as a custom button before Loop.
    pub const PROGRAM: [TransportButton; 12] = [
        TransportButton::AddMarker,
        TransportButton::MarkIn,
        TransportButton::MarkOut,
        TransportButton::GoToIn,
        TransportButton::StepBack,
        TransportButton::PlayStop,
        TransportButton::StepForward,
        TransportButton::GoToOut,
        TransportButton::Lift,
        TransportButton::Extract,
        TransportButton::ExportFrame,
        TransportButton::Loop,
    ];

    /// Premiere's Source monitor row.
    pub const SOURCE: [TransportButton; 11] = [
        TransportButton::AddMarker,
        TransportButton::MarkIn,
        TransportButton::MarkOut,
        TransportButton::GoToIn,
        TransportButton::StepBack,
        TransportButton::PlayStop,
        TransportButton::StepForward,
        TransportButton::GoToOut,
        TransportButton::Insert,
        TransportButton::Overwrite,
        TransportButton::ExportFrame,
    ];

    /// An app-defined button `id` showing `icon`, named `label`.
    pub fn custom(id: impl Into<String>, icon: IconId, label: impl Into<String>) -> Self {
        TransportButton::Custom { id: id.into(), icon, label: label.into() }
    }

    /// The button's icon; Play shows as Pause while `playing`.
    pub fn icon(&self, playing: bool) -> IconId {
        match self {
            Self::AddMarker => IconId::new("flag"),
            Self::MarkIn => IconId::new("mark_in"),
            Self::MarkOut => IconId::new("mark_out"),
            Self::GoToIn => IconId::new("chevrons_left"),
            Self::StepBack => IconId::new("step_back"),
            Self::PlayStop if playing => IconId::new("pause"),
            Self::PlayStop => IconId::new("play"),
            Self::StepForward => IconId::new("step_forward"),
            Self::GoToOut => IconId::new("chevrons_right"),
            Self::Insert => IconId::new("insert_clip"),
            Self::Overwrite => IconId::new("overwrite_clip"),
            Self::Lift => IconId::new("lift_clip"),
            Self::Extract => IconId::new("extract_clip"),
            Self::ExportFrame => IconId::new("image"),
            Self::Loop => IconId::new("refresh"),
            Self::Custom { icon, .. } => icon.clone(),
        }
    }

    fn label<'a>(&'a self, words: &'a TransportWords) -> &'a str {
        match self {
            Self::AddMarker => words.add_marker.as_str(),
            Self::MarkIn => words.mark_in.as_str(),
            Self::MarkOut => words.mark_out.as_str(),
            Self::GoToIn => words.go_to_in.as_str(),
            Self::StepBack => words.step_back.as_str(),
            Self::PlayStop => words.play_stop.as_str(),
            Self::StepForward => words.step_forward.as_str(),
            Self::GoToOut => words.go_to_out.as_str(),
            Self::Insert => words.insert.as_str(),
            Self::Overwrite => words.overwrite.as_str(),
            Self::Lift => words.lift.as_str(),
            Self::Extract => words.extract.as_str(),
            Self::ExportFrame => words.export_frame.as_str(),
            Self::Loop => words.loop_playback.as_str(),
            Self::Custom { label, .. } => label.as_str(),
        }
    }
}

/// The buttons a monitor's row leaves out, chosen in its button editor; the app saves one per monitor (Premiere keeps
/// the Program and Source rows apart).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TransportBarSettings {
    /// The buttons left out of the row.
    pub hidden: Vec<TransportButton>,
}

impl TransportBarSettings {
    /// Settings that leave out `hidden`.
    pub fn new(hidden: Vec<TransportButton>) -> Self {
        TransportBarSettings { hidden }
    }
}

/// What the transport row did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct TransportBarOutput {
    /// The row's response, for hover and focus.
    pub response: Response,
    /// The button pressed this pass.
    pub pressed: Option<TransportButton>,
    /// The button editor changed [`TransportBarSettings`]; the app saves them.
    pub settings_changed: bool,
}

impl Outcome for TransportBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Whether the button editor's popover is open.
#[derive(Clone, Debug, Default)]
struct EditorOpen(bool);

/// A monitor's transport row (Premiere's, as FilmCraft draws it): a tool `Bar` of icon buttons, Pause in place of
/// Play while playing, Loop pressed while on, and with `editable` a "+" button editor at the end (a popover with a
/// checkbox per button and Reset).
///
/// - Keyboard and AccessKit are the bar's: buttons named by their tooltips, Space and Enter press. The keys in the
///   tooltips (Space, I, O, Left, Right) are the app's shortcuts, not handled here.
/// - Buttons in [`TransportBarSettings::hidden`] are left out; the editor changes the settings and sets
///   [`TransportBarOutput::settings_changed`].
#[derive(Pattern, Props)]
pub struct TransportBar<'a> {
    label: Label,
    buttons: &'a [TransportButton],
    /// The row's saved settings; without them every button shows and the editor is off.
    #[prop(into)]
    settings: Option<Bind<'a, TransportBarSettings>>,
    /// Whether playback is running; Play shows as Pause while it is.
    #[prop]
    playing: bool,
    /// Whether Loop is on; the Loop button shows pressed.
    #[prop]
    looping: bool,
    /// Shows the "+" button editor; on by default. The editor needs the row's settings.
    #[prop]
    editable: bool,
}

impl<'a> TransportBar<'a> {
    /// A row named `label` of `buttons`, in order.
    pub fn new(label: impl Into<Label>, buttons: &'a [TransportButton]) -> Self {
        TransportBar { label: label.into(), buttons, settings: None, playing: false, looping: false, editable: true }
    }

    /// Shows the row and reports the button pressed.
    pub fn show(self, ui: &mut Ui) -> TransportBarOutput {
        let TransportBar { label, buttons, settings, playing, looping, editable } = self;
        let words = Env::of(ui).words::<TransportWords>();
        let mut held = settings.map(Held::new);
        let (mut pressed, mut changed) = (None, false);
        let shown = Bar::new(label.clone(), BarKind::Tool).show(ui, |ui, cx| {
            let before = held.as_mut().and_then(|held| held.read(cx)).unwrap_or_default();
            let mut next = before.clone();
            for button in buttons.iter().filter(|button| !before.hidden.contains(button)) {
                let name = button.label(&words).to_owned();
                if *button == TransportButton::Loop {
                    let mut on = looping;
                    let edit = ToggleButton::new(name, Bind::bound(&mut on)).icon(button.icon(playing)).icon_only().show(ui);
                    if !edit.events.is_empty() {
                        pressed = Some(button.clone());
                    }
                } else if clicked(&IconButton::new(button.icon(playing)).label(name).show(ui)) {
                    pressed = Some(button.clone());
                }
            }
            if editable && held.is_some() {
                let opener = IconButton::new(IconId::new("plus")).label(words.button_editor.clone()).popup(PopupKind::Dialog).show(ui);
                let mut open = cx.keep::<EditorOpen>().0;
                if clicked(&opener) {
                    open = !open;
                }
                Popover::new(words.button_editor.clone(), Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
                    for button in buttons {
                        let mut visible = !next.hidden.contains(button);
                        let edit = Checkbox::new(Bind::bound(&mut visible)).label(button.label(&words).to_owned()).show(ui);
                        if !edit.events.is_empty() {
                            next.hidden.retain(|hidden| hidden != button);
                            if !visible {
                                next.hidden.push(button.clone());
                            }
                        }
                    }
                    Divider::horizontal().show(ui);
                    if clicked(&Button::new(words.reset.clone()).show(ui)) {
                        next.hidden.clear();
                    }
                });
                cx.keep::<EditorOpen>().0 = open;
            }
            if next != before {
                changed = true;
                if let Some(held) = held.as_mut() {
                    held.write(cx, &before, next, label.as_str());
                }
            }
        });
        TransportBarOutput { response: shown.output.response, pressed, settings_changed: changed }
    }
}

/// What the media controls did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct MediaControlsOutput {
    /// The controls' response, for hover and focus.
    pub response: Response,
    /// Play or Pause was pressed.
    pub toggle: bool,
    /// -0.25 or +0.25 seconds.
    pub nudge: Option<f64>,
    /// A drag of the position slider, in seconds, as one gesture.
    pub seek: EditEvents<f64>,
}

impl Outcome for MediaControlsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// PowerPoint's media controls under a video or sound on a slide: play or pause, back and forward a quarter second,
/// a seek track with the trim marks, and the time (`1:05.25` style in the mono role, digits through the units). The
/// controls are a named toolbar `Bar`, so a press on them never reaches the canvas under them. The seek track is a
/// `Slider` (one Tab stop; Left and Right a quarter second, Home and End) reporting an `EditEvent<f64>` gesture in
/// seconds.
#[derive(Pattern, Props)]
pub struct MediaControls {
    label: Label,
    position: f64,
    duration: f64,
    /// Whether playback is running; Play shows as Pause while it is.
    #[prop]
    playing: bool,
    /// The trimmed start and end, in seconds, marked on the track.
    #[prop(into, quantity = "seconds")]
    trim: Option<(f64, f64)>,
}

impl MediaControls {
    /// Controls named `label` at `position` of `duration` seconds.
    pub fn new(label: impl Into<Label>, position: f64, duration: f64) -> Self {
        MediaControls { label: label.into(), position, duration, playing: false, trim: None }
    }

    /// Shows the controls and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> MediaControlsOutput {
        let MediaControls { label, position, duration, playing, trim } = self;
        let env = Env::of(ui);
        let words = env.words::<TransportWords>();
        let units = env.units();
        let playable = duration.is_finite() && duration > 0.0;
        let position = if position.is_finite() { position.clamp(0.0, if playable { duration } else { 0.0 }) } else { 0.0 };
        let (mut toggle, mut nudge, mut seek) = (false, None, EditEvents::new());
        let shown = Bar::new(label, BarKind::Tool).show(ui, |ui, _| {
            let play = IconId::new(if playing { "pause" } else { "play" });
            toggle = clicked(&IconButton::new(play).label(words.play_pause_media.clone()).show(ui));
            if clicked(&IconButton::new(IconId::new("chevron_left")).label(words.back_quarter.clone()).show(ui)) {
                nudge = Some(-NUDGE);
            }
            if clicked(&IconButton::new(IconId::new("chevron_right")).label(words.forward_quarter.clone()).show(ui)) {
                nudge = Some(NUDGE);
            }
            let range = if playable { 0.0..=duration } else { 0.0..=1.0 };
            let marks: Vec<f64> = trim.filter(|_| playable).map(|(start, end)| [start, end]).into_iter().flatten().collect();
            let edit = Slider::new(Bind::given(position), range).step(NUDGE).marks(&marks).enabled(playable).label(words.seek.clone()).show(ui);
            seek = edit.events;
            Text::new(clock(&units, position)).role(TypeRole::Mono).show(ui);
        });
        MediaControlsOutput { response: shown.output.response, toggle, nudge, seek }
    }
}

/// `1:05.25`: minutes, seconds and hundredths, in the locale's digits.
fn clock(units: &craft_ui::runtime::env::Units, seconds: f64) -> String {
    let hundredths = (seconds.max(0.0) * 100.0).round() as u64;
    let (minutes, rest) = (hundredths / 6000, (hundredths % 6000) as f64 / 100.0);
    let pad = if rest < 10.0 { units.number_ungrouped(0.0, 0..=0) } else { String::new() };
    format!("{}:{pad}{}", units.number_ungrouped(minutes as f64, 0..=0), units.number_ungrouped(rest, 2..=2))
}
