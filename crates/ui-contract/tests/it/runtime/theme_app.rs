//! SG-6: a theme's `apps` table pins tokens for one app; the root env resolves them for the app the frame's host
//! names, and for no other.

use craft_host::{AppInfo, Host};
use craft_theme::{AppId, Appearance, ColorToken, Contrast, Mode, ModeChoice, Size, TextSize, ThemeDef, format, resolve};
use craft_ui::runtime::env::{Env, RootEnv};
use craft_ui::testing::{Frames, trial};
use std::sync::Arc;

/// A library theme whose `apps` table pins the canvas of the app `pinned` in both modes. The pin is a colour the
/// theme's own derivation never gives, so a canvas of that colour can only come from the pin.
const PINNED_THEME: &str = r##"{
    "schema": "craft-design-theme", "schema_version": 1,
    "id": "trial.app-pins", "name": "App pins",
    "seeds": {
        "neutral": "#535353", "accent": "#78ADF6",
        "status": { "good": "#85DC6A", "warn": "#E8B046", "bad": "#FF8080", "info": "#78ADF6" }
    },
    "apps": {
        "pinned": { "light": { "surface.canvas": "#123456" }, "dark": { "surface.canvas": "#123456" } }
    }
}"##;

/// The canvas colour the root env of one frame resolves from `def` in dark mode, with a host for the app `app`
/// (`None`: no host).
fn canvas(def: &Arc<ThemeDef>, app: Option<&'static str>) -> craft_theme::Rgba {
    let mut frames = Frames::new();
    frames.root(RootEnv::new().library_theme(def.clone()).mode(ModeChoice::Dark));
    // The host reaches the frame as an app's `FrameInput.host` does; Fixture takes no library theme, so the trial
    // drives `Frames`, which `Fixture::host` forwards to.
    if let Some(id) = app {
        frames.host(Arc::new(Host::builder(AppInfo { id, name: "Trial", version: "0" }).build()));
    }
    let mut seen = None;
    frames.frame(Vec::new(), |ui| seen = Some(Env::of(ui).tokens.tokens().color(ColorToken::SurfaceCanvas)));
    seen.expect("a pass ran")
}

/// Tokens resolved for a host whose app id has pins in the theme differ from those without: the pins apply to that
/// app only. Another app, no host and an id that is not a valid `AppId` all resolve as no app.
#[trial]
fn an_apps_theme_pins_apply_only_to_that_app() {
    let def = Arc::new(format::parse(PINNED_THEME.as_bytes()).expect("the trial's theme parses").theme);
    let appearance =
        Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::default(), text_size: TextSize::default(), reduced_motion: false };
    let app = AppId::parse("pinned").expect("a valid app id");
    let with_pins = resolve(&def, appearance, Some(&app)).tokens.color(ColorToken::SurfaceCanvas);
    let without = resolve(&def, appearance, None).tokens.color(ColorToken::SurfaceCanvas);
    assert_ne!(with_pins, without, "the theme's pin changes the app's canvas");

    assert_eq!(canvas(&def, Some("pinned")), with_pins, "the host's app gets its pins");
    assert_eq!(canvas(&def, None), without, "with no host no app's pins apply");
    assert_eq!(canvas(&def, Some("other")), without, "another app does not get the pinned app's pins");
    assert_eq!(canvas(&def, Some("Pinned App")), without, "an id that does not parse resolves like no app");
    // The thread keeps resolved tokens by their inputs: the pinned app after other apps still gets its own.
    assert_eq!(canvas(&def, Some("pinned")), with_pins, "the pins follow the host every time");
}
