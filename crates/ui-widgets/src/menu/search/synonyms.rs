//! The groups of words that mean the same thing when searching commands.

use crate::runtime::env::Env;

craft_ui::words! {
    /// The synonym groups of the command search: each row is the group's English words joined by ", "; a catalog may
    /// add the group's words in its language the same way.
    pub struct SearchWords {
        remove: "search.remove" = "remove, delete, erase, clear";
        background: "search.background" = "background, bg";
        image: "search.image" = "image, picture, photo, img, bitmap";
        text: "search.text" = "text, type";
        reflect: "search.reflect" = "reflect, flip";
        unite: "search.unite" = "unite, combine";
        trace: "search.trace" = "trace, vectorize";
        color: "search.color" = "color, colour";
        gray: "search.gray" = "gray, grey";
        grayscale: "search.grayscale" = "grayscale, greyscale";
        center: "search.center" = "center, centre";
        duplicate: "search.duplicate" = "duplicate, clone";
        zoom: "search.zoom" = "zoom, magnify";
        rotate: "search.rotate" = "rotate, turn, spin";
        scale: "search.scale" = "scale, resize";
        opacity: "search.opacity" = "opacity, transparency";
        font: "search.font" = "font, typeface";
        preferences: "search.preferences" = "preferences, settings, prefs";
        artboard: "search.artboard" = "artboard, canvas";
        swatch: "search.swatch" = "swatch, palette";
        eyedropper: "search.eyedropper" = "eyedropper, dropper";
    }
}

/// The groups of words that mean the same thing, in English and in the UI language.
pub struct Synonyms {}

impl Synonyms {
    /// The groups of `env`: the 21 English groups, each joined with its row in the UI language when a catalog has one.
    pub fn of(env: &Env) -> Synonyms {
        let _ = env;
        todo!("mn.M23")
    }
}
