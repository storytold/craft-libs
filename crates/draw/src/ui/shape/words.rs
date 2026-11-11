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
    }
}
