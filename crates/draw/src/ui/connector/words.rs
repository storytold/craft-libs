//! The words of the Connector tool, its handles and its bar, English by default.

craft_ui::words! {
    /// What a screen reader reads for the connector tool, its handles and the edit labels.
    pub struct ConnectorWords {
        /// The handles' group name.
        group: "connectorTool.group" = "Connector";
        /// The start end glued to `{target}` (the app's object name).
        start_attached: template "connectorTool.startAttached" = "Start, attached to {target}";
        start_free: "connectorTool.startFree" = "Start, free";
        /// The finish end glued to `{target}`.
        end_attached: template "connectorTool.endAttached" = "End, attached to {target}";
        end_free: "connectorTool.endFree" = "End, free";
        /// A path handle: `{index}` from 1 of `{count}`.
        bend: template "connectorTool.bend" = "Bend {index} of {count}";
        /// The label of the gesture that draws a connector.
        draw: "connectorTool.draw" = "Draw connector";
    }
}

craft_ui::words! {
    /// The words of [`ConnectorBar`](super::ConnectorBar) (FigCraft's; US spelling).
    pub struct ConnectorBarWords {
        /// The routing buttons' group name.
        routing: "connectorBar.routing" = "Connector type";
        straight: "connectorBar.straight" = "Straight";
        elbow: "connectorBar.elbow" = "Elbow";
        curved: "connectorBar.curved" = "Curved";
        start: "connectorBar.start" = "Start point";
        end: "connectorBar.end" = "End point";
        line_style: "connectorBar.lineStyle" = "Stroke style";
        solid: "connectorBar.solid" = "Solid";
        dashed: "connectorBar.dashed" = "Dash";
        /// A weight row: `{weight}` in points.
        weight: template "connectorBar.weight" = "Stroke weight {weight}";
    }
}
