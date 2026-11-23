//! The words of a picture that could not be made.

craft_ui::words! {
    /// The words a painter shows for a failed texture (`LoadFailure::text`).
    pub struct TextureWords {
        missing: "textures.missing" = "The file is missing";
        unreadable: "textures.unreadable" = "The picture could not be read";
        panicked: "textures.panicked" = "The picture could not be made";
    }
}
