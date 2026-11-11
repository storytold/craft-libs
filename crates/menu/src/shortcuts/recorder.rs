//! `ChordRecorder`: the focusable field that records shortcuts. It shows the text it was given, starts recording on
//! a click or Enter, and while recording reports the chords of the pass. It never decides what a chord means.
//!
//! One component serves both states. Idle, the field has the plain widget keys (Tab leaves, Enter and Space start
//! recording). Recording, its key use is `KeyUse::RECORDER` (every key is a chord; bare Tab stays a chord because
//! the field keeps Tab). The recording field takes key focus on every run it lacks it.

use craft_behavior::{Metrics, Proposal, Sense};
use craft_keymap::{Chord, Key};
use craft_theme::Stateful;
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, NoValue, Outcome, PaintUi, Pointer as _, Props as _, Role, Show as _, Styled as _,
};
use craft_ui::primitives::{TextRun, Wrap};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{self, FocusTarget, KeepKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, Ink, StyleCx, SurfaceKind, SurfaceLook, TextLook, TypeRole};
use egui::{Rect, Response, Ui, Vec2};

#[cfg(feature = "screen-testing")]
mod stories;

/// The field claims bare Escape while recording, so it reaches the field as a chord (the keymap's recorder reads it as
/// cancel) and neither leaves the row nor closes the dialog.
const ESCAPE: &[Chord] = &[Chord::plain(Key::Escape)];

/// What the recorder reports in one pass.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct RecorderOutput {
    /// The field's response.
    pub response: Response,
    /// The chords pressed this pass while recording, in order.
    pub chords: Vec<Chord>,
    /// A click or Enter asked to start recording.
    pub started: bool,
}

impl Outcome for RecorderOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of the field: a well, the text in the normal ink and an accent well while recording.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct FieldStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(surface)]
    surface: SurfaceLook,
    #[style(focus)]
    focus: FocusLook,
    #[style(text)]
    text: TextLook,
    /// Around the text.
    pad: f32,
}

fn field_style(cx: &StyleCx<'_>, recording: bool) -> FieldStyle {
    let well = cx.surface(SurfaceKind::Well);
    let fill = if recording { cx.tone_fill(craft_theme::variant::Tone::Accent) } else { cx.color(cx.tokens.color.surface.sunken) };
    let control = cx.control();
    // The accent well reads in the accent ink, the pair the theme tables for it.
    let ink = if recording { cx.tone(craft_theme::variant::Tone::Accent) } else { cx.ink(Ink::OnPanel) };
    FieldStyle {
        metrics: Metrics::new().min_size(Vec2::splat(control.height)),
        surface: SurfaceLook::new(Stateful::new(fill)).kind(SurfaceKind::Well).radius(well.radius),
        focus: cx.focus_look(),
        text: TextLook::new(TypeRole::Label, Stateful::new(ink)),
        pad: cx.tokens.space.pad.chip,
    }
}

/// The field that records. `shown` is the recording prompt or the chord.
#[derive(craft_ui::contract::Component)]
struct Recorder {
    #[common]
    common: Common,
    #[style_key]
    recording: bool,
    shown: Label,
    chords: Vec<Chord>,
    started: bool,
}

impl Component for Recorder {
    const ROLE: Role = Role::BUTTON;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(craft_behavior::Input::Pointer);
    type Style = FieldStyle;
    type Output = RecorderOutput;

    fn keys(&self) -> KeyUse {
        if self.recording { KeyUse::RECORDER.with_keep(KeepKeys::Tab).with_claims(ESCAPE) } else { Self::KEYS }
    }

    fn style(cx: &StyleCx<'_>, key: RecorderStyleKey) -> FieldStyle {
        field_style(cx, key.recording)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let text = cx.layout(&TextRun::new(self.shown.as_str()).role(cx.style.text.role).wrap(Wrap::Line)).size();
        text + Vec2::splat(cx.style.pad * 2.0)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if self.recording {
            if !cx.response.has_focus() {
                focus::request(&cx.response.ctx, FocusTarget::Id(cx.response.id));
            }
            self.chords.extend(cx.intents().take_chords());
            if cx.take_claimed(ESCAPE[0]) {
                self.chords.push(ESCAPE[0]);
            }
        } else if cx.activation().is_some() {
            self.started = true;
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        access.name(Name::Text(cx.resolved.label.to_owned()));
        access.value(self.shown.as_str().to_owned());
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let run = TextRun::new(self.shown.as_str()).role(cx.style.text.role).color(cx.resolve(&cx.style.text.color)).wrap(Wrap::Elide);
        ui.draw().text(cx.content_rect, &run);
    }

    fn finish(self, cx: Cx<'_, Self>) -> RecorderOutput {
        RecorderOutput { response: cx.response.clone(), chords: self.chords, started: self.started }
    }
}

/// The field that records: its accessible name, whether it records now and the text it shows (the recording
/// prompt, or the chord).
pub(crate) struct ChordRecorder {
    label: Label,
    recording: bool,
    shown: Label,
    salt: Option<&'static str>,
}

impl ChordRecorder {
    /// A recorder named `label` showing `shown`.
    pub(crate) fn new(label: impl Into<Label>, recording: bool, shown: impl Into<Label>) -> Self {
        ChordRecorder { label: label.into(), recording, shown: shown.into(), salt: None }
    }

    /// Salts the field's id.
    #[must_use]
    pub(crate) fn id_salt(mut self, salt: &'static str) -> Self {
        self.salt = Some(salt);
        self
    }

    fn component(self) -> (Recorder, Option<&'static str>) {
        let ChordRecorder { label, recording, shown, salt } = self;
        (Recorder { common: Common::new(), recording, shown, chords: Vec::new(), started: false }.label(label), salt)
    }

    /// Shows the field.
    pub(crate) fn show(self, ui: &mut Ui) -> RecorderOutput {
        let (mut field, salt) = self.component();
        if let Some(salt) = salt {
            field = field.id_salt(salt);
        }
        field.show(ui)
    }

    /// The width the field has under a component's measure, as [`ChordRecorder::child`] will show it.
    pub(crate) fn width<C: Component>(self, cx: &MeasureCx<'_, C>, key: impl std::hash::Hash + std::fmt::Debug) -> f32 {
        cx.child(key, &mut self.component().0, Proposal::Ideal).x
    }

    /// Shows the field as a child of a component's paint at `rect`. Idle and recording share `key`, so focus
    /// stays on the field when recording starts.
    pub(crate) fn child(self, ui: &mut PaintUi<'_>, key: impl std::hash::Hash + std::fmt::Debug, rect: Rect) -> RecorderOutput {
        ui.child(key, rect, self.component().0)
    }
}
