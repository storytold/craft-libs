//! Served faces in a browser: a character no installed face has fetches the one served face that
//! has it, a character no served face has fetches nothing, and a failed face is fetched again
//! later.
//!
//! Start `tests/web/serve_fonts.py` (127.0.0.1:8137; another address through
//! `CRAFT_UI_FONT_SERVER` when building), then run
//!
//! ```text
//! cargo test -p craft-ui --target wasm32-unknown-unknown --test web_fonts
//! ```
//!
//! with `wasm-bindgen-test-runner` as the target's runner: headless through a WebDriver, or with
//! `NO_HEADLESS=1`, which serves the test page for any browser to open.
#![cfg(target_arch = "wasm32")]

use craft_ui::fonts::{I18nScripts, ServedFace, SystemFonts, UiFonts};
use egui::{FontDefinitions, FontFamily, FontId};
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

fn url(path: &str) -> &'static str {
    let server = option_env!("CRAFT_UI_FONT_SERVER").unwrap_or("http://127.0.0.1:8137");
    Box::leak(format!("{server}{path}").into_boxed_str())
}

fn face(family: &'static str, scripts: &'static [&'static str], path: &str, coverage: Vec<u8>) -> ServedFace {
    ServedFace { family, style: "Regular", scripts, index: 0, url: url(path), size: 0, integrity: "", coverage: coverage.leak() }
}

fn coverage_of(font: &[u8]) -> Vec<u8> {
    craft_fonts_build::coverage::of_face(font, 0).expect("a font")
}

async fn sleep(millis: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let window = web_sys::window().expect("a browser window");
        window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis).expect("setTimeout");
    });
    JsFuture::from(promise).await.expect("a timer");
}

/// Run passes until the context draws `c`, for up to `seconds`.
async fn draws_within(ctx: &egui::Context, c: char, seconds: f64) -> bool {
    let start = js_sys::Date::now();
    loop {
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        if ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(14.0), c)) {
            return true;
        }
        if js_sys::Date::now() - start > seconds * 1000.0 {
            return false;
        }
        sleep(50).await;
    }
}

/// The paths the font server was asked for, in order.
async fn requested() -> Vec<String> {
    let window = web_sys::window().expect("a browser window");
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str(url("/log"))).await.expect("the font server").unchecked_into();
    let paths = JsFuture::from(response.json().expect("JSON")).await.expect("JSON");
    js_sys::Array::from(&paths).iter().filter_map(|path| path.as_string()).collect()
}

#[wasm_bindgen_test]
async fn a_missing_character_fetches_the_face_that_has_it_and_a_failed_one_is_retried() {
    let encode = craft_fonts_build::coverage::encode;
    let served = [
        face("Noto Sans", &["Latn", "Cyrl", "Grek", "Deva"], "/fonts/NotoSans-test.ttf", coverage_of(include_bytes!("fonts/NotoSans-test.ttf"))),
        face("Noto Color Emoji", &["Zsye"], "/missing/NotoColorEmoji.ttf", encode(&[(0x1F300, 0x1FAFF)])),
        // ⌃ ⎋, and ⌥ ⌫ ⏎: the keyboard keys are split between the two symbol faces.
        face("Noto Sans Symbols", &["Zsym"], "/fonts/NotoSansSymbols-test.ttf", coverage_of(include_bytes!("fonts/NotoSansSymbols-test.ttf"))),
        // Fails the first time.
        face("Noto Sans Symbols 2", &["Zsym"], "/flaky/NotoSansSymbols2-test.ttf", coverage_of(include_bytes!("fonts/NotoSansSymbols2-test.ttf"))),
        face("Noto Sans Math", &["Zmth", "Zsym"], "/missing/NotoSansMath-Regular.ttf", encode(&[(0x2200, 0x22FF)])),
    ];
    let defaults = FontDefinitions::default();
    let ubuntu = (**defaults.font_data.get("Ubuntu-Light").expect("egui default_fonts")).clone();
    let ctx = egui::Context::default();
    UiFonts::new(&I18nScripts)
        .app_face("ui", ubuntu, &[FontFamily::Proportional])
        .served_faces(&served)
        .ui_language("en")
        .system_fonts(SystemFonts::Off)
        .stack()
        .install(&ctx);

    assert!(draws_within(&ctx, 'ổ', 5.0).await, "Vietnamese from Noto Sans");
    assert!(draws_within(&ctx, '⌃', 5.0).await, "a key from Noto Sans Symbols");
    // Only Symbols 2 has ⌥: it fails, and is fetched again after two seconds.
    assert!(draws_within(&ctx, '⌥', 10.0).await, "a key from Noto Sans Symbols 2 after a failure");
    assert!(draws_within(&ctx, '⌫', 1.0).await);
    // No face has ẫ.
    assert!(!draws_within(&ctx, 'ẫ', 1.0).await);
    assert_eq!(
        requested().await,
        ["/fonts/NotoSans-test.ttf", "/fonts/NotoSansSymbols-test.ttf", "/flaky/NotoSansSymbols2-test.ttf", "/flaky/NotoSansSymbols2-test.ttf"],
        "each character fetches only the face that has it",
    );
}
