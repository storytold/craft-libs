//! Built-in words of the tile grid family: one set, ids written `tileGrid.<type><Name>`.

craft_ui::words! {
    /// The words of a tile and of the thumbnail slider.
    pub struct TileWords {
        /// A tile's accessible name: `{name}`, then `{details}`, the parts that apply joined with a comma.
        tile_name: template "tileGrid.tileName" = "{name}, {details}";
        picked: "tileGrid.tilePicked" = "picked";
        rejected: "tileGrid.tileRejected" = "rejected";
        /// `{label}` is the app's name for the colour label.
        label: template "tileGrid.tileLabel" = "{label} label";
        offline: "tileGrid.tileOffline" = "Offline";
        /// An offline tile's detail: `{offline}` is the word above, `{message}` the app's text.
        offline_with: template "tileGrid.tileOfflineWith" = "{offline}: {message}";
        unreadable: "tileGrid.tileUnreadable" = "Unreadable";
        failed: "tileGrid.tileFailed" = "Preview failed";
        /// The name of a tile's check box: `{name}` is the tile's name.
        select: template "tileGrid.tileSelect" = "Select {name}";
        /// The name of a tile's star rating: `{name}` is the tile's name.
        rating: template "tileGrid.tileRating" = "Rating of {name}";
        /// The slider's end buttons.
        smaller: "tileGrid.thumbnailSliderSmaller" = "Smaller Thumbnails";
        larger: "tileGrid.thumbnailSliderLarger" = "Larger Thumbnails";
    }
}
