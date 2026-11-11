//! The browsing widgets.
//!
//! - `bin`: `MediaBin`, the project or media panel.
//! - `freebin`: `FreeBin`, the free-layout collection of cards.
//! - `places`: `PlacesTree`, the folder tree.
//! - `compare`: `CompareTile` and the compare arrangements.
//! - `relink`: `LinkList` and `RelinkDialog`.

pub mod bin;
pub mod compare;
pub mod freebin;
pub mod places;
pub mod relink;

pub use bin::{BinEvent, BinState, BinWords, MediaBin};
pub use compare::{CompareTile, CompareWords};
pub use freebin::{FreeBin, FreeBinMove, FreeBinOutput, FreeformCard};
pub use places::{PlacesEvent, PlacesTree, PlacesWords};
pub use relink::{LinkList, LinkListWords, RelinkDialog, RelinkWords, StatusBadge};

/// The stories and trials this crate registers for the integration binary.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}
