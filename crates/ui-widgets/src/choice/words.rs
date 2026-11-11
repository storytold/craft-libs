//! Built-in words of the choice family.

craft_ui::words! {
    /// The words of the rating's stars.
    pub struct ChoiceWords {
        unrated: "choice.ratingUnrated" = "Unrated";
        swatch_mixed: "choice.swatchMixed" = "Mixed";
        stars: count "choice.ratingStars" = "{n} star" | "{n} stars";
    }
}
