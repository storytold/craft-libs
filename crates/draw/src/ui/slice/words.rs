//! The words of the Slice tools, their bar and dialogs, English by default.

craft_ui::words! {
    /// What the slice control, the tool and a screen reader read.
    pub struct SliceToolWords {
        group: "sliceTool.group" = "Slices";
        /// A slice part: `{number}` two digits, `{origin}` one of the origin words, `{width}`, `{height}`, `{x}`, `{y}`
        /// in pixels.
        slice: template "sliceTool.slice" = "Slice {number}, {origin}, {width} by {height} at {x}, {y}";
        user: "sliceTool.user" = "user slice";
        auto: "sliceTool.auto" = "auto slice";
        layer: "sliceTool.layer" = "layer slice";
        /// The status text of a refused press (PhotoCraft's).
        locked: "sliceTool.locked" = "Slices are locked (View › Lock Slices)";
        /// The undo step names.
        new_slice: "sliceTool.newSlice" = "New Slice";
        move_slice: "sliceTool.moveSlice" = "Move Slice";
        resize_slice: "sliceTool.resizeSlice" = "Resize Slice";
        /// The tool's name for assistive technology.
        tool: "sliceTool.tool" = "Slice";
        handle_top_left: "sliceTool.handleTopLeft" = "Top left handle";
        handle_top: "sliceTool.handleTop" = "Top handle";
        handle_top_right: "sliceTool.handleTopRight" = "Top right handle";
        handle_right: "sliceTool.handleRight" = "Right handle";
        handle_bottom_right: "sliceTool.handleBottomRight" = "Bottom right handle";
        handle_bottom: "sliceTool.handleBottom" = "Bottom handle";
        handle_bottom_left: "sliceTool.handleBottomLeft" = "Bottom left handle";
        handle_left: "sliceTool.handleLeft" = "Left handle";
    }
}

craft_ui::words! {
    /// The words of the bar and the dialogs (PhotoCraft's and Photoshop's, no colons; US spelling).
    pub struct SliceWords {
        hide_auto: "slice.hideAuto" = "Hide Auto Slices";
        /// The name of the bar's command strips.
        commands: "slice.commands" = "Slice commands";
        options_title: "slice.optionsTitle" = "Slice Options";
        kind: "slice.kind" = "Slice Type";
        image: "slice.image" = "Image";
        no_image: "slice.noImage" = "No Image";
        html_text: "slice.htmlText" = "HTML Text";
        name: "slice.name" = "Name";
        url: "slice.url" = "URL";
        target: "slice.target" = "Target";
        message: "slice.message" = "Message Text";
        alt: "slice.alt" = "Alt Tag";
        cell_text: "slice.cellText" = "Text";
        background: "slice.background" = "Background";
        background_none: "slice.backgroundNone" = "None";
        background_color: "slice.backgroundColor" = "Slice Background Color";
        background_kind: "slice.backgroundKind" = "Background Type";
        background_color_choice: "slice.backgroundColorChoice" = "Color";
        divide_title: "slice.divideTitle" = "Divide Slice";
        horizontally: "slice.horizontally" = "Divide Horizontally Into";
        vertically: "slice.vertically" = "Divide Vertically Into";
        divide_by: "slice.divideBy" = "Divide By";
        count: "slice.count" = "slices, evenly spaced";
        size: "slice.size" = "pixels per slice";
        divide_value: "slice.divideValue" = "Value";
    }
}
