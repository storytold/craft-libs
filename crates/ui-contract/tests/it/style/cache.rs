//! The style cache, the env key and app style replacements.

use craft_theme::{Contrast, ControlSize, Mode, Variant};
use craft_ui::style::{ComponentStyle, Metrics, StyleCx, StyleKey};
use craft_ui::testing::style::{style_of, tokens, with_cx};
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
