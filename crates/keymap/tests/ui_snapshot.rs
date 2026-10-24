#![cfg(feature = "snapshot")]
//! Screenshots of the Keyboard Shortcuts dialog for review, not a golden comparison. With
//! `CRAFT_KEYMAP_SNAPSHOT_DIR` set, writes PNGs of a few states in light and dark at 1× and 2×;
//! without it the test does nothing.
//!
//! `CRAFT_KEYMAP_SNAPSHOT_DIR=/tmp/shots cargo test -p craft-keymap --features snapshot --test ui_snapshot`
//!
//! egui's default fonts have no ⇧ ⌥ ⌃ or arrows. Apps load their own fonts; here the pictures add
//! fallback fonts when they are found: the file in `CRAFT_KEYMAP_SNAPSHOT_FONT`, DejaVu Sans
//! from the usual Linux path, and BIZ UDPGothic from a craft-fonts checkout in `CRAFT_FONTS_DIR`
//! (⇧ ⌘ and arrows, not ⌥ ⌃). Without any of them those glyphs show as boxes.

use std::path::PathBuf;

use craft_keymap::ui::{DialogState, English, Options, ShortcutsDialog, Text};
use craft_keymap::{ContextRule, Flavor, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, Keymap, Overrides, Platform, Profiles};
use egui::{Key as EguiKey, Modifiers, Theme, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ui_fixture::{EnglishRtl, Pseudo, fixture, with_long_label};

mod ui_fixture;

struct Fx {
    km: Keymap,
    text: Box<dyn Text>,
    options: Options,
    state: DialogState,
    sets: Option<(Profiles, Vec<(&'static str, Overrides)>)>,
}

impl Fx {
    fn dialog(&mut self) -> (ShortcutsDialog<'_>, &mut DialogState) {
        let Fx { km, text, options, state, sets } = self;
        let d = ShortcutsDialog::new(km, &**text).options(options);
        let d = match sets {
            Some((p, b)) => d.sets(p, b),
            None => d,
        };
        (d, state)
    }
}

/// A named state of the dialog: window size and what to do before the picture.
struct Case {
    name: &'static str,
    size: Vec2,
    platform: Platform,
    /// Show named sets.
    sets: bool,
    /// Show the on-screen keyboard.
    keyboard: bool,
    words: Words,
    /// Add a command with a very long label and name the user set at length.
    long: bool,
    act: fn(&mut Harness<'_, Fx>),
}

#[derive(Clone, Copy)]
enum Words {
    English,
    /// English laid out right to left.
    Rtl,
    /// The pseudo translation of the tests, left to right.
    Pseudo,
}

impl Case {
    const fn new(name: &'static str, width: f32, height: f32, act: fn(&mut Harness<'_, Fx>)) -> Case {
        Case { name, size: Vec2::new(width, height), platform: Platform::Mac, sets: false, keyboard: false, words: Words::English, long: false, act }
    }

    const fn windows(mut self) -> Case {
        self.platform = Platform::Windows;
        self
    }

    const fn sets(mut self) -> Case {
        self.sets = true;
        self
    }

    const fn keyboard(mut self) -> Case {
        self.keyboard = true;
        self
    }

    const fn words(mut self, words: Words) -> Case {
        self.words = words;
        self
    }

    const fn long(mut self) -> Case {
        self.long = true;
        self
    }
}

/// Fallback fonts with the key glyphs, when this machine has them.
fn fonts() -> egui::FontDefinitions {
    let mut defs = egui::FontDefinitions::default();
    let mut files: Vec<PathBuf> = std::env::var_os("CRAFT_KEYMAP_SNAPSHOT_FONT").map(PathBuf::from).into_iter().collect();
    files.push("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into());
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR") {
        files.push(PathBuf::from(dir).join("fonts/biz-ud-pgothic/BIZUDPGothic-Regular.ttf"));
    }
    for (i, file) in files.iter().enumerate() {
        let Ok(bytes) = std::fs::read(file) else { continue };
        let name = format!("fallback-{i}");
        defs.font_data.insert(name.clone(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        for family in defs.families.values_mut() {
            family.push(name.clone());
        }
    }
    defs
}

fn foreign_review(h: &mut Harness<'_, Fx>) {
    let read = |text: &str| ForeignKey::read(Some("Default"), text, Flavor::Windows);
    let set = ForeignSet::new(vec![
        ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Ctrl+Alt+S")).at_line(3),
        ForeignEntry::new(Some("File"), ["Frobnicate"]).with_key(read("Ctrl+F")).at_line(4),
        ForeignEntry::new(Some("File"), ["Export…"]).with_key(read("Ctrl+Blorp")).at_line(5),
        ForeignEntry::new(Some("Object"), ["Arrange", "Bring to Front"]).with_key(read("Num 7")).at_line(6),
        ForeignEntry::new(Some("File"), ["Sve Az"]).with_key(read("Ctrl+F9")).at_line(7),
    ])
    .with_name("Theirs");
    let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], suggest: true, ..ImportOptions::default() };
    let (d, state) = h.state_mut().dialog();
    d.review_import(state, set, options);
    h.run();
}

fn click(h: &mut Harness<'_, Fx>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn select(h: &mut Harness<'_, Fx>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

const CASES: &[Case] = &[
    Case::new("selected", 1000.0, 640.0, |h| select(h, "Save As…")),
    Case::new("conflict", 1000.0, 640.0, |h| {
        click(h, "Export…: ⌘E");
        h.key_press_modifiers(Modifiers::COMMAND, EguiKey::S);
        h.run();
    }),
    Case::new("recording", 1000.0, 640.0, |h| click(h, "Save: Ctrl+S")).windows(),
    Case::new("narrow", 560.0, 640.0, |h| select(h, "Save")),
    Case::new("sets", 1000.0, 640.0, |h| {
        click(h, "Save: ⌘S");
        h.key_press(EguiKey::F2);
        h.run();
        click(h, "New Set…");
    })
    .sets(),
    Case::new("sets-narrow", 560.0, 640.0, |h| click(h, "More")).sets(),
    Case::new("keyboard", 1100.0, 720.0, |h| {
        click(h, "Save As…");
        click(h, "⌘ Command");
        click(h, "⌘S: Save");
    })
    .keyboard(),
    Case::new("keyboard-windows", 860.0, 640.0, |h| click(h, "L: Loop (Timeline)")).windows().keyboard(),
    Case::new("review", 1000.0, 640.0, foreign_review).sets(),
    Case::new("long", 1000.0, 640.0, |h| {
        let state = &mut h.state_mut().state;
        state.select(&craft_keymap::CommandRef::from("object.distribute"));
        h.run();
    })
    .sets()
    .long(),
    Case::new("long-narrow", 560.0, 640.0, |h| {
        h.state_mut().state.select(&craft_keymap::CommandRef::from("object.distribute"));
        h.run();
    })
    .sets()
    .long(),
    Case::new("rtl", 1000.0, 640.0, |h| {
        click(h, "Export…: ⌘E");
        h.key_press_modifiers(Modifiers::COMMAND, EguiKey::S);
        h.run();
    })
    .sets()
    .long()
    .words(Words::Rtl),
    Case::new("rtl-narrow", 560.0, 640.0, |h| select(h, "Bold")).words(Words::Rtl).long(),
    Case::new("rtl-keyboard", 1100.0, 720.0, |h| click(h, "L: Loop (Timeline)")).keyboard().words(Words::Rtl),
    Case::new("rtl-review", 1000.0, 640.0, foreign_review).sets().words(Words::Rtl),
    Case::new("pseudo", 1100.0, 720.0, |h| {
        h.state_mut().state.select(&craft_keymap::CommandRef::from("file.save"));
        h.run();
    })
    .sets()
    .keyboard()
    .long()
    .words(Words::Pseudo),
    Case::new("pseudo-narrow", 560.0, 640.0, |h| {
        h.state_mut().state.select(&craft_keymap::CommandRef::from("object.distribute"));
        h.run();
    })
    .sets()
    .long()
    .words(Words::Pseudo),
];

#[test]
fn dialog_screenshots() {
    let Some(dir) = std::env::var_os("CRAFT_KEYMAP_SNAPSHOT_DIR").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    // `CRAFT_KEYMAP_SNAPSHOT_ONLY=keyboard` renders only the cases whose name starts with it.
    let only = std::env::var("CRAFT_KEYMAP_SNAPSHOT_ONLY").unwrap_or_default();
    let fonts = fonts();
    for case in CASES.iter().filter(|c| c.name.starts_with(&only)) {
        for (theme_name, theme) in [("light", Theme::Light), ("dark", Theme::Dark)] {
            for scale in [1.0, 2.0] {
                let sets = case.sets.then(|| {
                    let mut p = Profiles::default();
                    let builtins = vec![("Fixture Defaults", Overrides::new())];
                    if case.long {
                        let name = "My Shortcuts for Long Evenings of Retouching Photographs";
                        if let Err(e) = p.save_as(name, Overrides::new(), &builtins).and_then(|n| p.select(&n, &builtins)) {
                            panic!("{name}: {e}");
                        }
                    }
                    (p, builtins)
                });
                let options = Options::default().with(|o| o.keyboard = case.keyboard);
                let km = if case.long { with_long_label(fixture(case.platform)) } else { fixture(case.platform) };
                let text: Box<dyn Text> = match case.words {
                    Words::English => Box::new(English),
                    Words::Rtl => Box::new(EnglishRtl),
                    Words::Pseudo => Box::new(Pseudo::default()),
                };
                let fx = Fx { km, text, options, state: DialogState::new(), sets };
                let mut h = Harness::builder().with_size(case.size).with_pixels_per_point(scale).with_theme(theme).wgpu().build_ui_state(
                    |ui, fx: &mut Fx| {
                        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
                            let (d, state) = fx.dialog();
                            d.ui(ui, state);
                        });
                    },
                    fx,
                );
                h.ctx.set_fonts(fonts.clone());
                h.run();
                (case.act)(&mut h);
                let image = h.render().unwrap_or_else(|e| panic!("render {}: {e}", case.name));
                let path = dir.join(format!("shortcuts-{}-{theme_name}@{scale}x.png", case.name));
                image.save(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            }
        }
    }
}
