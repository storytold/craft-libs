//! The words of the Artboard tool, its bar and the Properties section, English by default.

craft_ui::words! {
    /// What a screen reader and the undo steps read for the Artboard tool's parts.
    pub struct ArtboardToolWords {
        group: "artboardTool.group" = "Artboards";
        /// A board part: `{index}` from 1, `{width}`, `{height}`, `{x}`, `{y}` in pixels.
        board: template "artboardTool.board" = "Artboard {index}, {width} by {height} at {x}, {y}";
        /// A handle part's value: `{x}`, `{y}` in pixels.
        handle_at: template "artboardTool.handleAt" = "{x}, {y}";
        handle_top_left: "artboardTool.handleTopLeft" = "Top left handle";
        handle_top: "artboardTool.handleTop" = "Top handle";
        handle_top_right: "artboardTool.handleTopRight" = "Top right handle";
        handle_right: "artboardTool.handleRight" = "Right handle";
        handle_bottom_right: "artboardTool.handleBottomRight" = "Bottom right handle";
        handle_bottom: "artboardTool.handleBottom" = "Bottom handle";
        handle_bottom_left: "artboardTool.handleBottomLeft" = "Bottom left handle";
        handle_left: "artboardTool.handleLeft" = "Left handle";
        new_board: "artboardTool.newBoard" = "New Artboard";
        move_board: "artboardTool.moveBoard" = "Move Artboard";
        resize_board: "artboardTool.resizeBoard" = "Resize Artboard";
        tool: "artboardTool.tool" = "Artboard";
    }
}

craft_ui::words! {
    /// The words of the bar and the Properties section (PhotoCraft's, no colons).
    pub struct ArtboardWords {
        artboard: "artboard.artboard" = "Artboard";
        size: "artboard.size" = "Size";
        custom: "artboard.custom" = "Custom";
        width: "artboard.width" = "W";
        width_name: "artboard.widthName" = "Width";
        height: "artboard.height" = "H";
        height_name: "artboard.heightName" = "Height";
        x: "artboard.x" = "X";
        y: "artboard.y" = "Y";
        make_portrait: "artboard.makePortrait" = "Make Portrait";
        make_landscape: "artboard.makeLandscape" = "Make Landscape";
        background: "artboard.background" = "Background";
        white: "artboard.white" = "White";
        black: "artboard.black" = "Black";
        transparent: "artboard.transparent" = "Transparent";
        other: "artboard.other" = "Other…";
        /// The colour picker's title for Other….
        background_color: "artboard.backgroundColor" = "Artboard Background Color";
        /// The name of the bar's command strip.
        commands: "artboard.commands" = "Artboard commands";
    }
}
