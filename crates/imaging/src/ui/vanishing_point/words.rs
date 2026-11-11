//! [`VanishingPointWords`]: Vanishing Point's words.

craft_ui::words! {
    /// Vanishing Point's words, English by default (Photoshop's names). The tools' names are the commands' labels.
    pub struct VanishingPointWords {
        name: "vanishingPoint.name" = "Vanishing Point";
        tools: "vanishingPoint.tools" = "Tools";
        options: "vanishingPoint.options" = "Vanishing Point options";
        diameter: "vanishingPoint.diameter" = "Diameter";
        hardness: "vanishingPoint.hardness" = "Hardness";
        opacity: "vanishingPoint.opacity" = "Opacity";
        aligned: "vanishingPoint.aligned" = "Aligned";
        grid_size: "vanishingPoint.gridSize" = "Grid Size";
        paste: "vanishingPoint.paste" = "Paste";
        width: "vanishingPoint.width" = "Width";
        new_layer: "vanishingPoint.newLayer" = "New Layer";
        corner_part: template "vanishingPoint.cornerPart" = "Plane {n}, {corner}";
        top_left: "vanishingPoint.topLeft" = "top-left corner";
        top_right: "vanishingPoint.topRight" = "top-right corner";
        bottom_right: "vanishingPoint.bottomRight" = "bottom-right corner";
        bottom_left: "vanishingPoint.bottomLeft" = "bottom-left corner";
        questionable: "vanishingPoint.questionable" = "This plane's perspective is questionable";
        invalid: "vanishingPoint.invalid" = "This plane's perspective is invalid";
        parts: "vanishingPoint.parts" = "Plane corners";
        planes: "vanishingPoint.planes" = "Planes";
        stamp_no_source: "vanishingPoint.stampNoSource" = "Alt-click a plane to set the clone source";
        stamp_too_many: "vanishingPoint.stampTooMany" = "Too many clone strokes: apply Vanishing Point first";
        edit_planes_step: "vanishingPoint.editPlanesStep" = "Edit Planes";
        paste_step: "vanishingPoint.pasteStep" = "Move Paste";
        paste_width_step: "vanishingPoint.pasteWidthStep" = "Paste Width";
        stamp_step: "vanishingPoint.stampStep" = "Clone Stamp";
        source_step: "vanishingPoint.sourceStep" = "Set Clone Source";
        new_plane: "vanishingPoint.newPlane" = "New Plane";
        new_plane_corner: template "vanishingPoint.newPlaneCorner" = "Corner {n}";
        new_plane_x: "vanishingPoint.newPlaneX" = "X";
        new_plane_y: "vanishingPoint.newPlaneY" = "Y";
    }
}
