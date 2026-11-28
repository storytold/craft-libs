//! The words of the shape panel and the Shape tools' options bar, English by default (PhotoCraft's, without colons;
//! US spelling).

craft_ui::words! {
    /// The words of [`ShapePanel`](super::ShapePanel) and [`ShapeBar`](super::ShapeBar).
    pub struct ShapeWords {
        /// The panel's accessible name.
        properties: "shape.properties" = "Shape properties";
        /// The Appearance section (the shape's look, not the app's theme).
        appearance: "shape.appearance" = "Appearance";
        /// The live shape section.
        shape: "shape.shape" = "Shape";
        fill: "shape.fill" = "Fill";
        stroke: "shape.stroke" = "Stroke";
        /// The fill chip's accessible name.
        fill_color: "shape.fillColor" = "Fill color";
        /// The stroke chip's accessible name.
        stroke_color: "shape.strokeColor" = "Stroke color";
        /// The stroke width field's accessible name.
        stroke_width: "shape.strokeWidth" = "Stroke width";
        /// The Stroke style button's accessible name.
        stroke_style: "shape.strokeStyle" = "Stroke style";
        /// The fill chip's tooltip.
        set_fill_type: "shape.setFillType" = "Set shape fill type";
        /// The stroke chip's tooltip.
        set_stroke_type: "shape.setStrokeType" = "Set shape stroke type";
        width: "shape.width" = "W";
        width_name: "shape.widthName" = "Width";
        height: "shape.height" = "H";
        height_name: "shape.heightName" = "Height";
        corner_radius: "shape.cornerRadius" = "Corner radius";
        top_left: "shape.topLeft" = "Top Left";
        top_right: "shape.topRight" = "Top Right";
        bottom_right: "shape.bottomRight" = "Bottom Right";
        bottom_left: "shape.bottomLeft" = "Bottom Left";
        sides: "shape.sides" = "Sides";
        star_ratio: "shape.starRatio" = "Star ratio";
        weight: "shape.weight" = "Weight";
        /// The options bar's corner radius cell.
        radius: "shape.radius" = "Radius";
        /// The vector mode select's accessible name.
        mode: "shape.mode" = "Mode";
        /// The mode entry that draws a shape layer.
        shape_mode: "shape.shapeMode" = "Shape";
        /// The mode entry that draws a path.
        path_mode: "shape.pathMode" = "Path";
        /// The mode entry that draws filled pixels.
        pixels_mode: "shape.pixelsMode" = "Pixels";
        /// The arrowhead popover's name and its button's accessible name.
        arrowheads: "shape.arrowheads" = "Arrowheads";
        /// The check box for an arrowhead at the start.
        arrow_start: "shape.arrowStart" = "Start";
        /// The check box for an arrowhead at the end.
        arrow_end: "shape.arrowEnd" = "End";
        /// The arrowhead width field.
        arrow_width: "shape.arrowWidth" = "Width";
        /// The arrowhead length field.
        arrow_length: "shape.arrowLength" = "Length";
        /// The arrowhead concavity field.
        arrow_concavity: "shape.arrowConcavity" = "Concavity";
        /// The check box that makes the arrowhead values the defaults of new arrows.
        set_as_default: "shape.setAsDefault" = "Set as default";
        /// The tooltip of that check box.
        set_as_default_tip: "shape.setAsDefaultTip" = "Use these settings for new arrows";
        /// The Rectangle gear's button: its accessible name and tooltip.
        shape_options: "shape.shapeOptions" = "Set additional shape and path options";
        /// The gear's radio that draws the dragged box.
        unconstrained: "shape.unconstrained" = "Unconstrained";
        /// The gear's radio that draws a square.
        square: "shape.square" = "Square";
        /// The gear's radio that draws a box of a fixed width and height.
        fixed_size: "shape.fixedSize" = "Fixed Size";
        /// The gear's radio that draws a box of a fixed width to height ratio.
        proportional: "shape.proportional" = "Proportional";
        /// The gear's check box that draws from the press as the centre.
        from_center: "shape.fromCenter" = "From Center";
        /// The Fixed Size width field's accessible name.
        fixed_width: "shape.fixedWidth" = "Fixed Size W";
        /// The Fixed Size height field's accessible name.
        fixed_height: "shape.fixedHeight" = "Fixed Size H";
        /// The Proportional width field's accessible name.
        ratio_width: "shape.ratioWidth" = "Proportional W";
        /// The Proportional height field's accessible name.
        ratio_height: "shape.ratioHeight" = "Proportional H";
    }
}
