//! The style cache, the env key and app style replacements.

use craft_theme::{Contrast, ContrastChoice, ControlSize, Mode, ModeChoice, Variant};
use craft_ui::runtime::env::{Env, RootEnv, system_appearance};
use craft_ui::style::{ComponentStyle, Metrics, StyleCx, StyleKey};
use craft_ui::testing::style::{report_os_contrast, root_token_resolves, style_of, tokens, with_cx};
use craft_ui::testing::{Frames, trial};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FirstKey;
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SecondKey;
// derive output
impl StyleKey for FirstKey {}
// derive output
impl StyleKey for SecondKey {}

/// One style type shared by two components.
#[derive(Clone, PartialEq, Debug)]
struct SharedStyle {
    built_by: &'static str,
}

// derive output
impl ComponentStyle for SharedStyle {
    fn metrics(&self) -> Metrics {
        Metrics::new()
    }
}

#[trial]
fn two_components_sharing_a_style_type_get_their_own_style() {
    static BUILDS: AtomicUsize = AtomicUsize::new(0);
    fn first(_: &StyleCx<'_>, _: FirstKey) -> SharedStyle {
        BUILDS.fetch_add(1, Ordering::SeqCst);
        SharedStyle { built_by: "first" }
    }
    fn second(_: &StyleCx<'_>, _: SecondKey) -> SharedStyle {
        BUILDS.fetch_add(1, Ordering::SeqCst);
        SharedStyle { built_by: "second" }
    }
    let frames = Frames::new();
    let tokens = tokens(Mode::Light, Contrast::Standard);
    with_cx(&frames.ctx, &tokens, Variant::SECONDARY, ControlSize::Md, |cx| {
        for _ in 0..3 {
            assert_eq!(style_of(&frames.ctx, cx, FirstKey, first).built_by, "first");
            assert_eq!(style_of(&frames.ctx, cx, SecondKey, second).built_by, "second");
        }
    });
    assert_eq!(BUILDS.load(Ordering::SeqCst), 2, "each key built once, every later read hit the cache");
}

/// The OS's appearance reaches the root tokens through the system reader: the colour scheme egui's integration
/// reports and a contrast change each resolve the root tokens once and raise the generation; frames with nothing new
/// resolve nothing and leave it.
#[trial]
fn the_os_appearance_re_resolves_the_root_tokens_only_when_it_changes() {
    let mut frames = Frames::new();
    frames.root(RootEnv::new().mode(ModeChoice::System).contrast(ContrastChoice::System));
    frames.frame(vec![], |_| {});
    let (generation, resolves) = (system_appearance(&frames.ctx).generation, root_token_resolves(&frames.ctx));

    let mut mode = None;
    frames.frame(vec![egui::Event::SystemTheme(egui::Theme::Dark)], |ui| mode = Some(Env::of(ui).view().mode()));
    assert_eq!(mode, Some(Mode::Dark), "the reported scheme reaches the pass that carries it");
    report_os_contrast(&frames.ctx, Contrast::High);
    frames.frame(vec![], |_| {});
    let os = system_appearance(&frames.ctx);
    assert_eq!(os.appearance.contrast, Some(Contrast::High));
    assert_eq!(RootEnv::new().contrast(ContrastChoice::System).resolved(&frames.ctx).contrast, Contrast::High);
    assert_eq!(os.generation, generation + 2, "one step per change");
    assert_eq!(root_token_resolves(&frames.ctx), resolves + 2, "each change resolves the root tokens once");

    frames.frame(vec![egui::Event::SystemTheme(egui::Theme::Dark)], |_| {});
    frames.idle(3, |_| {});
    assert_eq!(system_appearance(&frames.ctx).generation, os.generation, "a repeated scheme is no change");
    assert_eq!(root_token_resolves(&frames.ctx), resolves + 2, "frames with nothing new resolve nothing");
}
