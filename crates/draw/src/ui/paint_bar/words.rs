//! The words of the paint bar ([`PaintBarWords`]).

craft_ui::words! {
    /// The words of [`PaintBar`](super::PaintBar), English by default (Photoshop's).
    pub struct PaintBarWords {
        /// The Opacity field's caption.
        opacity: "paintBar.opacity" = "Opacity";
        /// The Flow field's caption.
        flow: "paintBar.flow" = "Flow";
        /// The Mode field's caption.
        mode: "paintBar.mode" = "Mode";
        /// The Smoothing field's caption.
        smoothing: "paintBar.smoothing" = "Smoothing";
        /// The smoothing gear and its popover.
        smoothing_options: "paintBar.smoothingOptions" = "Smoothing options";
        /// Pulled String Mode.
        pulled_string: "paintBar.pulledString" = "Pulled String Mode";
        /// Stroke Catch-up.
        catch_up: "paintBar.catchUp" = "Stroke Catch-up";
        /// Catch-up on Stroke End.
        catch_up_on_end: "paintBar.catchUpOnEnd" = "Catch-up on Stroke End";
        /// Adjust for Zoom.
        adjust_for_zoom: "paintBar.adjustForZoom" = "Adjust for Zoom";
        /// The toggle that lets pen pressure control the opacity.
        pressure_opacity: "paintBar.pressureOpacity" = "Pressure controls opacity";
        /// The toggle that lets pen pressure control the size.
        pressure_size: "paintBar.pressureSize" = "Pressure controls size";
        /// The build-up (airbrush) toggle.
        build_up: "paintBar.buildUp" = "Build-up";
        /// The Eraser's Erase to History toggle.
        erase_to_history: "paintBar.eraseToHistory" = "Erase to History";
        /// The Pencil's Auto Erase toggle.
        auto_erase: "paintBar.autoErase" = "Auto Erase";
        /// The tooltip of Auto Erase.
        auto_erase_tip: "paintBar.autoEraseTip" = "Draws background color over foreground colors";
        /// The Eraser's Brush mode.
        eraser_brush: "paintBar.eraserBrush" = "Brush";
        /// The Eraser's Pencil mode.
        eraser_pencil: "paintBar.eraserPencil" = "Pencil";
        /// The Eraser's Block mode.
        eraser_block: "paintBar.eraserBlock" = "Block";
        /// The Mixer Brush's load swatch.
        load_swatch: "paintBar.loadSwatch" = "Current brush load";
        /// The load swatch menu's Load Brush.
        load_brush: "paintBar.loadBrush" = "Load Brush";
        /// The load swatch menu's Clean Brush.
        clean_brush: "paintBar.cleanBrush" = "Clean Brush";
        /// The load swatch menu's Load Solid Colors Only.
        solid_colors_only: "paintBar.solidColorsOnly" = "Load Solid Colors Only";
        /// The toggle that loads the brush after each stroke.
        load_after_stroke: "paintBar.loadAfterStroke" = "Load the brush after each stroke";
        /// The toggle that cleans the brush after each stroke.
        clean_after_stroke: "paintBar.cleanAfterStroke" = "Clean the brush after each stroke";
        /// The combinations list's name.
        combination: "paintBar.combination" = "Useful mixer brush combinations";
        /// The Wet field's caption.
        wet: "paintBar.wet" = "Wet";
        /// The Load field's caption.
        load: "paintBar.load" = "Load";
        /// The Mix field's caption.
        mix: "paintBar.mix" = "Mix";
        /// The Sample All Layers toggle.
        sample_all_layers: "paintBar.sampleAllLayers" = "Sample All Layers";
        /// The names of the mixer combinations, in [`MIXER_COMBINATIONS`](super::MIXER_COMBINATIONS) order.
        combinations: array [
        "paintBar.combination0" = "Custom",
        "paintBar.combination1" = "Dry",
        "paintBar.combination2" = "Dry, Light Load",
        "paintBar.combination3" = "Dry, Heavy Load",
        "paintBar.combination4" = "Moist",
        "paintBar.combination5" = "Moist, Light Mix",
        "paintBar.combination6" = "Moist, Heavy Mix",
        "paintBar.combination7" = "Wet",
        "paintBar.combination8" = "Wet, Light Mix",
        "paintBar.combination9" = "Wet, Heavy Mix",
        "paintBar.combination10" = "Very Wet",
        "paintBar.combination11" = "Very Wet, Light Mix",
        "paintBar.combination12" = "Very Wet, Heavy Mix"
        ];
    }
}
