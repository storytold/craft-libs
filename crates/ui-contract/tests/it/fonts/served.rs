//! Served faces: ordered like embedded ones, fetched through a `FontFetcher` when the UI needs them.

use std::sync::{Arc, Mutex};

use craft_fonts_build::ServedFace;
use craft_ui::fonts::{Family, FetchDone, FetchWake, FontFetcher, FontRef, I18nScripts, SystemFonts, UiFonts};
use craft_ui::testing::{Frames, trial};
use egui::FontId;

const ARABIC: &[u8] = include_bytes!("../../fonts/NotoSansArabic-test.ttf");
/// Noto Sans with Vietnamese ổ but not ẫ.
const NOTO_SANS_LATIN: &[u8] = include_bytes!("../../fonts/NotoSans-test.ttf");
/// Noto Sans Symbols' ⌃ ⎋ and Noto Sans Symbols 2's ⌥ ⌫ ⏎: the keyboard keys are split between them.
const SYMBOLS: &[u8] = include_bytes!("../../fonts/NotoSansSymbols-test.ttf");
const SYMBOLS_2: &[u8] = include_bytes!("../../fonts/NotoSansSymbols2-test.ttf");

const OFFERED: &[&str] = &["en", "ar", "ja", "zh-CN", "zh-TW", "zh-HK", "ko", "th", "ka", "ru"];

/// Coverage of every character, for faces whose tests are about scripts.
const ALL: &[u8] = &[0, 0xFF, 0xFF, 0x43];

const fn served(family: &'static str, style: &'static str, scripts: &'static [&'static str], url: &'static str, index: u32) -> ServedFace {
    ServedFace { family, style, scripts, index, url, size: 1, integrity: "", coverage: ALL }
}

fn coverage_of(font: &[u8]) -> &'static [u8] {
    Box::leak(craft_fonts_build::coverage::of_face(font, 0).expect("a font").into_boxed_slice())
}

fn egui_face(name: &str) -> egui::FontData {
    (**craft_ui::fonts::deflated_default_definitions().font_data.get(name).expect("an egui default face")).clone()
}

fn web_fonts(ui_language: &str, served: &[ServedFace]) -> UiFonts<'static> {
    UiFonts::new(&I18nScripts)
        .app_face("ui", egui_face("Ubuntu-Light"), &[FontRef::proportional()])
        .app_face("mono", egui_face("Hack"), &[FontRef::monospace()])
        .family(Family::named("semibold").weight(600))
        .served_faces(served)
        .languages(OFFERED)
        .ui_language(ui_language)
        .system_fonts(SystemFonts::Off)
}

/// craft-fonts' faces for Latin, Greek and Cyrillic, symbols and emoji, as a web build serves them.
const SERVED_SYMBOLS: &[ServedFace] = &[
    served("Noto Sans", "Regular", &["Latn", "Cyrl", "Grek", "Deva"], "fonts/noto-sans/NotoSans.ttf", 0),
    served("Noto Color Emoji", "Regular", &["Zsye"], "fonts/noto-color-emoji/NotoColorEmoji.ttf", 0),
    served("Noto Sans Symbols", "Regular", &["Zsym"], "fonts/noto-sans-symbols/NotoSansSymbols.ttf", 0),
    served("Noto Sans Symbols 2", "Regular", &["Zsym"], "fonts/noto-sans-symbols-2/NotoSansSymbols2-Regular.ttf", 0),
    served("Noto Sans Math", "Regular", &["Zmth", "Zsym"], "fonts/noto-sans-math/NotoSansMath-Regular.ttf", 0),
];

/// Fetches nothing until the test answers, on a clock the test moves.
#[derive(Default)]
struct FakeFetch {
    pending: Mutex<Vec<(&'static str, &'static str, FetchDone)>>,
    requested: Mutex<Vec<&'static str>>,
    now: Mutex<f64>,
    timers: Mutex<Vec<(f64, FetchWake)>>,
}

impl FontFetcher for FakeFetch {
    fn fetch(&self, url: &'static str, integrity: &'static str, done: FetchDone) {
        self.requested.lock().expect("lock").push(url);
        self.pending.lock().expect("lock").push((url, integrity, done));
    }

