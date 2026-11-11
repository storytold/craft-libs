//! Built-in words of the canvas controls family.

craft_ui::words! {
    /// The words of the canvas controls: the guides' accessible names and position labels (`guides`), the
    /// marquee's accessible description (`marquee`) and the row labels of the brush resize readout
    /// (`brushResize`).
    pub struct CanvasControlWords {
        vertical: "guides.vertical" = "Vertical guide";
        horizontal: "guides.horizontal" = "Horizontal guide";
        x: "guides.x" = "X";
        y: "guides.y" = "Y";

        shape_rect: "marquee.shapeRect" = "Rectangle";
        shape_ellipse: "marquee.shapeEllipse" = "Ellipse";
        shape_polygon: "marquee.shapePolygon" = "Polygon";
        shape_polyline: "marquee.shapePolyline" = "Open path";
        shape_segments: "marquee.shapeSegments" = "Outline edges";
        bounds: template "marquee.bounds" = "{width} × {height} at {x}, {y}";

        diameter: "brushResize.diameter" = "Diameter:";
        hardness: "brushResize.hardness" = "Hardness:";
        opacity: "brushResize.opacity" = "Opacity:";
        density: "brushResize.density" = "Density:";
    }
}
