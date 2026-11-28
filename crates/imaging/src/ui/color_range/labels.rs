//! [`ColorRangeWords`]: the Color Range dialog's words.

craft_ui::words! {
    /// The words of the Color Range dialog, in the installed language unless set.
    pub struct ColorRangeWords {
        /// The dialog's title.
        name: "colorRange.name" = "Color Range";
        select: "colorRange.select" = "Select";
        localized: "colorRange.localized" = "Localized Color Clusters";
        fuzziness: "colorRange.fuzziness" = "Fuzziness";
        range: "colorRange.range" = "Range";
        shadows_up_to: "colorRange.shadowsUpTo" = "Shadows up to";
        highlights_from: "colorRange.highlightsFrom" = "Highlights from";
        midtones_from: "colorRange.midtonesFrom" = "Midtones from";
        midtones_to: "colorRange.midtonesTo" = "Midtones to";
        preview: "colorRange.preview" = "Color Range preview";
        sample_tools: "colorRange.sampleTools" = "Sample tools";
        eyedropper: "colorRange.eyedropper" = "Eyedropper";
        add_to_sample: "colorRange.addToSample" = "Add to Sample";
        subtract_from_sample: "colorRange.subtractFromSample" = "Subtract from Sample";
        sample_foreground: "colorRange.sampleForeground" = "Sample: foreground color";
        one_sample: template "colorRange.oneSample" = "{n} sample";
        many_samples: template "colorRange.manySamples" = "{n} samples";
        with_subtracted: template "colorRange.withSubtracted" = "{added}, {removed} subtracted";
        clear_samples: "colorRange.clearSamples" = "Clear Samples";
        invert: "colorRange.invert" = "Invert";
        selection: "colorRange.selection" = "Selection";
        image: "colorRange.image" = "Image";
        selection_preview: "colorRange.selectionPreview" = "Selection Preview";
        none: "colorRange.none" = "None";
        sampled: "colorRange.sampled" = "Sampled Colors";
        reds: "colorRange.reds" = "Reds";
        yellows: "colorRange.yellows" = "Yellows";
        greens: "colorRange.greens" = "Greens";
        cyans: "colorRange.cyans" = "Cyans";
        blues: "colorRange.blues" = "Blues";
        magentas: "colorRange.magentas" = "Magentas";
        highlights: "colorRange.highlights" = "Highlights";
        midtones: "colorRange.midtones" = "Midtones";
        shadows: "colorRange.shadows" = "Shadows";
        skin_tones: "colorRange.skinTones" = "Skin Tones";
        out_of_gamut: "colorRange.outOfGamut" = "Out Of Gamut";
    }
}