    fn now(&self) -> f64 {
        *self.now.lock().expect("lock")
    }

    fn after(&self, seconds: f64, wake: FetchWake) {
        let at = self.now() + seconds;
        self.timers.lock().expect("lock").push((at, wake));
    }
}

impl FakeFetch {
    /// The URLs requested since the last call.
    fn requested(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.requested.lock().expect("lock"))
    }

    fn answer(&self, url: &str, result: Result<&[u8], &str>) {
        self.answer_checked(url, None, result);
    }

    /// Answers the request for `url` made with `integrity` (any, for `None`).
    fn answer_checked(&self, url: &str, integrity: Option<&str>, result: Result<&[u8], &str>) {
        let mut pending = self.pending.lock().expect("lock");
        let i = pending
            .iter()
            .position(|(u, checked, _)| *u == url && integrity.is_none_or(|wanted| wanted == *checked))
            .unwrap_or_else(|| panic!("{url} is not being fetched"));
        let (.., done) = pending.remove(i);
        drop(pending);
        done(result.map(<[u8]>::to_vec).map_err(str::to_owned));
    }

    fn wait(&self, seconds: f64) {
        let now = {
            let mut now = self.now.lock().expect("lock");
            *now += seconds;
            *now
        };
        let due: Vec<_> = {
            let mut timers = self.timers.lock().expect("lock");
            let (due, later) = std::mem::take(&mut *timers).into_iter().partition(|(at, _)| *at <= now);
            *timers = later;
            due
        };
        for (_, wake) in due {
            wake();
        }
    }
}

const NOTO_SANS_URL: &str = "fonts/noto-sans/NotoSans.ttf";
const SYMBOLS_2_URL: &str = "fonts/noto-sans-symbols-2/NotoSansSymbols2-Regular.ttf";
const MATH_URL: &str = "fonts/noto-sans-math/NotoSansMath-Regular.ttf";

/// An English UI on the web whose faces lack Vietnamese ổ and the keyboard keys, with `fetch`
/// fetching its served faces; and whether the context draws a character, as a pass would ask.
fn web_context(fetch: &Arc<FakeFetch>) -> (egui::Context, impl FnMut(char) -> bool) {
    let mut faces = SERVED_SYMBOLS.to_vec();
    faces[0].coverage = coverage_of(NOTO_SANS_LATIN);
    faces[2].coverage = coverage_of(SYMBOLS);
    faces[3].coverage = coverage_of(SYMBOLS_2);
    // Arrows and technical symbols, ⌥ and ⇥ among them; never served.
    faces[4].coverage = craft_fonts_build::coverage::encode(&[(0x2190, 0x21FF), (0x2300, 0x23FF)]).leak();
    web_context_with("en", &faces, fetch)
}

/// A web context in `ui_language` with these served faces, fetched by `fetch`; and whether the
/// context draws a character, as a pass would ask.
fn web_context_with(ui_language: &str, faces: &[ServedFace], fetch: &Arc<FakeFetch>) -> (egui::Context, impl FnMut(char) -> bool + use<>) {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    web_fonts(ui_language, faces).fetcher(Arc::clone(fetch) as Arc<dyn FontFetcher>).install(&ctx);
    let draws = {
        let ctx = ctx.clone();
        move |c: char| {
            frames.frame(vec![], |_| {});
            ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(14.0), c))
        }
    };
    (ctx, draws)
}

#[trial]
fn a_missing_character_fetches_only_the_face_that_draws_it() {
    let fetch = Arc::new(FakeFetch::default());
    let (_ctx, mut draws) = web_context(&fetch);
    assert!(fetch.requested().is_empty(), "English needs no served face up front");

    assert!(!draws('ổ'));
    assert_eq!(fetch.requested(), [NOTO_SANS_URL]);
    // While a face is on its way, characters it draws wait for it.
    assert!(!draws('ạ'));
    assert!(fetch.requested().is_empty());
    fetch.answer(NOTO_SANS_URL, Ok(NOTO_SANS_LATIN));
    assert!(draws('ổ') && draws('ạ'));

    // ⌥ is in the second symbol face only: the first is never fetched for it.
    assert!(!draws('⌥'));
    assert_eq!(fetch.requested(), [SYMBOLS_2_URL]);
    fetch.answer(SYMBOLS_2_URL, Ok(SYMBOLS_2));
    assert!(draws('⌥') && draws('⌫'));
    assert!(fetch.requested().is_empty());

    // A letter no served face has fetches nothing.
    assert!(!draws('ẫ'));
    assert!(fetch.requested().is_empty(), "the emoji face is never a candidate for letters or keys");
}

