//! `CompareTile`, `CompareCaption` and the compare arrangements (`ComparePair`, `survey_columns`, `survey_cell`).
//!
//! ```ignore
//! let [left, right] = ComparePair::new(space).rtl(rtl).rects(area);
//! let select = CompareTile::new("Select", Bind::bound(&mut state.view), photo_a.image())   // one view for both
//!     .caption(CompareCaption::new(tr("Select"), &photo_a.name).rating(photo_a.rating).flag(photo_a.flag))
//!     .active(active == 0)
//!     .place(ui, left);
//! if select.clicked {
//!     active = 0;
//! }
//! ```

mod arrange;
mod caption;
#[cfg(feature = "screen-testing")]
mod stories;
mod style;
mod tile;

pub use arrange::{ArrangeSpace, ComparePair, survey_cell, survey_columns};
pub use caption::CompareCaption;
pub use style::CompareTileStyle;
pub use tile::{CompareImage, CompareOutput, CompareTile, RenderState};

craft_ui::words! {
    /// The compare tile's words, English by default.
    #[non_exhaustive]
    pub struct CompareWords {
        /// The accessible description of a tile whose render is running.
        rendering: "compareTile.rendering" = "Rendering…";
        /// The message of a tile whose render failed, and its accessible description.
        render_failed: "compareTile.renderFailed" = "Render failed";
        /// The tile's accessible name: `{label}`: `{name}`, then `{details}` (rating, flag), each led by a comma.
        tile_name: template "compareTile.tileName" = "{label}: {name}{details}";
        /// One entry of `{details}`: the pick flag.
        picked: "compareTile.picked" = "picked";
        /// One entry of `{details}`: the reject flag.
        rejected: "compareTile.rejected" = "rejected";
        /// One entry of `{details}`: the rating.
        stars: count "compareTile.stars" = "{n} star" | "{n} stars";
    }
}
