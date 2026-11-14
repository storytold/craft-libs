//! Screenshot and input smoke tests of an app's screens in the suite's test languages
//! (`screen-testing` feature).
//!
//! [`Screens`] renders a UI closure with egui_kittest once per language in [`LANGUAGES`]: Arabic,
//! Japanese, Traditional Chinese (Taiwan), Korean, Thai and Vietnamese. Each run installs the
//! app's font stack for that UI language with no system fonts, so a snapshot looks the same on
//! every machine and a character no face draws fails the test (kittest panics on a missing glyph).
//! A right-to-left language lays the screen out from the right. [`compose`] types through an IME
//! composition, as a Korean, Japanese or Chinese input method sends it.
//!
//! The app's closure gets the [`Locale`] and picks its strings and settings from it; the
//! harness's language is not visible to the app otherwise.
//!
//! ```no_run
//! use craft_ui::testing::{KOREAN, Screens, compose};
//! use egui_kittest::kittest::Queryable as _;
//!
//! # const FACES: &[craft_ui::fonts::CraftFace] = &[];
//! let screens = Screens::new("rename").size([320.0, 160.0]).fonts(|fonts| fonts.craft_faces(FACES));
//! // Compares tests/snapshots/rename-ar.png, rename-ja.png, … (UPDATE_SNAPSHOTS=1 writes them).
//! screens.snapshots(|ui, locale| {
//!     ui.label(locale.language);
//! });
//!
//! let mut harness = screens.harness_state("ko", |ui, _, name: &mut String| {
//!     ui.text_edit_singleline(name);
//! }, String::new());
//! harness.get_by_role(egui::accesskit::Role::TextInput).focus();
//! compose(&mut harness, &KOREAN);
//! assert_eq!(harness.state(), "한");
//! ```
//!
//! # Shortcut smoke test
//!
//! craft-ui doesn't depend on craft-keymap, so this one is a recipe. Send the key event egui
//! reports for a press on a non-Latin layout and check the app ran the command the press means
//! by craft-keymap's layout rule. On Hebrew, the key at the Q position types `/`; with Ctrl held
//! egui reports `Slash` at physical `Q`, and the rule reads it as Ctrl+Q once the layout is known
//! not to be Latin (`Layout::with_latin(false)`, or after the app has seen Hebrew typed):
//!
//! ```ignore
//! let press = egui::Event::Key {
//!     key: egui::Key::Slash,
//!     physical_key: Some(egui::Key::Q),
//!     pressed: true,
//!     repeat: false,
//!     modifiers: egui::Modifiers::COMMAND,
//! };
//! let mut harness = screens.harness_state("ar", app_ui, app);
//! harness.event(press);
//! harness.run();
//! assert_eq!(harness.state().last_command(), Some(Command::Quit));
//! ```
//!
//! On Russian, the `с` key reports `C` at physical `C` and must run Ctrl+C. The app's dispatcher
//! (`craft_keymap::egui::Dispatcher`) turns the event into a `KeyInput` with
//! `KeyInput::from_egui`.

#[cfg(test)]
mod tests;

use std::rc::Rc;

use craft_i18n::Direction;
use egui::{Align, Context, Event, ImeEvent, Layout, Ui, Vec2};
use egui_kittest::{Harness, SnapshotResults};

use crate::fonts::{I18nScripts, SystemFonts, UiFonts};

/// The UI languages every app's key screens are tested in: one per kind of script layout
/// (right to left with joining, Han with kana, Traditional Han, Hangul, Thai marks without word
/// spaces, Latin with stacked diacritics).
pub const LANGUAGES: [&str; 6] = ["ar", "ja", "zh-TW", "ko", "th", "vi"];

/// The language a screen is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locale {
    /// Language tag, as in [`LANGUAGES`].
    pub language: &'static str,
    pub direction: Direction,
}

impl Locale {
    pub fn new(language: &'static str) -> Self {
        Self { language, direction: craft_i18n::direction(language) }
    }
}

