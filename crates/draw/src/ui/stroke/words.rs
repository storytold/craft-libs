//! The words of the stroke editor and its button ([`StrokeWords`]).

craft_ui::words! {
    /// The words of [`StrokeEditor`](super::StrokeEditor) and [`StrokeButton`](super::StrokeButton), English by default
    /// (PhotoCraft's and Photoshop's).
    pub struct StrokeWords {
        /// The popover's title, the opener's accessible name and tooltip.
        title: "strokeEditor.title" = "Stroke Options";
        /// The solid preset, and the opener's value for it.
        solid: "strokeEditor.solid" = "Solid";
        /// The dashed preset, and the opener's value for it.
        dashed: "strokeEditor.dashed" = "Dashed";
        /// The dotted preset, and the opener's value for it.
        dotted: "strokeEditor.dotted" = "Dotted";
        /// The opener's value for a pattern that is no preset.
        custom: "strokeEditor.custom" = "Custom";
        /// The opener's text while the stroke is off.
        no_color: "strokeEditor.noColor" = "No Color";
        /// The Alignment row.
        alignment: "strokeEditor.alignment" = "Alignment";
        /// The inside alignment.
        inside: "strokeEditor.inside" = "Inside";
        /// The centred alignment.
        center: "strokeEditor.center" = "Center";
        /// The outside alignment.
        outside: "strokeEditor.outside" = "Outside";
        /// The Caps row.
        caps: "strokeEditor.caps" = "Caps";
        /// The flat cap.
        butt: "strokeEditor.butt" = "Butt / Flat";
        /// The round cap.
        round_cap: "strokeEditor.roundCap" = "Round";
        /// The square cap.
        square: "strokeEditor.square" = "Square";
        /// The Corners row.
        corners: "strokeEditor.corners" = "Corners";
        /// The mitered corner.
        miter: "strokeEditor.miter" = "Miter";
        /// The round corner.
        round_join: "strokeEditor.roundJoin" = "Round";
        /// The bevelled corner.
        bevel: "strokeEditor.bevel" = "Bevel";
        /// The Miter limit row.
        miter_limit: "strokeEditor.miterLimit" = "Miter limit";
        /// The Opacity row.
        opacity: "strokeEditor.opacity" = "Opacity";
        /// Under the rows of an open path.
        open_paths: "strokeEditor.openPaths" = "Open paths use centered strokes";
        /// The check that turns the dashes on.
        dashed_line: "strokeEditor.dashedLine" = "Dashed line";
        /// Over the dash fields, for `DashUnit::Widths`.
        dash_widths: "strokeEditor.dashWidths" = "Dash / gap lengths × stroke width";
        /// Over the dash fields, for `DashUnit::Document`.
        dash_document: "strokeEditor.dashDocument" = "Dash / gap lengths in document units";
        /// A dash length field.
        dash: "strokeEditor.dash" = "Dash";
        /// A gap length field.
        gap: "strokeEditor.gap" = "Gap";
        /// The accessible name of the `n`th dash field.
        dash_number: template "strokeEditor.dashNumber" = "Dash {n}";
        /// The accessible name of the `n`th gap field.
        gap_number: template "strokeEditor.gapNumber" = "Gap {n}";
        /// The Dash offset row.
        dash_offset: "strokeEditor.dashOffset" = "Dash offset";
        /// Under the dash rows while the style fails `StrokeStyle::validate`.
        invalid: "strokeEditor.invalid" = "Use finite lengths and a positive pattern total";
    }
}
