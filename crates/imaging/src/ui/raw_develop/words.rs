//! `RawDevelopWords`: the RAW Develop workspace's words, and the labels of its panel, mixer and scope components.
//!
//! Naming rule for the component labels: a slider's field is named after the `RawDevelopSettings` field it edits;
//! section titles are `section_*`, Color Mixer tabs `mixer_*` and bands `band_*`, grading wheels `wheel_*`.

craft_ui::words! {
    /// The RAW Develop workspace's words, English by default (Photoshop's Camera Raw names). OK and Cancel are the
    /// dialog's common words.
    #[non_exhaustive]
    pub struct RawDevelopWords {
        /// The title's name while a raw is being opened.
        name: "rawDevelop.name" = "RAW Develop";
        /// The title's name of the filter.
        name_filter: "rawDevelop.nameFilter" = "RAW Develop Filter";
        /// The footer's Open button.
        open: "rawDevelop.open" = "Open";
        /// The footer's Done button.
        done: "rawDevelop.done" = "Done";
        /// The footer's Save Settings button.
        save_settings: "rawDevelop.saveSettings" = "Save Settings…";
        /// The footer's Load Settings button.
        load_settings: "rawDevelop.loadSettings" = "Load Settings…";
        /// The Before view's name and its history step's name.
        before: "rawDevelop.before" = "Before";
        /// The accessible name of the side column.
        settings: "rawDevelop.settings" = "Settings";
        /// The history step of Auto and the Auto button.
        auto: "rawDevelop.auto" = "Auto";
        /// The history step of Load Settings.
        load_step: "rawDevelop.loadStep" = "Load Settings";
        /// The accessible name of the filmstrip.
        filmstrip: "rawDevelop.filmstrip" = "Raw files";
        /// The tip of the edited dot on a filmstrip tile.
        edited: "rawDevelop.edited" = "Edited";
        /// The accessible name of the clipping overlay.
        clipping: "rawDevelop.clipping" = "Clipping";
        /// The accessible name of the color sampler marks and of the Color Sampler tool's readouts.
        samplers: "rawDevelop.samplers" = "Color samplers";
        /// The accessible name of the scope.
        scope: "rawDevelop.scope" = "Scope";
        /// The note under the canvas; `{n}` the milliseconds the develop took.
        preview_ms: template "rawDevelop.previewMs" = "Preview {n} ms";
        /// The note under the canvas; `{n}` the milliseconds, `{backend}` what developed it.
        preview_ms_backend: template "rawDevelop.previewMsBackend" = "Preview {n} ms, {backend}";
        /// A color sampler's readout; `{n}` its number, `{r}`, `{g}`, `{b}` the 8-bit channels.
        sampler_readout: template "rawDevelop.samplerReadout" = "#{n}  R {r}  G {g}  B {b}";
        /// A color sampler's Lab readout; `{n}` its number, `{l}`, `{a}`, `{b}` the Lab values.
        sampler_readout_lab: template "rawDevelop.samplerReadoutLab" = "#{n}  L {l}  a {a}  b {b}";
        /// The Float action of the scope strip.
        float: "rawDevelop.float" = "Float";
        /// The Dock action of the floating scope.
        dock: "rawDevelop.dock" = "Dock";
        /// The Demosaic select's name.
        demosaic: "rawDevelop.demosaic" = "Demosaic";
        /// The AHD demosaic.
        demosaic_ahd: "rawDevelop.demosaicAhd" = "AHD";
        /// The MHC demosaic.
        demosaic_mhc: "rawDevelop.demosaicMhc" = "MHC";
        /// The bilinear demosaic.
        demosaic_bilinear: "rawDevelop.demosaicBilinear" = "Bilinear";
        /// The Light section's title.
        section_light: "rawDevelop.sectionLight" = "Light";
        /// The Color section's title.
        section_color: "rawDevelop.sectionColor" = "Color";
        /// The Effects section's title.
        section_effects: "rawDevelop.sectionEffects" = "Effects";
        /// The Curve section's title.
        section_curve: "rawDevelop.sectionCurve" = "Curve";
        /// The Color Mixer section's title.
        section_color_mixer: "rawDevelop.sectionColorMixer" = "Color Mixer";
        /// The Color Grading section's title.
        section_color_grading: "rawDevelop.sectionColorGrading" = "Color Grading";
        /// The Detail section's title.
        section_detail: "rawDevelop.sectionDetail" = "Detail";
        /// The Calibration section's title.
        section_calibration: "rawDevelop.sectionCalibration" = "Calibration";
        /// The temperature slider.
        temperature: "rawDevelop.temperature" = "Temperature";
        /// The tint slider.
        tint: "rawDevelop.tint" = "Tint";
        /// The exposure slider.
        exposure: "rawDevelop.exposure" = "Exposure";
        /// The contrast slider.
        contrast: "rawDevelop.contrast" = "Contrast";
        /// The highlights slider.
        highlights: "rawDevelop.highlights" = "Highlights";
        /// The shadows slider.
        shadows: "rawDevelop.shadows" = "Shadows";
        /// The whites slider.
        whites: "rawDevelop.whites" = "Whites";
        /// The blacks slider.
        blacks: "rawDevelop.blacks" = "Blacks";
        /// The texture slider.
        texture: "rawDevelop.texture" = "Texture";
        /// The clarity slider.
        clarity: "rawDevelop.clarity" = "Clarity";
        /// The dehaze slider.
        dehaze: "rawDevelop.dehaze" = "Dehaze";
        /// The vibrance slider.
        vibrance: "rawDevelop.vibrance" = "Vibrance";
        /// The saturation slider.
        saturation: "rawDevelop.saturation" = "Saturation";
        /// The curve highlights slider.
        curve_highlights: "rawDevelop.curveHighlights" = "Highlights";
        /// The curve lights slider.
        curve_lights: "rawDevelop.curveLights" = "Lights";
        /// The curve darks slider.
        curve_darks: "rawDevelop.curveDarks" = "Darks";
        /// The curve shadows slider.
        curve_shadows: "rawDevelop.curveShadows" = "Shadows";
        /// The curve's split between the shadows and darks regions.
        curve_shadow_split: "rawDevelop.curveShadowSplit" = "Shadow Split";
        /// The curve's split between the darks and lights regions.
        curve_midtone_split: "rawDevelop.curveMidtoneSplit" = "Midtone Split";
        /// The curve's split between the lights and highlights regions.
        curve_highlight_split: "rawDevelop.curveHighlightSplit" = "Highlight Split";
        /// The sharpen amount slider.
        sharpen_amount: "rawDevelop.sharpenAmount" = "Sharpening";
        /// The sharpen radius slider.
        sharpen_radius: "rawDevelop.sharpenRadius" = "Radius";
        /// The sharpen detail slider.
        sharpen_detail: "rawDevelop.sharpenDetail" = "Detail";
        /// The sharpen masking slider.
        sharpen_masking: "rawDevelop.sharpenMasking" = "Masking";
        /// The noise luminance slider.
        noise_luminance: "rawDevelop.noiseLuminance" = "Noise Reduction";
        /// The noise luminance detail slider.
        noise_luminance_detail: "rawDevelop.noiseLuminanceDetail" = "Luminance Detail";
        /// The noise color slider.
        noise_color: "rawDevelop.noiseColor" = "Color Noise Reduction";
        /// The noise color detail slider.
        noise_color_detail: "rawDevelop.noiseColorDetail" = "Color Detail";
        /// The grain amount slider.
        grain_amount: "rawDevelop.grainAmount" = "Grain";
        /// The grain size slider.
        grain_size: "rawDevelop.grainSize" = "Grain Size";
        /// The grain roughness slider.
        grain_roughness: "rawDevelop.grainRoughness" = "Grain Roughness";
        /// The vignette amount slider.
        vignette_amount: "rawDevelop.vignetteAmount" = "Vignette";
        /// The vignette midpoint slider.
        vignette_midpoint: "rawDevelop.vignetteMidpoint" = "Midpoint";
        /// The vignette roundness slider.
        vignette_roundness: "rawDevelop.vignetteRoundness" = "Roundness";
        /// The vignette feather slider.
        vignette_feather: "rawDevelop.vignetteFeather" = "Feather";
        /// The vignette highlights slider.
        vignette_highlights: "rawDevelop.vignetteHighlights" = "Vignette Highlights";
        /// The vignette style select's name.
        vignette_style: "rawDevelop.vignetteStyle" = "Style";
        /// The highlight priority vignette style.
        highlight_priority: "rawDevelop.highlightPriority" = "Highlight Priority";
        /// The color priority vignette style.
        color_priority: "rawDevelop.colorPriority" = "Color Priority";
        /// The paint overlay vignette style.
        paint_overlay: "rawDevelop.paintOverlay" = "Paint Overlay";
        /// The Color Mixer's Hue tab.
        mixer_hue: "rawDevelop.mixerHue" = "Hue";
        /// The Color Mixer's Saturation tab.
        mixer_saturation: "rawDevelop.mixerSaturation" = "Saturation";
        /// The Color Mixer's Luminance tab.
        mixer_luminance: "rawDevelop.mixerLuminance" = "Luminance";
        /// The Reds band.
        band_reds: "rawDevelop.bandReds" = "Reds";
        /// The Oranges band.
        band_oranges: "rawDevelop.bandOranges" = "Oranges";
        /// The Yellows band.
        band_yellows: "rawDevelop.bandYellows" = "Yellows";
        /// The Greens band.
        band_greens: "rawDevelop.bandGreens" = "Greens";
        /// The Aquas band.
        band_aquas: "rawDevelop.bandAquas" = "Aquas";
        /// The Blues band.
        band_blues: "rawDevelop.bandBlues" = "Blues";
        /// The Purples band.
        band_purples: "rawDevelop.bandPurples" = "Purples";
        /// The Magentas band.
        band_magentas: "rawDevelop.bandMagentas" = "Magentas";
        /// The shadows grading wheel.
        wheel_shadows: "rawDevelop.wheelShadows" = "Shadows";
        /// The midtones grading wheel.
        wheel_midtones: "rawDevelop.wheelMidtones" = "Midtones";
        /// The highlights grading wheel.
        wheel_highlights: "rawDevelop.wheelHighlights" = "Highlights";
        /// The global grading wheel.
        wheel_global: "rawDevelop.wheelGlobal" = "Global";
        /// The grade blending slider.
        blending: "rawDevelop.blending" = "Blending";
        /// The grade balance slider.
        balance: "rawDevelop.balance" = "Balance";
        /// The calibration shadows tint slider.
        shadow_tint: "rawDevelop.shadowTint" = "Shadows Tint";
        /// The calibration hue sliders' name; `{primary}` is the primary.
        calibration_hue: template "rawDevelop.calibrationHue" = "{primary} Primary Hue";
        /// The calibration saturation sliders' name; `{primary}` is the primary.
        calibration_sat: template "rawDevelop.calibrationSat" = "{primary} Primary Saturation";
        /// The red primary.
        primary_red: "rawDevelop.primaryRed" = "Red";
        /// The green primary.
        primary_green: "rawDevelop.primaryGreen" = "Green";
        /// The blue primary.
        primary_blue: "rawDevelop.primaryBlue" = "Blue";
    }
}
