//! The pattern fill editor's words, English by default (Photoshop's and PowerPoint's, no colons).

craft_ui::words! {
    /// The words of [`PatternFillEditor`](super::PatternFillEditor) and [`PatternFillDialog`](super::PatternFillDialog).
    pub struct PatternWords {
        title: "patternFill.title" = "Pattern Fill";
        pattern: "patternFill.pattern" = "Pattern";
        no_patterns: "patternFill.noPatterns" = "No patterns";
        snap_to_origin: "patternFill.snapToOrigin" = "Snap to Origin";
        angle: "patternFill.angle" = "Angle";
        scale: "patternFill.scale" = "Scale";
        link: "patternFill.link" = "Link with Layer";
        foreground: "patternFill.foreground" = "Foreground";
        background: "patternFill.background" = "Background";
    }
}
