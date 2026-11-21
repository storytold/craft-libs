//! The groups of words that mean the same thing when searching commands.

use super::index::is_one_word;

/// The most characters a word of a catalog row may have.
const MAX_CHARS: usize = 20;

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
pub struct Synonyms {
    groups: Vec<Vec<String>>,
}

impl Synonyms {
    /// The groups of `words`: the 21 English groups, each joined with its row in the UI language when a catalog has one.
    /// A row holds single words of at most 20 characters; any other piece of it is left out.
    pub fn of(words: &SearchWords) -> Synonyms {
        let rows = [
            words.remove.as_str(),
            words.background.as_str(),
            words.image.as_str(),
            words.text.as_str(),
            words.reflect.as_str(),
            words.unite.as_str(),
            words.trace.as_str(),
            words.color.as_str(),
            words.gray.as_str(),
            words.grayscale.as_str(),
            words.center.as_str(),
            words.duplicate.as_str(),
            words.zoom.as_str(),
            words.rotate.as_str(),
            words.scale.as_str(),
            words.opacity.as_str(),
            words.font.as_str(),
            words.preferences.as_str(),
            words.artboard.as_str(),
            words.swatch.as_str(),
            words.eyedropper.as_str(),
        ];
        Synonyms { groups: rows.iter().map(|row| row.split(',').filter_map(single).collect()).collect() }
    }

    /// The words of each group.
    pub(super) fn groups(&self) -> &[Vec<String>] {
        &self.groups
    }
}

/// `piece` lowercased, when it is one word the index can look up.
fn single(piece: &str) -> Option<String> {
    let word = piece.trim().to_lowercase();
    (!word.is_empty() && word.chars().count() <= MAX_CHARS && is_one_word(&word)).then_some(word)
}
