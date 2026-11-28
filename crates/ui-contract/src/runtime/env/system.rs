//! The OS's appearance as the host's [`SystemReader`] reports it: the one source the root's `System` choices
//! resolve from.

use crate::runtime::egui::memory::Service;
use craft_platform::appearance::{SystemReader, Theme};
use craft_theme::SystemAppearance;
use egui::{Context, Event};

/// The context's reader, from install on; a context without the runtime reads no answers.
pub(crate) struct Source(SystemReader);

impl Default for Source {
    fn default() -> Self {
        Source(SystemReader::unwatched())
    }
}

/// The OS's appearance now, and the generation that rises each time it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SystemState {
    /// Light or dark, contrast and reduced motion; `None` where the OS gave no answer.
    pub appearance: SystemAppearance,
    /// Rises on each change of the reading. A resolve made from the reading (a library theme, say) runs again
    /// when it moved.
    pub generation: u64,
}

/// What the OS reports on `ctx` now: the host's reader (`InstallSpec::system`), with the colour scheme egui's
/// integration reports (`Event::SystemTheme`) where the reader's own watcher has none.
pub fn system_appearance(ctx: &Context) -> SystemState {
    Service::<Source>::run(ctx, |source| {
        // Generation first: a change landing between the two reads then shows again at the next read.
        let generation = source.0.generation();
        SystemState { appearance: source.0.read().into(), generation }
    })
}

/// The desktop text scale from the installed [`SystemReader`], `None` where the OS gives none.
pub fn system_text_scale(ctx: &Context) -> Option<f32> {
    Service::<Source>::run(ctx, |source| source.0.text_scale())
}

/// Tells the reader whether the app is active, so a desktop source that reports the appearance only for an active app
/// reads again when the root window gains focus.
pub(crate) fn set_active(ctx: &Context, active: bool) {
    // PLAN: assumed SystemReader::set_active(&self, bool) (pdf-shell-w3)
    Service::<Source>::run(ctx, |source| source.0.set_active(active));
}

pub(crate) fn install(ctx: &Context, reader: SystemReader) {
    Service::<Source>::run(ctx, |source| source.0 = reader);
}

/// The input hook's step: the integration's colour scheme reaches the reader before the pass reads it.
pub(crate) fn input_hook(ctx: &Context, events: &[Event]) {
    let mut themes = events.iter().filter_map(|event| if let Event::SystemTheme(theme) = event { Some(*theme) } else { None }).peekable();
    if themes.peek().is_none() {
        return;
    }
    Service::<Source>::run(ctx, |source| {
        for theme in themes {
            source.0.report_theme(if theme == egui::Theme::Dark { Theme::Dark } else { Theme::Light });
        }
    });
}

/// Reports the colour scheme `dark` as the OS's through the host's reader alone, as a desktop source would,
/// without an `Event::SystemTheme` from the integration.
#[cfg(feature = "testing")]
pub(crate) fn report_theme(ctx: &Context, dark: bool) {
    Service::<Source>::run(ctx, |source| source.0.report_theme(if dark { Theme::Dark } else { Theme::Light }));
}

/// Reports `scale` as the OS's text scale (`None`: no scaling), as a desktop source would.
#[cfg(feature = "testing")]
pub(crate) fn report_text_scale(ctx: &Context, scale: Option<f32>) {
    Service::<Source>::run(ctx, |source| source.0.report_text_scale(scale));
}

/// Reports `contrast` as the OS's, as a desktop source would.
#[cfg(feature = "testing")]
pub(crate) fn report_contrast(ctx: &Context, contrast: craft_theme::Contrast) {
    use craft_platform::appearance::Contrast;
    let contrast = if contrast == craft_theme::Contrast::High { Contrast::High } else { Contrast::Standard };
    Service::<Source>::run(ctx, |source| source.0.report_contrast(contrast));
}