#[trial]
fn bytes_fetched_under_one_integrity_never_reach_a_face_requiring_another() {
    const URL: &str = "fonts/shared.ttf";
    const STRICT: &str = "sha256-strict";
    // Two faces of one file: one needs no integrity check, the other a strict one.
    let faces = [
        served("Noto Sans Arabic", "Regular", &["Arab"], URL, 0),
        ServedFace { integrity: STRICT, ..served("Noto Sans Thai", "Regular", &["Thai"], URL, 0) },
    ];
    let installed = |ctx: &egui::Context, name: &str| ctx.fonts(|f| f.definitions().font_data.contains_key(name));

    let fetch = Arc::new(FakeFetch::default());
    let (ctx, mut draws) = web_context_with("en", &faces, &fetch);
    assert!(!draws('ก'));
    assert_eq!(fetch.requested(), [URL]);
    fetch.answer_checked(URL, Some(STRICT), Ok(ARABIC));
    draws('ก');
    assert!(installed(&ctx, "Noto Sans Thai Regular"));
    assert!(!installed(&ctx, "Noto Sans Arabic Regular"), "fetched under a strict check: the unchecked face does not take it");

    let fetch = Arc::new(FakeFetch::default());
    let (ctx, mut draws) = web_context_with("en", &faces, &fetch);
    assert!(!draws('ع'));
    fetch.answer_checked(URL, Some(""), Ok(ARABIC));
    assert!(draws('ع'));
    assert!(!installed(&ctx, "Noto Sans Thai Regular"), "fetched without a check: the strict face does not take it");
}

#[trial]
fn a_failed_download_moves_on_and_is_retried_later() {
    let fetch = Arc::new(FakeFetch::default());
    let (_ctx, mut draws) = web_context(&fetch);

    assert!(!draws('⌥'));
    assert_eq!(fetch.requested(), [SYMBOLS_2_URL]);
    fetch.answer(SYMBOLS_2_URL, Err("HTTP 503"));
    // The next face that draws it is fetched at once.
    assert!(!draws('⌥'));
    assert_eq!(fetch.requested(), [MATH_URL]);
    fetch.answer(MATH_URL, Err("HTTP 503"));
    assert!(!draws('⌥'));
    assert!(fetch.requested().is_empty(), "failed faces wait for their retry time");

    // Failures of one script don't stop another's face.
    assert!(!draws('ổ'));
    assert_eq!(fetch.requested(), [NOTO_SANS_URL]);
    fetch.answer(NOTO_SANS_URL, Ok(NOTO_SANS_LATIN));
    assert!(draws('ổ'));

    // At their retry time the failed faces are fetched again, in order.
    fetch.wait(1.0);
    assert!(!draws('⌥'));
    assert!(fetch.requested().is_empty());
    fetch.wait(1.0);
    assert!(!draws('⌥'));
    assert_eq!(fetch.requested(), [SYMBOLS_2_URL]);
    fetch.answer(SYMBOLS_2_URL, Ok(SYMBOLS_2));
    assert!(draws('⌥'));

    // A face that fails again waits twice as long.
    assert!(!draws('⇥'));
    assert_eq!(fetch.requested(), [MATH_URL]);
    fetch.answer(MATH_URL, Err("HTTP 503"));
    fetch.wait(3.0);
    assert!(!draws('⇥'));
    assert!(fetch.requested().is_empty());
    fetch.wait(1.0);
    assert!(!draws('⇥'));
    assert_eq!(fetch.requested(), [MATH_URL]);
}
