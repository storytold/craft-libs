//! The words of the Shape tools and the Create Shape dialog, English by default.

craft_ui::words! {
    /// What the status readout and a screen reader read for a Shape tool while it draws.
    pub struct ShapeDragWords {
        /// The readout's first row label: the shape's width.
        width: "shapeDragTool.width" = "Width";
        /// The readout's second row label: the shape's height.
        height: "shapeDragTool.height" = "Height";
        /// The size of the shape being drawn, as the access node's value.
        size: template "shapeDragTool.size" = "{width} × {height}";
        /// The label of the gesture that draws a shape.
        draw: "shapeDragTool.draw" = "Draw shape";
        rectangle: "shapeDragTool.rectangle" = "Rectangle";
        rounded_rectangle: "shapeDragTool.roundedRectangle" = "Rounded Rectangle";
        ellipse: "shapeDragTool.ellipse" = "Ellipse";
        triangle: "shapeDragTool.triangle" = "Triangle";
        polygon: "shapeDragTool.polygon" = "Polygon";
        star: "shapeDragTool.star" = "Star";
        line: "shapeDragTool.line" = "Line";
        custom: "shapeDragTool.custom" = "Custom Shape";
    }
}

craft_ui::words! {
    /// The Create Shape dialog's words (PhotoCraft's, without colons; US spelling).
    pub struct CreateShapeWords {
        rectangle: "createShape.rectangle" = "Create Rectangle";
        rounded_rectangle: "createShape.roundedRectangle" = "Create Rounded Rectangle";
        ellipse: "createShape.ellipse" = "Create Ellipse";
        triangle: "createShape.triangle" = "Create Triangle";
        polygon: "createShape.polygon" = "Create Polygon";
        star: "createShape.star" = "Create Star";
        width: "createShape.width" = "Width";
        height: "createShape.height" = "Height";
        radius: "createShape.radius" = "Radius";
        top_left: "createShape.topLeft" = "Top Left";
        top_right: "createShape.topRight" = "Top Right";
        bottom_right: "createShape.bottomRight" = "Bottom Right";
        bottom_left: "createShape.bottomLeft" = "Bottom Left";
        link_radii: "createShape.linkRadii" = "Link radii";
        sides: "createShape.sides" = "Sides";
        star_ratio: "createShape.starRatio" = "Star Ratio";
        from_center: "createShape.fromCenter" = "From Center";
        /// The click lies more than 300,000 px from the origin on an axis.
        bad_position: "createShape.badPosition" = "The shape's position is out of range";
        /// A width or height outside 1–300,000 px; `{field}` is the field's label, `{max}` the limit.
        bad_size: template "createShape.badSize" = "{field} must be between 1 and {max} pixels";
    }
}
