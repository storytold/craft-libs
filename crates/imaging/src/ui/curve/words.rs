//! The words of the curve editor.

craft_ui::words! {
    /// The words of the curve editor and its graph.
    pub struct CurveWords {
        /// The selected point's input field.
        input: "curve.input" = "Input";
        /// The selected point's output field.
        output: "curve.output" = "Output";
        /// The check box that makes the selected point a corner point.
        corner: "curve.corner" = "Corner";
        /// A point's accessible name.
        point: template "curve.point" = "Point {i} of {n}";
        /// A point's accessible value.
        point_value: template "curve.pointValue" = "Input {x}, Output {y}";
        /// A corner point's accessible value.
        corner_value: template "curve.cornerValue" = "Input {x}, Output {y}, corner";
        /// The graph's accessible value: `{points}` is the count text.
        graph_value: template "curve.graphValue" = "{channel}, {points}";
        /// The graph's accessible value without channels.
        graph_points: count "curve.graphPoints" = "{n} point" | "{n} points";
        /// The accessible value of a drawn curve.
        graph_drawn: template "curve.graphDrawn" = "{channel}, drawn";
        /// The accessible value of a drawn curve without channels.
        drawn: "curve.drawn" = "Drawn";
        /// A value in percent of the levels.
        percent: template "curve.percent" = "{n}%";
        /// The unit shown after a field in percent.
        percent_unit: "curve.percentUnit" = "%";
    }
}
