//! [`FillWords`]: Content-Aware Fill's words. OK and Cancel are the common words, Apply the dialog family's and
//! Reset the parameter dialogs'. The tools' names are their commands' labels ([`FillCommands`](super::FillCommands)).

craft_ui::words! {
    /// Content-Aware Fill's words, English by default (Photoshop's names).
    pub struct FillWords {
        /// The title's name and the settings column's header.
        name: "fill.name" = "Content-Aware Fill";
        tools: "fill.tools" = "Tools";
        options: "fill.options" = "Options";
        add: "fill.add" = "Add";
        subtract: "fill.subtract" = "Subtract";
        size: "fill.size" = "Size";
        new_selection: "fill.newSelection" = "New selection";
        add_to_selection: "fill.addToSelection" = "Add to selection";
        subtract_from_selection: "fill.subtractFromSelection" = "Subtract from selection";
        intersect_with_selection: "fill.intersectWithSelection" = "Intersect with selection";
        /// The caption of the Expand and Contract amount in the options bar.
        grow_by: "fill.growBy" = "By";
        expand: "fill.expand" = "Expand";
        contract: "fill.contract" = "Contract";
        /// The accessible name of the fill area's outline on the canvas.
        fill_area: "fill.fillArea" = "Fill area";
        sampling_overlay: "fill.samplingOverlay" = "Sampling Area Overlay";
        show_sampling_area: "fill.showSamplingArea" = "Show Sampling Area";
        opacity: "fill.opacity" = "Opacity";
        color: "fill.color" = "Color";
        indicates: "fill.indicates" = "Indicates";
        sampling_area: "fill.samplingArea" = "Sampling Area";
        excluded_area: "fill.excludedArea" = "Excluded Area";
        sampling_options: "fill.samplingOptions" = "Sampling Area Options";
        auto: "fill.auto" = "Auto";
        rectangular: "fill.rectangular" = "Rectangular";
        custom: "fill.custom" = "Custom";
        sample_all_layers: "fill.sampleAllLayers" = "Sample All Layers";
        fill_settings: "fill.fillSettings" = "Fill Settings";
        color_adaptation: "fill.colorAdaptation" = "Color Adaptation";
        rotation_adaptation: "fill.rotationAdaptation" = "Rotation Adaptation";
        none: "fill.none" = "None";
        default: "fill.default" = "Default";
        low: "fill.low" = "Low";
        medium: "fill.medium" = "Medium";
        high: "fill.high" = "High";
        very_high: "fill.veryHigh" = "Very High";
        full: "fill.full" = "Full";
        scale: "fill.scale" = "Scale";
        mirror: "fill.mirror" = "Mirror";
        output_settings: "fill.outputSettings" = "Output Settings";
        output_to: "fill.outputTo" = "Output To";
        current_layer: "fill.currentLayer" = "Current Layer";
        new_layer: "fill.newLayer" = "New Layer";
        duplicate_layer: "fill.duplicateLayer" = "Duplicate Layer";
        /// The stroke limit's sentence ([`FillLimit::Strokes`](super::FillLimit::Strokes)).
        stroke_limit: count "fill.limitStrokes" = "at most {n} sampling stroke: undo some, or reset the sampling area" | "at most {n} sampling strokes: undo some, or reset the sampling area";
        /// The lasso edit limit's sentence ([`FillLimit::LassoEdits`](super::FillLimit::LassoEdits)).
        lasso_edit_limit: count "fill.limitLassoEdits" = "at most {n} lasso edit: undo some, or reset the fill area" | "at most {n} lasso edits: undo some, or reset the fill area";
        /// The points limit's sentence ([`FillLimit::Vertices`](super::FillLimit::Points)).
        point_limit: count "fill.limitPoints" = "at most {n} point in all sampling strokes and lasso edits: undo some, or reset" | "at most {n} points in all sampling strokes and lasso edits: undo some, or reset";
    }
}
