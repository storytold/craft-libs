//! A library theme the root names resolves from its own definition.

use craft_theme::{Appearance, ColorToken, Contrast, Mode, ModeChoice, Size, TextSize, ThemeId, builtins, resolve};
use craft_ui::runtime::env::{Env, RootEnv};
use craft_ui::testing::{Frames, trial};
use std::sync::Arc;

#[trial]
fn a_library_theme_resolves_from_its_definition_and_an_unknown_id_from_the_default() {
    let studio = builtins().iter().find(|theme| theme.id.as_str() == "craft.studio").expect("a built-in").clone();
    let mut mine = studio.clone();
    mine.id = ThemeId::parse("mine.studio").expect("a valid id");
    let appearance =
        Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::default(), text_size: TextSize::default(), reduced_motion: false };
    let expected = resolve(&mine, appearance, None).tokens.color(ColorToken::SurfacePanel);

    let panel = |root: RootEnv| {
        let mut frames = Frames::new();
        frames.root(root);
        let mut seen = None;
        frames.frame(Vec::new(), |ui| seen = Some(Env::of(ui).tokens.tokens().color(ColorToken::SurfacePanel)));
        seen.expect("a pass ran")
    };
    let dark = |root: RootEnv| root.mode(ModeChoice::Dark);
    assert_eq!(panel(dark(RootEnv::new().library_theme(Arc::new(mine.clone())))), expected, "the root's own definition");
    let unknown = panel(dark(RootEnv::new().theme(mine.id.clone())));
    assert_ne!(unknown, expected, "an id with no definition and no built-in shows the suite default");
    let renamed = panel(dark(RootEnv::new().library_theme(Arc::new(mine.clone())).theme(mine.id.clone())));
    assert_eq!(renamed, unknown, "naming a theme by id drops the earlier definition");
}
