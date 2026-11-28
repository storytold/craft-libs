//! [`SelectMaskWords`]: Select and Mask's words. The tools' names are their commands' labels
//! ([`SelectMaskCommands`](super::SelectMaskCommands)); the view controls' words are `MaskViewWords`; OK and Cancel
//! are the dialog footer's shared words.

use craft_ui::runtime::env::Env;

use super::session::LimitRefused;

craft_ui::words! {
    /// Select and Mask's words, English by default (Photoshop's names).
    #[non_exhaustive]
    pub struct SelectMaskWords {
        /// The title's name.
        name: "selectMask.name" = "Select and Mask";
        /// The tool strip's accessible name.
        tools: "selectMask.tools" = "Tools";
        add: "selectMask.add" = "Add";
        subtract: "selectMask.subtract" = "Subtract";
        size: "selectMask.size" = "Size";
        hardness: "selectMask.hardness" = "Hardness";
        sample_all_layers: "selectMask.sampleAllLayers" = "Sample All Layers";
        select_subject: "selectMask.selectSubject" = "Select Subject";
        refine_hair: "selectMask.refineHair" = "Refine Hair";
        refine_hair_missing: "selectMask.refineHairMissing" = "Refine Hair needs hair detection, which isn't available yet";
        properties: "selectMask.properties" = "Properties";
        reset: "selectMask.reset" = "Reset";
        remember: "selectMask.remember" = "Remember Settings";
        view_mode: "selectMask.viewMode" = "View Mode";
        real_time: "selectMask.realTime" = "Real-Time Refinement";
        high_quality: "selectMask.highQuality" = "High Quality Preview";
        overlay_color: "selectMask.overlayColor" = "Color";
        indicates: "selectMask.indicates" = "Indicates";
        masked_areas: "selectMask.maskedAreas" = "Masked Areas";
        selected_areas: "selectMask.selectedAreas" = "Selected Areas";
        refine_mode: "selectMask.refineMode" = "Refine Mode";
        color_aware: "selectMask.colorAware" = "Color Aware";
        object_aware: "selectMask.objectAware" = "Object Aware";
        object_aware_missing: "selectMask.objectAwareMissing" = "Object Aware needs a segmentation model, which isn't available yet";
        edge_detection: "selectMask.edgeDetection" = "Edge Detection";
        radius: "selectMask.radius" = "Radius";
        smart_radius: "selectMask.smartRadius" = "Smart Radius";
        global_refinements: "selectMask.globalRefinements" = "Global Refinements";
        smooth: "selectMask.smooth" = "Smooth";
        feather: "selectMask.feather" = "Feather";
        contrast: "selectMask.contrast" = "Contrast";
        shift_edge: "selectMask.shiftEdge" = "Shift Edge";
        clear_selection: "selectMask.clearSelection" = "Clear Selection";
        invert: "selectMask.invert" = "Invert";
        output_settings: "selectMask.outputSettings" = "Output Settings";
        decontaminate: "selectMask.decontaminate" = "Decontaminate Colors";
        amount: "selectMask.amount" = "Amount";
        output_to: "selectMask.outputTo" = "Output To";
        output_selection: "selectMask.outputSelection" = "Selection";
        output_layer_mask: "selectMask.outputLayerMask" = "Layer Mask";
        output_new_layer: "selectMask.outputNewLayer" = "New Layer";
        output_new_layer_with_mask: "selectMask.outputNewLayerWithMask" = "New Layer with Layer Mask";
        output_new_document: "selectMask.outputNewDocument" = "New Document";
        output_new_document_over_mask: "selectMask.outputNewDocumentWithMask" = "New Document with Layer Mask";
        limit_edits: count "selectMask.limitEdits" = "at most {n} edit: undo some, or reset" | "at most {n} edits: undo some, or reset";
        limit_edit_points: count "selectMask.limitEditPoints" = "at most {n} point in all edits: undo some, or reset" | "at most {n} points in all edits: undo some, or reset";
        limit_strokes: count "selectMask.limitStrokes" = "at most {n} Refine Edge Brush stroke: undo some, or reset" | "at most {n} Refine Edge Brush strokes: undo some, or reset";
        limit_stroke_points: count "selectMask.limitStrokePoints" = "at most {n} point in all Refine Edge Brush strokes: undo some, or reset" | "at most {n} points in all Refine Edge Brush strokes: undo some, or reset";
    }
}

impl LimitRefused {
    /// The message under the panel, in the words and numbers of `env`.
    pub fn message(self, env: &Env) -> String {
        let words = env.words::<SelectMaskWords>();
        let format = env.format();
        let (count, n) = match self {
            LimitRefused::Edits(n) => (&words.limit_edits, n),
            LimitRefused::EditPoints(n) => (&words.limit_edit_points, n),
            LimitRefused::Strokes(n) => (&words.limit_strokes, n),
            LimitRefused::StrokePoints(n) => (&words.limit_stroke_points, n),
        };
        let n = n as u64;
        count.text(n, &format.integer(n))
    }
}
