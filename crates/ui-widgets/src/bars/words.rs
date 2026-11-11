//! Built-in words of the bars family.

craft_ui::words! {
    /// The words of the bars family; each id is prefixed by its type.
    pub struct BarWords {
        bar_tool: "bars.barTool" = "Toolbar";
        bar_options: "bars.barOptions" = "Options";
        bar_status: "bars.barStatus" = "Status";
        items_actions: "bars.itemsActions" = "Actions";
        items_more: "bars.itemsMore" = "More";
        align_bar: "bars.alignBar" = "Align and distribute";
        breadcrumb_hidden: "bars.breadcrumbHidden" = "Show the whole path";
        constraint_style: "bars.constraintStyle" = "Style";
        constraint_normal: "bars.constraintNormal" = "Normal";
        constraint_fixed_ratio: "bars.constraintFixedRatio" = "Fixed Ratio";
        constraint_fixed_size: "bars.constraintFixedSize" = "Fixed Size";
        constraint_width: "bars.constraintWidth" = "Width";
        constraint_height: "bars.constraintHeight" = "Height";
        constraint_swap: "bars.constraintSwap" = "Swaps height and width";
        readout_zoom: "bars.statusReadoutZoom" = "Zoom";
        readout_menu: "bars.statusReadoutMenu" = "Show";
        readout_document_sizes: "bars.statusReadoutDocumentSizes" = "Document Sizes";
        readout_document_profile: "bars.statusReadoutDocumentProfile" = "Document Profile";
        readout_document_dimensions: "bars.statusReadoutDocumentDimensions" = "Document Dimensions";
        readout_measurement_scale: "bars.statusReadoutMeasurementScale" = "Measurement Scale";
        readout_scratch_sizes: "bars.statusReadoutScratchSizes" = "Scratch Sizes";
        readout_efficiency: "bars.statusReadoutEfficiency" = "Efficiency";
        readout_timing: "bars.statusReadoutTiming" = "Timing";
        readout_current_tool: "bars.statusReadoutCurrentTool" = "Current Tool";
        readout_layer_count: "bars.statusReadoutLayerCount" = "Layer Count";
        readout_layers: count "bars.statusReadoutLayers" = "{n} Layer" | "{n} Layers";
        readout_doc_sizes: template "bars.statusReadoutDocSizes" = "Doc: {flat}/{layered}";
        readout_scratch: template "bars.statusReadoutScratch" = "Scratch: {used}/{available}";
        readout_efficiency_text: template "bars.statusReadoutEfficiencyText" = "Efficiency: {value}";
        readout_dimensions_text: template "bars.statusReadoutDimensionsText" = "{width} x {height}";
        readout_timing_text: template "bars.statusReadoutTimingText" = "Timing: {value}";
    }
}