type Fonts = Rc<dyn Fn(UiFonts<'static>) -> UiFonts<'static>>;
type Setup = Rc<dyn Fn(&Context, &Locale)>;

/// One screen's tests across [`LANGUAGES`]. Build it, then take a [`Self::harness`] per language
/// or compare [`Self::snapshots`].
#[derive(Clone)]
pub struct Screens {
    name: String,
    size: Vec2,
    languages: Vec<&'static str>,
    fonts: Fonts,
    setup: Option<Setup>,
}

impl Screens {
    /// `name` prefixes the snapshot files: `{name}-{language}.png`.
    pub fn new(name: &str) -> Self {
        Self { name: name.to_owned(), size: Vec2::new(400.0, 300.0), languages: LANGUAGES.to_vec(), fonts: Rc::new(|fonts| fonts), setup: None }
    }

    /// The harness size in points (default 400 × 300).
    pub fn size(mut self, size: impl Into<Vec2>) -> Self {
        self.size = size.into();
        self
    }

    /// Languages to test instead of [`LANGUAGES`].
    pub fn languages(mut self, languages: &[&'static str]) -> Self {
        self.languages = languages.to_vec();
        self
    }

    /// The app's font configuration: its own faces and the craft faces it embeds. The testing
    /// module offers [`LANGUAGES`] (call [`UiFonts::languages`] to offer the app's list instead),
    /// then sets the UI language and turns system fonts off after this runs.
    pub fn fonts(mut self, fonts: impl Fn(UiFonts<'static>) -> UiFonts<'static> + 'static) -> Self {
        self.fonts = Rc::new(fonts);
        self
    }

    /// Called on the context before the first frame, after the fonts are installed: the app's
    /// theme, and anything else it stores on the context. The harness applies the locale's
    /// direction ([`direction::apply`](crate::direction::apply)) before this runs and again after
    /// it, so egui's own chrome follows the locale even when the callback installs a whole new
    /// style. The callback can't override the direction.
    pub fn setup(mut self, setup: impl Fn(&Context, &Locale) + 'static) -> Self {
        self.setup = Some(Rc::new(setup));
        self
    }

    /// A harness drawing `app` in `language`.
    pub fn harness<'a>(&self, language: &'static str, mut app: impl FnMut(&mut Ui, &Locale) + 'a) -> Harness<'a> {
        self.harness_state(language, move |ui, locale, ()| app(ui, locale), ())
    }

    /// A harness drawing `app` in `language`, with state the test can read and change
    /// ([`Harness::state`]).
    pub fn harness_state<'a, State>(
        &self,
        language: &'static str,
        mut app: impl FnMut(&mut Ui, &Locale, &mut State) + 'a,
        state: State,
    ) -> Harness<'a, State> {
        let locale = Locale::new(language);
        let fonts = (self.fonts)(UiFonts::new(&I18nScripts).languages(&LANGUAGES[..])).ui_language(language).system_fonts(SystemFonts::Off).stack();
        let setup = self.setup.clone();
        Harness::builder()
            .with_size(self.size)
            .with_context_setup(move |ctx| {
                fonts.install(ctx);
                crate::direction::apply(ctx, locale.direction);
                if let Some(setup) = setup {
                    setup(ctx, &locale);
                    crate::direction::apply(ctx, locale.direction);
                }
            })
            .build_ui_state(
                move |ui, state| {
                    // A right-to-left screen starts at the right: egui then starts each
                    // `ui.horizontal` row from the right too.
                    let layout = match locale.direction {
                        Direction::Rtl => Layout::top_down(Align::Max),
                        Direction::Ltr => Layout::top_down(Align::Min),
                    };
                    ui.with_layout(layout, |ui| app(ui, &locale, state));
                },
                state,
            )
    }

    /// Draw `app` in each language and compare it with `tests/snapshots/{name}-{language}.png`
    /// (kittest's snapshot options and `kittest.toml` apply). Run with `UPDATE_SNAPSHOTS=1` to
    /// write new snapshots; review them before committing. The returned results panic on drop if
    /// any language differs, after every language has been compared.
    pub fn snapshots(&self, mut app: impl FnMut(&mut Ui, &Locale)) -> SnapshotResults {
        let mut results = SnapshotResults::new();
        for language in self.languages.clone() {
            let mut harness = self.harness(language, &mut app);
            harness.run();
            results.add(harness.try_snapshot(format!("{}-{language}", self.name)));
        }
        results
    }
}

/// One IME composition: the marked text after each keystroke, then the text committed.
#[derive(Clone, Copy, Debug)]
pub struct Composition<'a> {
    pub preedit: &'a [&'a str],
    pub commit: &'a str,
}

/// 2-Set Korean: ㅎ, ㅏ, ㄴ build one syllable.
pub const KOREAN: Composition<'static> = Composition { preedit: &["ㅎ", "하", "한"], commit: "한" };

/// Japanese romaji input, converted to kanji before commit.
pub const JAPANESE: Composition<'static> = Composition { preedit: &["に", "にほ", "にほん", "にほんご", "日本語"], commit: "日本語" };

/// Pinyin: Latin letters marked until a candidate is chosen.
pub const PINYIN: Composition<'static> = Composition { preedit: &["z", "zh", "zho", "zhon", "zhong"], commit: "中" };

/// Vietnamese Telex (IBus, Fcitx5): `Vieejt` builds the word in place.
pub const TELEX: Composition<'static> = Composition { preedit: &["V", "Vi", "Vie", "Viê", "Việ", "Việt"], commit: "Việt" };

/// The events an input method sends for `composition`: a preedit per keystroke, then the commit.
/// Each preedit marks its whole text as the segment being converted. egui's `TextEdit` and
/// canvases reading `Event::Ime`, such as craft-ui's IME composer, both take these.
pub fn ime_events(composition: &Composition<'_>) -> Vec<Event> {
    let preedit = composition
        .preedit
        .iter()
        .map(|text| Event::Ime(ImeEvent::Preedit { text: (*text).to_owned(), active_range_chars: Some(0..text.chars().count()) }));
    preedit.chain([Event::Ime(ImeEvent::Commit(composition.commit.to_owned()))]).collect()
}

/// Type `composition` into the focused widget, one frame per event as a platform delivers them,
/// then run the harness until it settles. Focus the text field or canvas first.
pub fn compose<State>(harness: &mut Harness<'_, State>, composition: &Composition<'_>) {
    for event in ime_events(composition) {
        harness.event(event);
        harness.step();
    }
    harness.run();
}
