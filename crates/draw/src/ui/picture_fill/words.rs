//! The picture fill editor's words, English by default (PowerPoint's, as DeckCraft copies them).

craft_ui::words! {
    /// The words of [`PictureFillEditor`](super::PictureFillEditor).
    pub struct PictureFillWords {
        /// The picture source row.
        source: "pictureFill.source" = "Picture source";
        /// The source row's value while no picture is picked.
        no_source: "pictureFill.noSource" = "None";
        /// The button that picks a picture file.
        insert: "pictureFill.insert" = "Insert…";
        /// The check that switches to tiling.
        tile: "pictureFill.tile" = "Tile picture as texture";
        /// The tile offset across.
        offset_x: "pictureFill.offsetX" = "Offset X";
        /// The tile offset down.
        offset_y: "pictureFill.offsetY" = "Offset Y";
        /// The tile scale across.
        scale_x: "pictureFill.scaleX" = "Scale X";
        /// The tile scale down.
        scale_y: "pictureFill.scaleY" = "Scale Y";
        /// The alignment row.
        alignment: "pictureFill.alignment" = "Alignment";
        /// An alignment.
        top_left: "pictureFill.topLeft" = "Top left";
        /// An alignment.
        top: "pictureFill.top" = "Top";
        /// An alignment.
        top_right: "pictureFill.topRight" = "Top right";
        /// An alignment.
        left: "pictureFill.left" = "Left";
        /// An alignment.
        center: "pictureFill.center" = "Center";
        /// An alignment.
        right: "pictureFill.right" = "Right";
        /// An alignment.
        bottom_left: "pictureFill.bottomLeft" = "Bottom left";
        /// An alignment.
        bottom: "pictureFill.bottom" = "Bottom";
        /// An alignment.
        bottom_right: "pictureFill.bottomRight" = "Bottom right";
        /// The mirror row.
        mirror: "pictureFill.mirror" = "Mirror type";
        /// No mirroring.
        mirror_none: "pictureFill.mirrorNone" = "None";
        /// Mirroring left to right.
        horizontal: "pictureFill.horizontal" = "Horizontal";
        /// Mirroring top to bottom.
        vertical: "pictureFill.vertical" = "Vertical";
        /// Mirroring both ways.
        both: "pictureFill.both" = "Both";
        /// The stretch inset from the left.
        offset_left: "pictureFill.offsetLeft" = "Offset left";
        /// The stretch inset from the right.
        offset_right: "pictureFill.offsetRight" = "Offset right";
        /// The stretch inset from the top.
        offset_top: "pictureFill.offsetTop" = "Offset top";
        /// The stretch inset from the bottom.
        offset_bottom: "pictureFill.offsetBottom" = "Offset bottom";
        /// The check that turns the picture with the shape.
        rotate: "pictureFill.rotate" = "Rotate with shape";
        /// The transparency slider.
        transparency: "pictureFill.transparency" = "Transparency";
    }
}
