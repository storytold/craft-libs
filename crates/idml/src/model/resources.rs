//! Resources: style groups and styles, swatches, inks, stroke styles, colour groups, and the
//! preference elements.

use craft_xml::{El, Local, Node};

use super::{Values, tidy};
use crate::value::nums;

/// The children of `el` other than `<Properties>`.
fn others(el: &El) -> impl Iterator<Item = &El> {
    el.elements().filter(|e| !e.is(Local("Properties")))
}

/// Raw bullet and numbering properties on an IDML paragraph style or range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphListValues {
    pub applied_list: Option<String>,
    pub list_type: Option<String>,
    pub numbering_format: Option<String>,
    pub expression: Option<String>,
    pub separator: Option<String>,
    pub bullet_type: Option<String>,
    pub bullet_value: Option<String>,
    pub balance_ragged: Option<String>,
    pub level: Option<f64>,
    pub bullet_character_style: Option<String>,
    pub numbering_character_style: Option<String>,
    pub bullet_alignment: Option<String>,
    pub numbering_alignment: Option<String>,
    pub restart_numbers: Option<bool>,
    pub restart_after_level: Option<u32>,
    pub restart_after_specific_level: bool,
    pub numbering_continue: Option<bool>,
    pub numbering_start_at: Option<f64>,
    pub bullet_font: Option<String>,
    pub bullet_font_style: Option<String>,
}

/// Paragraph border and shading values after the caller resolves model colours and enums.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphBoxValues {
    pub shading_on: Option<bool>,
    pub shading_color: Option<String>,
    pub shading_tint: Option<f64>,
    pub shading_offsets: Option<[f64; 4]>,
    pub shading_overprint: Option<bool>,
    pub border_on: Option<bool>,
    pub border_color: Option<String>,
    pub border_tint: Option<f64>,
    pub border_weights: Option<[f64; 4]>,
    pub border_offsets: Option<[f64; 4]>,
    pub border_type: Option<String>,
    pub border_cap: Option<String>,
    pub border_join: Option<String>,
    pub border_corners: Option<[(String, f64); 4]>,
    pub border_width: Option<String>,
    pub border_top: Option<String>,
    pub border_bottom: Option<String>,
    pub border_display_if_splits: Option<bool>,
    pub border_merge: Option<bool>,
    pub border_gap_color: Option<String>,
    pub border_gap_tint: Option<f64>,
    pub border_overprint: Option<bool>,
    pub border_gap_overprint: Option<bool>,
    pub shading_width: Option<String>,
    pub shading_top: Option<String>,
    pub shading_bottom: Option<String>,
    pub shading_clip: Option<bool>,
    pub shading_nonprinting: Option<bool>,
    pub shading_corners: Option<[(String, f64); 4]>,
}

/// CJK paragraph composition settings read from an IDML style or text range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CjkParagraphValues {
    pub kinsoku_hang: Option<String>,
    pub mojikumi: Option<String>,
    pub kinsoku_type: Option<String>,
    pub bunri_kinshi: Option<bool>,
    pub rensuuji: Option<bool>,
    pub treat_ideographic_space_as_space: Option<bool>,
    pub kinsoku_set: Option<String>,
    pub auto_tcy: Option<f64>,
    pub auto_tcy_include_roman: Option<bool>,
    pub rotate_single_byte_characters: Option<bool>,
    pub allow_arbitrary_hyphenation: Option<bool>,
    pub grid_gyoudori: Option<f64>,
    pub paragraph_gyoudori: Option<bool>,
}

impl CjkParagraphValues {
    /// Reads CJK paragraph composition properties from IDML.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let bounded_count = |key: &str| values.num(key).map(|value| value.clamp(0.0, 1000.0));
        Self {
            kinsoku_hang: values.prop("KinsokuHangType"),
            mojikumi: values.prop("Mojikumi"),
            kinsoku_type: values.prop("KinsokuType"),
            bunri_kinshi: values.boolean("BunriKinshi"),
            rensuuji: values.boolean("Rensuuji"),
            treat_ideographic_space_as_space: values.boolean("TreatIdeographicSpaceAsSpace"),
            kinsoku_set: values.prop("KinsokuSet"),
            auto_tcy: bounded_count("AutoTcy"),
            auto_tcy_include_roman: values.boolean("AutoTcyIncludeRoman"),
            rotate_single_byte_characters: values.boolean("RotateSingleByteCharacters"),
            allow_arbitrary_hyphenation: values.boolean("AllowArbitraryHyphenation"),
            grid_gyoudori: bounded_count("GridGyoudori"),
            paragraph_gyoudori: values.boolean("ParagraphGyoudori"),
        }
    }

    /// Encodes a semantic kinsoku-set name as its IDML property value.
    pub fn kinsoku_reference(name: &str) -> String {
        if name.is_empty() { "Nothing".into() } else { format!("KinsokuTable/{}", crate::value::escape_id(name)) }
    }

    /// Writes CJK paragraph attributes and properties.
    pub fn to_parts(&self) -> (Vec<(String, String)>, Vec<El>) {
        let mut attrs = Vec::new();
        let mut properties = Vec::new();
        for (key, value) in [("KinsokuHangType", &self.kinsoku_hang), ("KinsokuType", &self.kinsoku_type)] {
            if let Some(value) = value {
                attrs.push((key.into(), value.clone()));
            }
        }
        if let Some(value) = &self.mojikumi {
            let kind = if value.starts_with("MojikumiTable/") { "object" } else { "enumeration" };
            properties.push(property("Mojikumi", kind, value.clone()));
        }
        for (key, value) in [
            ("BunriKinshi", self.bunri_kinshi),
            ("Rensuuji", self.rensuuji),
            ("TreatIdeographicSpaceAsSpace", self.treat_ideographic_space_as_space),
            ("AutoTcyIncludeRoman", self.auto_tcy_include_roman),
            ("RotateSingleByteCharacters", self.rotate_single_byte_characters),
            ("AllowArbitraryHyphenation", self.allow_arbitrary_hyphenation),
            ("ParagraphGyoudori", self.paragraph_gyoudori),
        ] {
            push_boolean(&mut attrs, key, value);
        }
        push_number(&mut attrs, "AutoTcy", self.auto_tcy);
        push_number(&mut attrs, "GridGyoudori", self.grid_gyoudori);
        if let Some(value) = &self.kinsoku_set {
            properties.push(property("KinsokuSet", if value == "Nothing" { "enumeration" } else { "object" }, value.clone()));
        }
        (attrs, properties)
    }
}

/// Paragraph span and split-column properties read from IDML.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphSpanValues {
    pub kind: Option<String>,
    pub split_column_count: Option<u32>,
    pub inside_gutter: Option<f64>,
    pub outside_gutter: Option<f64>,
    pub space_before: Option<f64>,
    pub space_after: Option<f64>,
}

impl ParagraphSpanValues {
    /// Reads paragraph span and split-column properties from IDML.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            kind: values.prop("SpanColumnType"),
            split_column_count: values.prop("SpanSplitColumnCount").and_then(|value| value.trim().parse().ok()),
            inside_gutter: values.num("SplitColumnInsideGutter"),
            outside_gutter: values.num("SplitColumnOutsideGutter"),
            space_before: values.num("SpanColumnMinSpaceBefore"),
            space_after: values.num("SpanColumnMinSpaceAfter"),
        }
    }

    /// Writes paragraph span and split-column attributes and properties.
    pub fn to_parts(&self) -> (Vec<(String, String)>, Vec<El>) {
        let mut attrs = Vec::new();
        let mut properties = Vec::new();
        if let Some(kind) = &self.kind {
            attrs.push(("SpanColumnType".into(), kind.clone()));
        }
        if let Some(count) = self.split_column_count {
            properties.push(if count == 0 {
                property("SpanSplitColumnCount", "enumeration", "All")
            } else {
                property("SpanSplitColumnCount", "long", count.to_string())
            });
        }
        for (key, value) in [
            ("SplitColumnInsideGutter", self.inside_gutter),
            ("SplitColumnOutsideGutter", self.outside_gutter),
            ("SpanColumnMinSpaceBefore", self.space_before),
            ("SpanColumnMinSpaceAfter", self.space_after),
        ] {
            push_number(&mut attrs, key, value);
        }
        (attrs, properties)
    }
}

/// Paragraph alignment, spacing, drop cap, baseline grid and composer values after the caller
/// maps application semantics to IDML enumerations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphAlignmentValues {
    pub justification: Option<String>,
    pub direction: Option<String>,
    pub kashidas: Option<String>,
    pub paragraph_justification: Option<String>,
    pub paragraph_kashida_width: Option<f64>,
    pub left_indent: Option<f64>,
    pub right_indent: Option<f64>,
    pub first_line_indent: Option<f64>,
    pub last_line_indent: Option<f64>,
    pub space_before: Option<f64>,
    pub space_after: Option<f64>,
    pub same_para_style_spacing: Option<Option<f64>>,
    pub drop_cap_lines: Option<f64>,
    pub drop_cap_chars: Option<f64>,
    pub dropcap_detail: Option<u32>,
    pub grid_alignment: Option<String>,
    pub grid_align_first_line_only: Option<bool>,
    pub composer: Option<String>,
}

/// Paragraph hyphenation, justification spacing and keep options after caller enum mapping.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphFlowValues {
    pub hyphenate: Option<bool>,
    pub hyphenate_min_word: Option<f64>,
    pub hyphenate_after_first: Option<f64>,
    pub hyphenate_before_last: Option<f64>,
    pub hyphenate_ladder_limit: Option<f64>,
    pub hyphenation_zone: Option<f64>,
    pub hyphenate_capitalized: Option<bool>,
    pub hyphenate_last_word: Option<bool>,
    pub hyphenate_across_columns: Option<bool>,
    pub hyphen_weight: Option<f64>,
    pub word_spacing: [Option<f64>; 3],
    pub letter_spacing: [Option<f64>; 3],
    pub glyph_scaling: [Option<f64>; 3],
    pub auto_leading: Option<f64>,
    pub single_word_justification: Option<String>,
    pub keep_with_next: Option<f64>,
    pub keep_lines_together: Option<bool>,
    pub ignore_edge_alignment: Option<bool>,
    pub keep_all_lines_together: Option<bool>,
    pub keep_first_lines: Option<f64>,
    pub keep_last_lines: Option<f64>,
    pub start_paragraph: Option<String>,
}

/// Raw IDML character typography values before the caller resolves fonts and semantic enums.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterTypographyValues {
    pub applied_font: Option<String>,
    pub font_style: Option<String>,
    pub point_size: Option<f64>,
    pub leading: Option<String>,
    pub kerning_method: Option<String>,
    pub kerning_value: Option<f64>,
    pub tracking: Option<f64>,
    pub leading_aki: Option<f64>,
    pub trailing_aki: Option<f64>,
    pub tsume: Option<f64>,
    pub jidori: Option<f64>,
    pub leading_model: Option<String>,
    pub character_alignment: Option<String>,
    pub horizontal_scale: Option<f64>,
    pub vertical_scale: Option<f64>,
    pub baseline_shift: Option<f64>,
    pub skew: Option<f64>,
}

/// One underline or strikethrough style as written in IDML.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextDecorationValues {
    pub weight: Option<f64>,
    pub offset: Option<f64>,
    pub color: Option<String>,
    pub tint: Option<f64>,
}

/// Character fill/stroke, decoration and toggle values before caller swatch resolution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterAppearanceValues {
    pub fill_color: Option<String>,
    pub fill_tint: Option<f64>,
    pub stroke_color: Option<String>,
    pub stroke_tint: Option<f64>,
    pub stroke_weight: Option<f64>,
    pub stroke_alignment: Option<String>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub ligatures: Option<bool>,
    pub underline_options: TextDecorationValues,
    pub strikethrough_options: TextDecorationValues,
}

/// Raw Ruby attributes and properties on an IDML character range or style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RubyValues {
    pub flag: Option<bool>,
    pub text: Option<String>,
    pub kind: Option<String>,
    pub alignment: Option<String>,
    pub position: Option<String>,
    pub x_offset: Option<f64>,
    pub y_offset: Option<f64>,
    pub font: Option<String>,
    pub font_style: Option<String>,
    pub font_size: Option<f64>,
    pub x_scale: Option<f64>,
    pub y_scale: Option<f64>,
    pub open_type_pro: Option<bool>,
    pub auto_tcy_digits: Option<f64>,
    pub auto_tcy_include_roman: Option<bool>,
    pub auto_tcy_auto_scale: Option<bool>,
    pub overhang: Option<bool>,
    pub parent_overhang: Option<String>,
    pub parent_spacing: Option<String>,
    pub auto_align: Option<bool>,
    pub auto_scaling: Option<bool>,
    pub parent_scaling: Option<f64>,
    pub fill: Option<String>,
    pub stroke: Option<String>,
    pub fill_tint: Option<f64>,
    pub stroke_tint: Option<f64>,
    pub stroke_weight: Option<f64>,
    pub overprint_fill: Option<String>,
    pub overprint_stroke: Option<String>,
}

/// Raw remaining character attributes and properties before semantic enum and condition mapping.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterSpecialValues {
    pub capitalization: Option<String>,
    pub position: Option<String>,
    pub glyph_form: Option<String>,
    pub no_break: Option<bool>,
    pub tate_chu_yoko: Option<bool>,
    pub tate_chu_yoko_x_offset: Option<f64>,
    pub tate_chu_yoko_y_offset: Option<f64>,
    pub warichu: Option<bool>,
    pub warichu_lines: Option<f64>,
    pub warichu_size: Option<f64>,
    pub warichu_line_spacing: Option<f64>,
    pub warichu_alignment: Option<String>,
    pub warichu_chars_before_break: Option<f64>,
    pub warichu_chars_after_break: Option<f64>,
    pub digits: Option<String>,
    pub character_direction: Option<String>,
    pub allow_kashidas: Option<String>,
    pub diacritic_position: Option<String>,
    pub diacritic_x_offset: Option<f64>,
    pub diacritic_y_offset: Option<f64>,
    pub positional_form: Option<String>,
    pub applied_conditions: Option<String>,
    pub applied_language: Option<String>,
}

impl CharacterSpecialValues {
    /// Reads remaining character ranges/style fields from IDML.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            capitalization: values.prop("Capitalization"),
            position: values.prop("Position"),
            glyph_form: values.prop("GlyphForm"),
            no_break: values.boolean("NoBreak"),
            tate_chu_yoko: values.boolean("Tatechuyoko"),
            tate_chu_yoko_x_offset: values.num("TatechuyokoXOffset"),
            tate_chu_yoko_y_offset: values.num("TatechuyokoYOffset"),
            warichu: values.boolean("Warichu"),
            warichu_lines: values.num("WarichuLines"),
            warichu_size: values.num("WarichuSize"),
            warichu_line_spacing: values.num("WarichuLineSpacing"),
            warichu_alignment: values.prop("WarichuAlignment"),
            warichu_chars_before_break: values.num("WarichuCharsBeforeBreak"),
            warichu_chars_after_break: values.num("WarichuCharsAfterBreak"),
            digits: values.prop("DigitsType"),
            character_direction: values.prop("CharacterDirection"),
            allow_kashidas: values.prop("Kashidas"),
            diacritic_position: values.prop("DiacriticPosition"),
            diacritic_x_offset: values.num("XOffsetDiacritic"),
            diacritic_y_offset: values.num("YOffsetDiacritic"),
            positional_form: values.prop("PositionalForm"),
            applied_conditions: values.prop("AppliedConditions"),
            applied_language: values.prop("AppliedLanguage"),
        }
    }
}

impl RubyValues {
    /// Reads Ruby text and formatting values from an IDML character range/style.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            flag: values.boolean("RubyFlag").or_else(|| values.num("RubyFlag").map(|value| value != 0.0)),
            text: values.prop("RubyString"),
            kind: values.prop("RubyType"),
            alignment: values.prop("RubyAlignment"),
            position: values.prop("RubyPosition"),
            x_offset: values.num("RubyXOffset"),
            y_offset: values.num("RubyYOffset"),
            font: values.prop("RubyFont"),
            font_style: values.prop("RubyFontStyle"),
            font_size: values.num("RubyFontSize"),
            x_scale: values.num("RubyXScale"),
            y_scale: values.num("RubyYScale"),
            open_type_pro: values.boolean("RubyOpenTypePro"),
            auto_tcy_digits: values.num("RubyAutoTcyDigits"),
            auto_tcy_include_roman: values.boolean("RubyAutoTcyIncludeRoman"),
            auto_tcy_auto_scale: values.boolean("RubyAutoTcyAutoScale"),
            overhang: values.boolean("RubyOverhang"),
            parent_overhang: values.prop("RubyParentOverhangAmount"),
            parent_spacing: values.prop("RubyParentSpacing"),
            auto_align: values.boolean("RubyAutoAlign"),
            auto_scaling: values.boolean("RubyAutoScaling"),
            parent_scaling: values.num("RubyParentScalingPercent"),
            fill: values.prop("RubyFill"),
            stroke: values.prop("RubyStroke"),
            fill_tint: values.num("RubyTint"),
            stroke_tint: values.num("RubyStrokeTint"),
            stroke_weight: values.num("RubyWeight"),
            overprint_fill: values.prop("RubyOverprintFill"),
            overprint_stroke: values.prop("RubyOverprintStroke"),
        }
    }
}

impl CharacterAppearanceValues {
    /// Reads common character fill, stroke and decoration properties from IDML.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let decoration = |prefix: &str| {
            let number = |suffix: &str| values.num(&format!("{prefix}{suffix}")).filter(|value| *value > -9000.0);
            TextDecorationValues {
                weight: number("Weight"),
                offset: number("Offset"),
                color: values.prop(&format!("{prefix}Color")),
                tint: values.num(&format!("{prefix}Tint")),
            }
        };
        Self {
            fill_color: values.prop("FillColor"),
            fill_tint: values.num("FillTint"),
            stroke_color: values.prop("StrokeColor"),
            stroke_tint: values.num("StrokeTint"),
            stroke_weight: values.num("StrokeWeight"),
            stroke_alignment: values.prop("StrokeAlignment"),
            underline: values.boolean("Underline"),
            strikethrough: values.boolean("StrikeThru"),
            ligatures: values.boolean("Ligatures"),
            underline_options: decoration("Underline"),
            strikethrough_options: decoration("StrikeThru"),
        }
    }
}

impl CharacterTypographyValues {
    /// Reads the common typography attributes and properties from an IDML character range/style.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            applied_font: values.prop("AppliedFont"),
            font_style: values.prop("FontStyle"),
            point_size: values.num("PointSize"),
            leading: values.prop("Leading"),
            kerning_method: values.prop("KerningMethod"),
            kerning_value: values.num("KerningValue"),
            tracking: values.num("Tracking"),
            leading_aki: values.num("LeadingAki"),
            trailing_aki: values.num("TrailingAki"),
            tsume: values.num("Tsume"),
            jidori: values.num("Jidori"),
            leading_model: values.prop("LeadingModel"),
            character_alignment: values.prop("CharacterAlignment"),
            horizontal_scale: values.num("HorizontalScale"),
            vertical_scale: values.num("VerticalScale"),
            baseline_shift: values.num("BaselineShift"),
            skew: values.num("Skew"),
        }
    }
}

impl ParagraphFlowValues {
    /// Reads paragraph hyphenation and flow properties from IDML.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            hyphenate: values.boolean("Hyphenation"),
            hyphenate_min_word: values.num("HyphenateWordsLongerThan"),
            hyphenate_after_first: values.num("HyphenateAfterFirst"),
            hyphenate_before_last: values.num("HyphenateBeforeLast"),
            hyphenate_ladder_limit: values.num("HyphenateLadderLimit"),
            hyphenation_zone: values.num("HyphenationZone"),
            hyphenate_capitalized: values.boolean("HyphenateCapitalizedWords"),
            hyphenate_last_word: values.boolean("HyphenateLastWord"),
            hyphenate_across_columns: values.boolean("HyphenateAcrossColumns"),
            hyphen_weight: values.num("HyphenWeight"),
            word_spacing: [values.num("MinimumWordSpacing"), values.num("DesiredWordSpacing"), values.num("MaximumWordSpacing")],
            letter_spacing: [values.num("MinimumLetterSpacing"), values.num("DesiredLetterSpacing"), values.num("MaximumLetterSpacing")],
            glyph_scaling: [values.num("MinimumGlyphScaling"), values.num("DesiredGlyphScaling"), values.num("MaximumGlyphScaling")],
            auto_leading: values.num("AutoLeading"),
            single_word_justification: values.prop("SingleWordJustification"),
            keep_with_next: values.num("KeepWithNext"),
            keep_lines_together: values.boolean("KeepLinesTogether"),
            ignore_edge_alignment: values.boolean("IgnoreEdgeAlignment"),
            keep_all_lines_together: values.boolean("KeepAllLinesTogether"),
            keep_first_lines: values.num("KeepFirstLines"),
            keep_last_lines: values.num("KeepLastLines"),
            start_paragraph: values.prop("StartParagraph"),
        }
    }

    /// Writes IDML attributes for paragraph hyphenation and flow.
    pub fn to_attrs(&self) -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        for (key, value) in [
            ("Hyphenation", self.hyphenate),
            ("HyphenateCapitalizedWords", self.hyphenate_capitalized),
            ("HyphenateLastWord", self.hyphenate_last_word),
            ("HyphenateAcrossColumns", self.hyphenate_across_columns),
            ("KeepLinesTogether", self.keep_lines_together),
            ("IgnoreEdgeAlignment", self.ignore_edge_alignment),
            ("KeepAllLinesTogether", self.keep_all_lines_together),
        ] {
            if let Some(value) = value {
                attrs.push((key.to_string(), if value { "true" } else { "false" }.to_string()));
            }
        }
        for (key, value) in [
            ("HyphenateWordsLongerThan", self.hyphenate_min_word),
            ("HyphenateAfterFirst", self.hyphenate_after_first),
            ("HyphenateBeforeLast", self.hyphenate_before_last),
            ("HyphenateLadderLimit", self.hyphenate_ladder_limit),
            ("HyphenationZone", self.hyphenation_zone),
            ("HyphenWeight", self.hyphen_weight),
            ("MinimumWordSpacing", self.word_spacing[0]),
            ("DesiredWordSpacing", self.word_spacing[1]),
            ("MaximumWordSpacing", self.word_spacing[2]),
            ("MinimumLetterSpacing", self.letter_spacing[0]),
            ("DesiredLetterSpacing", self.letter_spacing[1]),
            ("MaximumLetterSpacing", self.letter_spacing[2]),
            ("MinimumGlyphScaling", self.glyph_scaling[0]),
            ("DesiredGlyphScaling", self.glyph_scaling[1]),
            ("MaximumGlyphScaling", self.glyph_scaling[2]),
            ("AutoLeading", self.auto_leading),
            ("KeepWithNext", self.keep_with_next),
            ("KeepFirstLines", self.keep_first_lines),
            ("KeepLastLines", self.keep_last_lines),
        ] {
            if let Some(value) = value {
                attrs.push((key.to_string(), crate::value::num(value)));
            }
        }
        for (key, value) in [("SingleWordJustification", &self.single_word_justification), ("StartParagraph", &self.start_paragraph)] {
            if let Some(value) = value {
                attrs.push((key.to_string(), value.clone()));
            }
        }
        attrs
    }
}

impl ParagraphAlignmentValues {
    /// Reads paragraph alignment and spacing properties from IDML.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            justification: values.prop("Justification"),
            direction: values.prop("ParagraphDirection"),
            kashidas: values.prop("Kashidas"),
            paragraph_justification: values.prop("ParagraphJustification"),
            paragraph_kashida_width: values.num("ParagraphKashidaWidth"),
            left_indent: values.num("LeftIndent"),
            right_indent: values.num("RightIndent"),
            first_line_indent: values.num("FirstLineIndent"),
            last_line_indent: values.num("LastLineIndent"),
            space_before: values.num("SpaceBefore"),
            space_after: values.num("SpaceAfter"),
            same_para_style_spacing: values.prop("SameParaStyleSpacing").map(|value| value.trim().parse::<f64>().ok().filter(|v| v.is_finite())),
            drop_cap_lines: values.num("DropCapLines"),
            drop_cap_chars: values.num("DropCapCharacters"),
            dropcap_detail: values.num("DropcapDetail").filter(|value| *value >= 0.0).map(|value| value as u32),
            grid_alignment: values.prop("GridAlignment"),
            grid_align_first_line_only: values.boolean("GridAlignFirstLineOnly"),
            composer: values.prop("Composer"),
        }
    }

    /// Writes the IDML attributes for paragraph alignment and spacing.
    pub fn to_attrs(&self) -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        for (key, value) in [
            ("Justification", &self.justification),
            ("ParagraphDirection", &self.direction),
            ("Kashidas", &self.kashidas),
            ("ParagraphJustification", &self.paragraph_justification),
            ("GridAlignment", &self.grid_alignment),
            ("Composer", &self.composer),
        ] {
            if let Some(value) = value {
                attrs.push((key.to_string(), value.clone()));
            }
        }
        for (key, value) in [
            ("ParagraphKashidaWidth", self.paragraph_kashida_width),
            ("LeftIndent", self.left_indent),
            ("RightIndent", self.right_indent),
            ("FirstLineIndent", self.first_line_indent),
            ("LastLineIndent", self.last_line_indent),
            ("SpaceBefore", self.space_before),
            ("SpaceAfter", self.space_after),
            ("DropCapLines", self.drop_cap_lines),
            ("DropCapCharacters", self.drop_cap_chars),
        ] {
            if let Some(value) = value {
                attrs.push((key.to_string(), crate::value::num(value)));
            }
        }
        if let Some(value) = self.same_para_style_spacing {
            attrs.push(("SameParaStyleSpacing".to_string(), value.map(crate::value::num).unwrap_or_else(|| "Nothing".to_string())));
        }
        if let Some(value) = self.dropcap_detail {
            attrs.push(("DropcapDetail".to_string(), value.to_string()));
        }
        if let Some(value) = self.grid_align_first_line_only {
            attrs.push(("GridAlignFirstLineOnly".to_string(), if value { "true" } else { "false" }.to_string()));
        }
        attrs
    }
}

impl ParagraphBoxValues {
    /// Reads paragraph border and shading values from IDML.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let sides = |prefix: &str, suffix: &str| {
            let values = ["Top", "Left", "Bottom", "Right"].map(|side| values.num(&format!("{prefix}{side}{suffix}")));
            values.iter().any(Option::is_some).then(|| values.map(|value| value.unwrap_or(0.0)))
        };
        let corners = |prefix: &str| {
            let names = ["TopLeft", "TopRight", "BottomRight", "BottomLeft"];
            let shapes = names.map(|corner| values.prop(&format!("{prefix}{corner}CornerOption")));
            let sizes = names.map(|corner| values.num(&format!("{prefix}{corner}CornerRadius")));
            if shapes.iter().all(Option::is_none) && sizes.iter().all(Option::is_none) {
                None
            } else {
                Some(std::array::from_fn(|index| {
                    (shapes[index].clone().unwrap_or_else(|| "None".to_string()), sizes[index].map_or(12.0, |value| value.max(0.0)))
                }))
            }
        };
        Self {
            shading_on: values.boolean("ParagraphShadingOn"),
            shading_color: values.prop("ParagraphShadingColor"),
            shading_tint: values.num("ParagraphShadingTint"),
            shading_offsets: sides("ParagraphShading", "Offset"),
            shading_overprint: values.boolean("ParagraphShadingOverprint"),
            border_on: values.boolean("ParagraphBorderOn"),
            border_color: values.prop("ParagraphBorderColor"),
            border_tint: values.num("ParagraphBorderTint"),
            border_weights: sides("ParagraphBorder", "LineWeight"),
            border_offsets: sides("ParagraphBorder", "Offset"),
            border_type: values.prop("ParagraphBorderType"),
            border_cap: values.prop("ParagraphBorderStrokeEndCap"),
            border_join: values.prop("ParagraphBorderStrokeEndJoin"),
            border_corners: corners("ParagraphBorder"),
            border_width: values.prop("ParagraphBorderWidth"),
            border_top: values.prop("ParagraphBorderTopOrigin"),
            border_bottom: values.prop("ParagraphBorderBottomOrigin"),
            border_display_if_splits: values.boolean("ParagraphBorderDisplayIfSplits"),
            border_merge: values.boolean("MergeConsecutiveParaBorders"),
            border_gap_color: values.prop("ParagraphBorderGapColor"),
            border_gap_tint: values.num("ParagraphBorderGapTint"),
            border_overprint: values.boolean("ParagraphBorderOverprint"),
            border_gap_overprint: values.boolean("ParagraphBorderGapOverprint"),
            shading_width: values.prop("ParagraphShadingWidth"),
            shading_top: values.prop("ParagraphShadingTopOrigin"),
            shading_bottom: values.prop("ParagraphShadingBottomOrigin"),
            shading_clip: values.boolean("ParagraphShadingClipToFrame"),
            shading_nonprinting: values.boolean("ParagraphShadingSuppressPrinting"),
            shading_corners: corners("ParagraphShading"),
        }
    }

    /// Writes IDML attributes and properties for paragraph borders and shading.
    pub fn to_parts(&self) -> (Vec<(String, String)>, Vec<El>) {
        use crate::value::num;

        let mut attrs = Vec::new();
        let mut props = Vec::new();
        push_boolean(&mut attrs, "ParagraphShadingOn", self.shading_on);
        if let Some(color) = &self.shading_color {
            props.push(property("ParagraphShadingColor", "object", color.clone()));
        }
        push_number(&mut attrs, "ParagraphShadingTint", self.shading_tint);
        push_boolean(&mut attrs, "ParagraphShadingOverprint", self.shading_overprint);
        const SIDES: [&str; 4] = ["Top", "Left", "Bottom", "Right"];
        if let Some(offsets) = self.shading_offsets {
            for (side, value) in SIDES.iter().zip(offsets) {
                push_number(&mut attrs, &format!("ParagraphShading{side}Offset"), Some(value));
            }
        }
        push_boolean(&mut attrs, "ParagraphBorderOn", self.border_on);
        if let Some(color) = &self.border_color {
            props.push(property("ParagraphBorderColor", "object", color.clone()));
        }
        push_number(&mut attrs, "ParagraphBorderTint", self.border_tint);
        push_boolean(&mut attrs, "ParagraphBorderOverprint", self.border_overprint);
        if let Some(weights) = self.border_weights {
            for (side, value) in SIDES.iter().zip(weights) {
                push_number(&mut attrs, &format!("ParagraphBorder{side}LineWeight"), Some(value));
            }
        }
        if let Some(offsets) = self.border_offsets {
            for (side, value) in SIDES.iter().zip(offsets) {
                push_number(&mut attrs, &format!("ParagraphBorder{side}Offset"), Some(value));
            }
        }
        if let Some(kind) = &self.border_type {
            props.push(property("ParagraphBorderType", "object", kind.clone()));
        }
        if let Some(cap) = &self.border_cap {
            attrs.push(("ParagraphBorderStrokeEndCap".into(), cap.clone()));
        }
        if let Some(join) = &self.border_join {
            attrs.push(("ParagraphBorderStrokeEndJoin".into(), join.clone()));
        }
        for (prefix, corners) in [("ParagraphBorder", &self.border_corners), ("ParagraphShading", &self.shading_corners)] {
            if let Some(corners) = corners {
                const CORNERS: [&str; 4] = ["TopLeft", "TopRight", "BottomRight", "BottomLeft"];
                for ((shape, radius), corner) in corners.iter().zip(CORNERS) {
                    attrs.push((format!("{prefix}{corner}CornerOption"), shape.clone()));
                    attrs.push((format!("{prefix}{corner}CornerRadius"), num(*radius)));
                }
            }
        }
        for (key, value) in [
            ("ParagraphBorderWidth", &self.border_width),
            ("ParagraphBorderTopOrigin", &self.border_top),
            ("ParagraphBorderBottomOrigin", &self.border_bottom),
            ("ParagraphShadingWidth", &self.shading_width),
            ("ParagraphShadingTopOrigin", &self.shading_top),
            ("ParagraphShadingBottomOrigin", &self.shading_bottom),
        ] {
            if let Some(value) = value {
                attrs.push((key.to_string(), value.clone()));
            }
        }
        push_boolean(&mut attrs, "ParagraphBorderDisplayIfSplits", self.border_display_if_splits);
        push_boolean(&mut attrs, "MergeConsecutiveParaBorders", self.border_merge);
        if let Some(color) = &self.border_gap_color {
            props.push(property("ParagraphBorderGapColor", "object", color.clone()));
        }
        push_number(&mut attrs, "ParagraphBorderGapTint", self.border_gap_tint);
        push_boolean(&mut attrs, "ParagraphBorderGapOverprint", self.border_gap_overprint);
        push_boolean(&mut attrs, "ParagraphShadingClipToFrame", self.shading_clip);
        push_boolean(&mut attrs, "ParagraphShadingSuppressPrinting", self.shading_nonprinting);
        (attrs, props)
    }
}

fn push_boolean(attrs: &mut Vec<(String, String)>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        attrs.push((key.to_string(), if value { "true" } else { "false" }.to_string()));
    }
}

fn push_number(attrs: &mut Vec<(String, String)>, key: &str, value: Option<f64>) {
    if let Some(value) = value {
        attrs.push((key.to_string(), crate::value::num(value)));
    }
}

impl ParagraphListValues {
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let restart = values.prop_el("NumberingRestartPolicies");
        let restart_after_level = restart.map(|policy| policy.attr("LowerLevel").and_then(|value| value.trim().parse::<u32>().ok()).unwrap_or(0));
        let restart_after_specific_level = restart.is_some_and(|policy| policy.attr("RestartPolicy").map(str::trim) == Some("SpecificPreviousLevel"));
        let bullet = values.prop_el("BulletChar");
        Self {
            applied_list: values.prop("AppliedNumberingList"),
            list_type: values.prop("BulletsAndNumberingListType"),
            numbering_format: values.prop("NumberingFormat"),
            expression: values.prop("NumberingExpression"),
            separator: values.prop("BulletsTextAfter"),
            bullet_type: bullet.and_then(|value| value.attr("BulletCharacterType")).map(str::to_owned),
            bullet_value: bullet.and_then(|value| value.attr("BulletCharacterValue")).map(str::to_owned),
            balance_ragged: values.prop("BalanceRaggedLines"),
            level: values.num("NumberingLevel"),
            bullet_character_style: values.prop("BulletsCharacterStyle"),
            numbering_character_style: values.prop("NumberingCharacterStyle"),
            bullet_alignment: values.prop("BulletsAlignment"),
            numbering_alignment: values.prop("NumberingAlignment"),
            restart_numbers: values.boolean("NumberingApplyRestartPolicy"),
            restart_after_level,
            restart_after_specific_level,
            numbering_continue: values.boolean("NumberingContinue"),
            numbering_start_at: values.num("NumberingStartAt"),
            bullet_font: values.prop("BulletsFont"),
            bullet_font_style: values.prop("BulletsFontStyle"),
        }
    }

    /// Encodes a semantic paragraph-list name as its IDML object reference.
    pub fn numbering_list_reference(name: &str) -> String {
        if name.is_empty() { "NumberingList/$ID/[Default]".into() } else { format!("NumberingList/{}", crate::value::escape_id(name)) }
    }

    /// IDML attributes represented by a paragraph's list settings.
    pub fn to_attrs(&self) -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        if let Some(value) = &self.applied_list {
            attrs.push(("AppliedNumberingList".into(), value.clone()));
        }
        if let Some(value) = self.numbering_continue {
            attrs.push(("NumberingContinue".into(), if value { "true" } else { "false" }.into()));
        }
        push_number(&mut attrs, "NumberingStartAt", self.numbering_start_at);
        for (key, value) in [
            ("BulletsAndNumberingListType", &self.list_type),
            ("NumberingExpression", &self.expression),
            ("BulletsTextAfter", &self.separator),
            ("NumberingLevel", &self.level.map(crate::value::num)),
            ("BulletsAlignment", &self.bullet_alignment),
            ("NumberingAlignment", &self.numbering_alignment),
            ("NumberingApplyRestartPolicy", &self.restart_numbers.map(|value| if value { "true".to_string() } else { "false".to_string() })),
        ] {
            if let Some(value) = value {
                attrs.push((key.to_string(), value.clone()));
            }
        }
        attrs
    }

    /// IDML property elements represented by a paragraph's list settings.
    pub fn to_els(&self) -> Vec<El> {
        let mut properties = Vec::new();
        if let Some(format) = &self.numbering_format {
            properties.push(property("NumberingFormat", "string", format.clone()));
        }
        if let (Some(kind), Some(value)) = (&self.bullet_type, &self.bullet_value) {
            properties.push(El::new("BulletChar").with_attr("BulletCharacterType", kind).with_attr("BulletCharacterValue", value));
        }
        if let Some(balance) = &self.balance_ragged {
            properties.push(property("BalanceRaggedLines", "enumeration", balance.clone()));
        }
        for (name, style) in [("BulletsCharacterStyle", &self.bullet_character_style), ("NumberingCharacterStyle", &self.numbering_character_style)] {
            if let Some(style) = style {
                properties.push(property(name, "object", style.clone()));
            }
        }
        if let Some(level) = self.restart_after_level {
            let mut restart = El::new("NumberingRestartPolicies")
                .with_attr("RestartPolicy", if self.restart_after_specific_level { "SpecificPreviousLevel" } else { "AnyPreviousLevel" })
                .with_attr("LowerLevel", level.to_string());
            restart.set_attr("UpperLevel", level.to_string());
            properties.push(restart);
        }
        if let Some(font) = &self.bullet_font {
            properties.push(property("BulletsFont", "string", if font.is_empty() { "$ID/".into() } else { font.clone() }));
        }
        if let Some(style) = self.bullet_font_style.as_ref().filter(|style| !style.is_empty()) {
            properties.push(property("BulletsFontStyle", "string", style.clone()));
        }
        properties
    }
}

/// One IDML nested paragraph style entry, before the caller resolves its character style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NestedStyleValue {
    pub character_style: String,
    pub inclusive: bool,
    pub repetition: u32,
    pub delimiter_type: String,
    pub delimiter: String,
}

/// One IDML nested line style entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NestedLineStyleValue {
    pub character_style: String,
    pub line_count: u32,
}

/// One IDML GREP style entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrepStyleValue {
    pub character_style: String,
    pub expression: String,
}

/// Nested style lists stored on an IDML paragraph style or text range.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NestedStyleValues {
    pub nested: Option<Vec<NestedStyleValue>>,
    pub lines: Option<Vec<NestedLineStyleValue>>,
    pub grep: Option<Vec<GrepStyleValue>>,
}

impl NestedStyleValues {
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let text = |item: &El, name: &str| item.find(name).map(El::deep_text).unwrap_or_default();
        let nested = values.prop_el("AllNestedStyles").map(|list| {
            list.find_all("ListItem")
                .map(|item| {
                    let delimiter = item.find("Delimiter");
                    NestedStyleValue {
                        character_style: text(item, "AppliedCharacterStyle"),
                        inclusive: text(item, "Inclusive").trim() != "false",
                        repetition: text(item, "Repetition").trim().parse().unwrap_or(1),
                        delimiter_type: delimiter.and_then(|el| el.attr("type")).unwrap_or("enumeration").to_owned(),
                        delimiter: text(item, "Delimiter"),
                    }
                })
                .collect()
        });
        let lines = values.prop_el("AllNestedLineStyles").map(|list| {
            list.find_all("ListItem")
                .map(|item| NestedLineStyleValue {
                    character_style: text(item, "AppliedCharacterStyle"),
                    line_count: text(item, "LineCount").trim().parse().unwrap_or(1),
                })
                .collect()
        });
        let grep = values.prop_el("AllGREPStyles").map(|list| {
            list.find_all("ListItem")
                .map(|item| GrepStyleValue { character_style: text(item, "AppliedCharacterStyle"), expression: text(item, "GrepExpression") })
                .collect()
        });
        Self { nested, lines, grep }
    }

    /// Writes the nested style property lists. The caller resolves semantic style references and
    /// delimiter enums into the IDML values held in this model.
    pub fn to_els(&self) -> Vec<El> {
        let mut properties = Vec::new();
        if let Some(entries) = &self.nested {
            let mut list = El::new("AllNestedStyles").with_attr("type", "list");
            for entry in entries {
                list.push(
                    El::new("ListItem")
                        .with_attr("type", "record")
                        .with_child(property("AppliedCharacterStyle", "object", entry.character_style.clone()))
                        .with_child(property("Delimiter", &entry.delimiter_type, entry.delimiter.clone()))
                        .with_child(property("Repetition", "long", entry.repetition.to_string()))
                        .with_child(property("Inclusive", "boolean", if entry.inclusive { "true" } else { "false" })),
                );
            }
            properties.push(list);
        }
        if let Some(entries) = &self.lines {
            let mut list = El::new("AllNestedLineStyles").with_attr("type", "list");
            for entry in entries {
                list.push(
                    El::new("ListItem")
                        .with_attr("type", "record")
                        .with_child(property("AppliedCharacterStyle", "object", entry.character_style.clone()))
                        .with_child(property("LineCount", "long", entry.line_count.to_string())),
                );
            }
            properties.push(list);
        }
        if let Some(entries) = &self.grep {
            let mut list = El::new("AllGREPStyles").with_attr("type", "list");
            for entry in entries {
                list.push(
                    El::new("ListItem")
                        .with_attr("type", "record")
                        .with_child(property("AppliedCharacterStyle", "object", entry.character_style.clone()))
                        .with_child(property("GrepExpression", "string", entry.expression.clone())),
                );
            }
            properties.push(list);
        }
        properties
    }
}

fn property(name: &str, kind: &str, value: impl Into<String>) -> El {
    El::new(name).with_attr("type", kind).with_text(value)
}

/// OpenType feature tags represented by IDML character attributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenTypeFeatureValues {
    pub features: Vec<String>,
    pub present: bool,
}

impl OpenTypeFeatureValues {
    /// Reads IDML OTF toggle, figure style and stylistic-set attributes into feature tags.
    pub fn from_el(el: &El) -> Self {
        const TOGGLES: &[(&str, &str)] = &[
            ("dlig", "OTFDiscretionaryLigature"),
            ("frac", "OTFFraction"),
            ("ordn", "OTFOrdinal"),
            ("swsh", "OTFSwash"),
            ("titl", "OTFTitling"),
            ("calt", "OTFContextualAlternate"),
            ("zero", "OTFSlashedZero"),
            ("palt", "OTFProportionalMetrics"),
            ("hkna", "OTFHVKana"),
            ("locl", "OTFLocale"),
            ("mark", "OTFMark"),
            ("hist", "OTFHistorical"),
            ("salt", "OTFStylisticAlternate"),
            ("jalt", "OTFJustificationAlternate"),
            ("ital", "OTFRomanItalics"),
        ];
        let values = Values::of(el);
        let mut result = Self::default();
        for (tag, attribute) in TOGGLES {
            let Some(on) = values.boolean(attribute) else { continue };
            let default_on = matches!(*tag, "calt" | "liga" | "locl" | "mark");
            result.features.retain(|feature| feature != tag && feature.strip_prefix('-') != Some(*tag));
            if on != default_on {
                result.features.push(if on { (*tag).to_owned() } else { format!("-{tag}") });
            }
            result.present = true;
        }
        if let Some(style) = values.value("OTFFigureStyle") {
            let tags: &[&str] = match style.as_str() {
                "TabularLining" => &["tnum", "lnum"],
                "ProportionalOldstyle" => &["pnum", "onum"],
                "ProportionalLining" => &["pnum", "lnum"],
                "TabularOldstyle" => &["tnum", "onum"],
                "Default" => &[],
                _ => &[],
            };
            if matches!(style.as_str(), "TabularLining" | "ProportionalOldstyle" | "ProportionalLining" | "TabularOldstyle" | "Default") {
                result.features.retain(|feature| !matches!(feature.as_str(), "tnum" | "pnum" | "lnum" | "onum"));
                result.features.extend(tags.iter().map(|tag| (*tag).to_owned()));
                result.present = true;
            }
        }
        if let Some(mask) = values.num("OTFStylisticSets") {
            let mask = mask.max(0.0) as u32;
            result.features.retain(|feature| !(feature.len() == 4 && feature.starts_with("ss") && feature[2..].parse::<u32>().is_ok()));
            result.features.extend((1..=20).filter(|number| mask & (1 << (number - 1)) != 0).map(|number| format!("ss{number:02}")));
            result.present = true;
        }
        result
    }
}

/// Raw character-level Japanese composition and emphasis properties from an IDML text range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CjkCharacterValues {
    pub kenten_kind: Option<String>,
    pub kenten_custom_character: Option<String>,
    pub kenten_font: Option<String>,
    pub kenten_font_style: Option<String>,
    pub kenten_size: Option<f64>,
    pub kenten_x_scale: Option<f64>,
    pub kenten_y_scale: Option<f64>,
    pub kenten_distance: Option<f64>,
    pub kenten_position: Option<String>,
    pub kenten_alignment: Option<String>,
    pub kenten_character_set: Option<String>,
    pub kenten_fill_color: Option<String>,
    pub kenten_stroke_color: Option<String>,
    pub kenten_fill_tint: Option<f64>,
    pub kenten_stroke_tint: Option<f64>,
    pub kenten_stroke_weight: Option<f64>,
    pub kenten_overprint_fill: Option<String>,
    pub kenten_overprint_stroke: Option<String>,
    pub shatai_magnification: Option<f64>,
    pub shatai_angle: Option<f64>,
    pub shatai_adjust_rotation: Option<bool>,
    pub shatai_adjust_tsume: Option<bool>,
}

impl CjkCharacterValues {
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            kenten_kind: values.get("KentenKind").map(str::to_owned),
            kenten_custom_character: values.prop("KentenCustomCharacter"),
            kenten_font: values.prop("KentenFont"),
            kenten_font_style: values.prop("KentenFontStyle"),
            kenten_size: values.num("KentenFontSize"),
            kenten_x_scale: values.num("KentenXScale"),
            kenten_y_scale: values.num("KentenYScale"),
            kenten_distance: values.num("KentenPlacement"),
            kenten_position: values.prop("KentenPosition"),
            kenten_alignment: values.prop("KentenAlignment"),
            kenten_character_set: values.prop("KentenCharacterSet"),
            kenten_fill_color: values.prop("KentenFillColor"),
            kenten_stroke_color: values.prop("KentenStrokeColor"),
            kenten_fill_tint: values.num("KentenTint"),
            kenten_stroke_tint: values.num("KentenStrokeTint"),
            kenten_stroke_weight: values.num("KentenWeight"),
            kenten_overprint_fill: values.prop("KentenOverprintFill"),
            kenten_overprint_stroke: values.prop("KentenOverprintStroke"),
            shatai_magnification: values.num("ShataiMagnification"),
            shatai_angle: values.num("ShataiDegreeAngle"),
            shatai_adjust_rotation: values.boolean("ShataiAdjustRotation"),
            shatai_adjust_tsume: values.boolean("ShataiAdjustTsume"),
        }
    }
}

/// Numeric and boolean values in an IDML `MojikumiTable` row.
#[derive(Clone, Debug, PartialEq)]
pub struct MojikumiRow {
    pub target_class: i16,
    pub side_class: i16,
    pub after: bool,
    pub minimum: f64,
    pub desired: f64,
    pub maximum: f64,
    pub priority: i16,
    pub does_not_float: bool,
}

impl MojikumiRow {
    /// Parses the IDML mojikumi row attributes with bounded integer and finite-number checks.
    pub fn from_el(el: &El) -> Result<Self, String> {
        let values = Values::of(el);
        let number = |key: &str| -> Result<f64, String> {
            let Some(raw) = values.get(key) else { return Ok(0.0) };
            raw.trim().parse::<f64>().ok().filter(|value| value.is_finite()).ok_or_else(|| format!("Mojikumi row has invalid {key}"))
        };
        let integer = |key: &str| -> Result<i16, String> {
            let value = number(key)?;
            if value.fract() != 0.0 || value < f64::from(i16::MIN) || value > f64::from(i16::MAX) {
                return Err(format!("Mojikumi {key} must be an in-range integer"));
            }
            Ok(value as i16)
        };
        let boolean = |key: &str| -> Result<bool, String> {
            match values.get(key).map(str::trim) {
                None | Some("false") => Ok(false),
                Some("true") => Ok(true),
                _ => Err(format!("Mojikumi row has invalid {key}")),
            }
        };
        Ok(Self {
            target_class: integer("TargetMojikumiClass")?,
            side_class: integer("SideMojikumiClass")?,
            after: boolean("SideIsAfterTarget")?,
            minimum: number("Minimum")?,
            desired: number("Desired")?,
            maximum: number("Maximum")?,
            priority: integer("CompressionPriority")?,
            does_not_float: boolean("AkiDoesNotFloat")?,
        })
    }
}

/// One IDML tab stop, before the caller maps alignment names into its text model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabStopValues {
    pub position: f64,
    pub alignment: String,
    pub leader: String,
    pub alignment_character: String,
}

impl TabStopValues {
    /// Reads `ListItem` records in a `TabList` property. Invalid or non-finite positions use 0,
    /// matching IDML import fallback behavior; semantic sanitization belongs to the caller.
    pub fn from_list(list: &El) -> Vec<Self> {
        list.children("ListItem")
            .map(|item| {
                let text = |name: &str| item.child_text(name).map(|value| value.into_owned());
                Self {
                    position: text("Position").and_then(|value| value.trim().parse::<f64>().ok()).filter(|value| value.is_finite()).unwrap_or(0.0),
                    alignment: text("Alignment").unwrap_or_default(),
                    leader: text("Leader").unwrap_or_default(),
                    alignment_character: text("AlignmentCharacter").unwrap_or_default(),
                }
            })
            .collect()
    }

    /// Writes tab-stop records in a `TabList` property.
    pub fn to_list(stops: &[Self]) -> El {
        let mut list = El::new("TabList").with_attr("type", "list");
        for stop in stops {
            list.push(
                El::new("ListItem")
                    .with_attr("type", "record")
                    .with_child(property("Alignment", "enumeration", stop.alignment.clone()))
                    .with_child(property("AlignmentCharacter", "string", stop.alignment_character.clone()))
                    .with_child(property("Leader", "string", stop.leader.clone()))
                    .with_child(property("Position", "unit", crate::value::num(stop.position))),
            );
        }
        list
    }
}

/// What a style styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleKind {
    Paragraph,
    Character,
    Object,
    Cell,
    Table,
}

impl StyleKind {
    pub const ALL: [StyleKind; 5] = [StyleKind::Paragraph, StyleKind::Character, StyleKind::Object, StyleKind::Cell, StyleKind::Table];

    /// The element name of a style: `ParagraphStyle`, ….
    pub fn style(self) -> &'static str {
        match self {
            StyleKind::Paragraph => "ParagraphStyle",
            StyleKind::Character => "CharacterStyle",
            StyleKind::Object => "ObjectStyle",
            StyleKind::Cell => "CellStyle",
            StyleKind::Table => "TableStyle",
        }
    }

    /// The element name of a group: `ParagraphStyleGroup`, ….
    pub fn group(self) -> &'static str {
        match self {
            StyleKind::Paragraph => "ParagraphStyleGroup",
            StyleKind::Character => "CharacterStyleGroup",
            StyleKind::Object => "ObjectStyleGroup",
            StyleKind::Cell => "CellStyleGroup",
            StyleKind::Table => "TableStyleGroup",
        }
    }

    /// The element name of the root group: `RootParagraphStyleGroup`, ….
    pub fn root(self) -> &'static str {
        match self {
            StyleKind::Paragraph => "RootParagraphStyleGroup",
            StyleKind::Character => "RootCharacterStyleGroup",
            StyleKind::Object => "RootObjectStyleGroup",
            StyleKind::Cell => "RootCellStyleGroup",
            StyleKind::Table => "RootTableStyleGroup",
        }
    }

    /// The kind whose root group is called `name`.
    pub fn of_root(name: &str) -> Option<StyleKind> {
        StyleKind::ALL.into_iter().find(|k| k.root() == name)
    }
}

/// A style group: a root group (`RootParagraphStyleGroup`, …) or a group in one.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleGroup {
    pub kind: StyleKind,
    /// The root group of its kind.
    pub root: bool,
    pub values: Values,
    pub children: Vec<StyleNode>,
}

/// Builds the nested style-folder tree written under an IDML root style group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleGroupWriter {
    kind: Option<StyleKind>,
    root_self_id: String,
    styles: Vec<El>,
    groups: Vec<(String, StyleGroupWriter)>,
}

impl StyleGroupWriter {
    pub fn new(kind: StyleKind, root_self_id: impl Into<String>) -> Self {
        Self { kind: Some(kind), root_self_id: root_self_id.into(), ..Self::default() }
    }

    /// Adds a style at its slash-separated folder path. Built-in names in brackets remain at
    /// the root, and folder nesting is capped at 32 levels.
    pub fn insert(&mut self, name: &str, style: El) {
        let builtin = name.starts_with('[');
        let mut segments: Vec<&str> = if builtin { vec![name] } else { name.split('/').collect() };
        segments.pop();
        let mut group = self;
        for segment in segments.into_iter().take(32) {
            let index = match group.groups.iter().position(|(name, _)| name == segment) {
                Some(index) => index,
                None => {
                    group.groups.push((segment.to_string(), StyleGroupWriter { kind: group.kind, ..Self::default() }));
                    group.groups.len() - 1
                }
            };
            group = &mut group.groups[index].1;
        }
        group.styles.push(style);
    }

    /// The complete root style group and its nested groups.
    pub fn to_el(self) -> El {
        let Some(kind) = self.kind else {
            return El::new("RootStyleGroup");
        };
        let root_self_id = self.root_self_id.clone();
        self.emit(kind.root(), &root_self_id, kind.group(), "")
    }

    fn emit(self, root_name: &str, self_id: &str, group_name: &str, path: &str) -> El {
        let mut element = El::new(root_name).with_attr("Self", self_id);
        for style in self.styles {
            element.push(style);
        }
        for (name, group) in self.groups {
            let full = if path.is_empty() { name.clone() } else { format!("{path}:{name}") };
            let nested = group.emit(group_name, &format!("{group_name}/{}", crate::value::escape_id(&full)), group_name, &full);
            let mut nested = nested;
            nested.set_attr("Name", name);
            element.push(nested);
        }
        element
    }
}

/// A child of a style group, in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleNode {
    Group(StyleGroup),
    Style(Style),
    Other(El),
}

/// EPUB export tagging stored on a style in IDML.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpubExportTagMap {
    pub tag: String,
    pub class: String,
    pub split_document: bool,
}

impl EpubExportTagMap {
    /// Reads an EPUB `StyleExportTagMap`; other export types are not this record.
    pub fn from_el(el: &El) -> Option<Self> {
        let values = Values::of(el);
        (el.local() == "StyleExportTagMap" && values.get("ExportType") == Some("EPUB")).then(|| Self {
            tag: values.get("ExportTag").unwrap_or("").to_owned(),
            class: values.get("ExportClass").unwrap_or("").to_owned(),
            split_document: values.get("SplitDocument") == Some("true"),
        })
    }

    /// Writes the EPUB tag map with its caller-assigned IDML `Self` identifier.
    pub fn to_el(&self, self_id: &str) -> El {
        El::new("StyleExportTagMap")
            .with_attr("Self", self_id)
            .with_attr("ExportType", "EPUB")
            .with_attr("ExportTag", &self.tag)
            .with_attr("ExportClass", &self.class)
            .with_attr("ExportAttributes", "")
            .with_attr("SplitDocument", if self.split_document { "true" } else { "false" })
            .with_attr("EmitCss", "true")
    }
}

/// A style, its values as written: `Name`, `Self`, `Properties/BasedOn`, its attributes; and its
/// other children (an object style's `TextFramePreference`, effect settings, …).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub kind: StyleKind,
    pub values: Values,
    pub rest: Vec<El>,
}

/// Parsed assignments on an IDML object style before application style-reference resolution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectStyleValues {
    pub name: Option<String>,
    pub based_on: Option<String>,
    pub fill_enabled: bool,
    pub fill_color: Option<String>,
    pub fill_tint: Option<f64>,
    pub stroke_enabled: bool,
    pub paragraph_style_enabled: bool,
    pub paragraph_style: Option<String>,
    pub text_frame_general_options: bool,
    pub text_frame_preference: Option<El>,
}

impl ObjectStyleValues {
    /// Reads object-style enable flags and the values controlled by those flags.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let enabled = |flag: &str, attribute: &str| match values.get(flag) {
            Some(value) => value == "true",
            None => values.get(attribute).is_some(),
        };
        Self {
            name: values.name().map(str::to_owned),
            based_on: values.prop("BasedOn"),
            fill_enabled: enabled("EnableFill", "FillColor"),
            fill_color: values.get("FillColor").map(str::to_owned),
            fill_tint: values.num("FillTint"),
            stroke_enabled: enabled("EnableStroke", "StrokeColor"),
            paragraph_style_enabled: enabled("EnableParagraphStyle", "AppliedParagraphStyle"),
            paragraph_style: values.get("AppliedParagraphStyle").map(str::to_owned),
            text_frame_general_options: values.get("EnableTextFrameGeneralOptions") == Some("true"),
            text_frame_preference: el.child(Local("TextFramePreference")).cloned(),
        }
    }
}

impl StyleGroup {
    /// The root group `el`; `None` when `el` is no root style group.
    pub fn from_el(el: &El) -> Option<StyleGroup> {
        let kind = StyleKind::of_root(el.local())?;
        Some(Self::read(kind, true, el))
    }

    fn read(kind: StyleKind, root: bool, el: &El) -> StyleGroup {
        let children = others(el)
            .map(|c| match c.local() {
                n if n == kind.group() => StyleNode::Group(Self::read(kind, false, c)),
                n if n == kind.style() => StyleNode::Style(Style { kind, values: Values::of(c), rest: others(c).map(tidy).collect() }),
                _ => StyleNode::Other(tidy(c)),
            })
            .collect();
        StyleGroup { kind, root, values: Values::of(el), children }
    }

    pub fn to_el(&self) -> El {
        let mut el = self.values.element(if self.root { self.kind.root() } else { self.kind.group() });
        el.children.extend(self.children.iter().map(|c| {
            Node::El(match c {
                StyleNode::Group(g) => g.to_el(),
                StyleNode::Style(s) => s.to_el(),
                StyleNode::Other(e) => e.clone(),
            })
        }));
        el
    }

    /// Every style in this group and the groups in it, in document order, with the names of
    /// the groups it is in (outermost first, this group's own name left out for a root group).
    pub fn styles(&self) -> Vec<(Vec<&str>, &Style)> {
        let mut out = Vec::new();
        let start: Vec<&str> = if self.root { Vec::new() } else { self.values.name().into_iter().collect() };
        // Iterative: nesting is bounded by the XML reader, but there is no need to recurse.
        let mut stack: Vec<(Vec<&str>, &StyleNode)> = self.children.iter().rev().map(|c| (start.clone(), c)).collect();
        while let Some((path, node)) = stack.pop() {
            match node {
                StyleNode::Style(s) => out.push((path, s)),
                StyleNode::Group(g) => {
                    let mut inner = path.clone();
                    inner.extend(g.values.name());
                    stack.extend(g.children.iter().rev().map(|c| (inner.clone(), c)));
                }
                StyleNode::Other(_) => {}
            }
        }
        out
    }
}

impl Style {
    pub fn to_el(&self) -> El {
        let mut el = self.values.element(self.kind.style());
        el.children.extend(self.rest.iter().cloned().map(Node::El));
        el
    }

    /// `Properties/BasedOn`: the `Self` of the style this one is based on (`$ID/[No paragraph
    /// style]` and the like for none).
    pub fn based_on(&self) -> Option<String> {
        self.values.value("BasedOn")
    }

    /// `NextStyle` of a paragraph style.
    pub fn next_style(&self) -> Option<String> {
        self.values.value("NextStyle")
    }

    /// The child element `name` (`TextFramePreference`, `TransparencySetting`, …).
    pub fn other(&self, name: &str) -> Option<&El> {
        self.rest.iter().find(|e| e.local() == name)
    }
}

/// The root style groups among `top`, in order.
pub fn style_groups(top: &[El]) -> Vec<StyleGroup> {
    top.iter().filter_map(StyleGroup::from_el).collect()
}

/// What a swatch is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwatchKind {
    Color,
    Tint,
    Gradient,
    MixedInk,
    MixedInkGroup,
    /// `Swatch`: the None swatch.
    Swatch,
}

impl SwatchKind {
    pub fn of(name: &str) -> Option<SwatchKind> {
        Some(match name {
            "Color" => SwatchKind::Color,
            "Tint" => SwatchKind::Tint,
            "Gradient" => SwatchKind::Gradient,
            "MixedInk" => SwatchKind::MixedInk,
            "MixedInkGroup" => SwatchKind::MixedInkGroup,
            "Swatch" => SwatchKind::Swatch,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            SwatchKind::Color => "Color",
            SwatchKind::Tint => "Tint",
            SwatchKind::Gradient => "Gradient",
            SwatchKind::MixedInk => "MixedInk",
            SwatchKind::MixedInkGroup => "MixedInkGroup",
            SwatchKind::Swatch => "Swatch",
        }
    }
}

/// A swatch (`Color`, `Tint`, `Gradient`, …), its values as written and its other children
/// (a gradient's `GradientStop`s).
#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    pub kind: SwatchKind,
    pub values: Values,
    pub rest: Vec<El>,
}

/// A colour's space, as `Space` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Cmyk,
    Rgb,
    Lab,
    /// `MixedInk`.
    MixedInk,
}

/// A stop of a gradient.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStop {
    /// `StopColor`: the `Self` of a colour.
    pub color: String,
    /// `Location`, 0 to 100.
    pub location: f64,
    /// `Midpoint`, 0 to 100 (50 when absent).
    pub midpoint: f64,
}

/// A `GradientStop` in source order, including optional values as written.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStopValue {
    pub color: Option<String>,
    pub location: Option<f64>,
    pub midpoint: Option<f64>,
}

impl Swatch {
    pub fn to_el(&self) -> El {
        let mut el = self.values.element(self.kind.name());
        el.children.extend(self.rest.iter().cloned().map(Node::El));
        el
    }

    /// `Model` is `Spot`.
    pub fn spot(&self) -> bool {
        self.values.get("Model") == Some("Spot")
    }

    /// `Space`; CMYK when absent or unknown.
    pub fn space(&self) -> ColorSpace {
        match self.values.get("Space") {
            Some("RGB") => ColorSpace::Rgb,
            Some("LAB") => ColorSpace::Lab,
            Some("MixedInk") => ColorSpace::MixedInk,
            _ => ColorSpace::Cmyk,
        }
    }

    /// `ColorValue`: CMYK in percent, RGB 0 to 255, Lab as L a b. Empty when absent or when a
    /// number is not finite.
    pub fn color_value(&self) -> Vec<f64> {
        self.values.get("ColorValue").map(nums).unwrap_or_default()
    }

    /// `BaseColor` of a tint: the `Self` of its colour.
    pub fn base_color(&self) -> Option<&str> {
        self.values.get("BaseColor")
    }

    /// `TintValue` of a tint, in percent.
    pub fn tint_value(&self) -> Option<f64> {
        self.values.num("TintValue")
    }

    /// `Type` of a gradient is `Radial`; linear otherwise.
    pub fn radial(&self) -> bool {
        self.values.get("Type") == Some("Radial")
    }

    /// A gradient's stops in order; a stop without a colour or a finite location is left out.
    pub fn stops(&self) -> Vec<GradientStop> {
        self.rest
            .iter()
            .filter(|e| e.local() == "GradientStop")
            .filter_map(|s| {
                let v = Values::of(s);
                Some(GradientStop {
                    color: v.get("StopColor")?.to_string(),
                    location: v.num("Location")?,
                    midpoint: v.num("Midpoint").unwrap_or(50.0),
                })
            })
            .collect()
    }

    /// Every gradient stop in source order, retaining absent or malformed optional values.
    pub fn stops_in_order(&self) -> Vec<GradientStopValue> {
        self.rest
            .iter()
            .filter(|e| e.local() == "GradientStop")
            .map(|e| {
                let values = Values::of(e);
                GradientStopValue {
                    color: values.get("StopColor").map(str::to_string),
                    location: values.num("Location"),
                    midpoint: values.num("Midpoint"),
                }
            })
            .collect()
    }
}

/// The swatches among `top`, in order.
pub fn swatches(top: &[El]) -> Vec<Swatch> {
    top.iter().filter_map(|e| Some(Swatch { kind: SwatchKind::of(e.local())?, values: Values::of(e), rest: others(e).map(tidy).collect() })).collect()
}

/// What a stroke style is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeStyleKind {
    Dashed,
    Dotted,
    Striped,
}

/// A custom stroke style.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    pub kind: StrokeStyleKind,
    pub values: Values,
    pub rest: Vec<El>,
}

impl StrokeStyle {
    pub fn to_el(&self) -> El {
        let name = match self.kind {
            StrokeStyleKind::Dashed => "DashedStrokeStyle",
            StrokeStyleKind::Dotted => "DottedStrokeStyle",
            StrokeStyleKind::Striped => "StripedStrokeStyle",
        };
        let mut el = self.values.element(name);
        el.children.extend(self.rest.iter().cloned().map(Node::El));
        el
    }

    /// `DashArray` of a dashed style: dash, gap, dash, gap, … in points.
    pub fn dash_array(&self) -> Vec<f64> {
        self.values.get("DashArray").map(nums).unwrap_or_default()
    }

    /// `StripeArray` of a striped style: each stripe's start and end, in percent of the weight.
    pub fn stripe_array(&self) -> Vec<f64> {
        self.values.get("StripeArray").map(nums).unwrap_or_default()
    }
}

/// The custom stroke styles among `top`, in order.
pub fn stroke_styles(top: &[El]) -> Vec<StrokeStyle> {
    top.iter()
        .filter_map(|e| {
            let kind = match e.local() {
                "DashedStrokeStyle" => StrokeStyleKind::Dashed,
                "DottedStrokeStyle" => StrokeStyleKind::Dotted,
                "StripedStrokeStyle" => StrokeStyleKind::Striped,
                _ => return None,
            };
            Some(StrokeStyle { kind, values: Values::of(e), rest: others(e).map(tidy).collect() })
        })
        .collect()
}

/// A colour group (`ColorGroup`): its values and the swatches it lists.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorGroup {
    pub values: Values,
    pub rest: Vec<El>,
}

/// An Ink Manager entry, preserving its source values.
#[derive(Clone, Debug, PartialEq)]
pub struct Ink {
    pub values: Values,
}

/// A neutral colour definition for writing `Graphic.xml`.
///
/// The caller supplies colour-space values as IDML's serialized value list; this type owns the
/// element and attribute grammar so applications do not need to emit `Color` XML themselves.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorDefinition {
    pub self_id: String,
    pub name: String,
    pub model: String,
    pub space: String,
    pub value: String,
    pub override_kind: String,
    pub editable: bool,
    pub removable: bool,
    pub visible: bool,
}

impl ColorDefinition {
    pub fn to_el(&self) -> El {
        El::new("Color")
            .with_attr("Self", &self.self_id)
            .with_attr("Model", &self.model)
            .with_attr("Space", &self.space)
            .with_attr("ColorValue", &self.value)
            .with_attr("ColorOverride", &self.override_kind)
            .with_attr("AlternateSpace", "NoAlternateColor")
            .with_attr("AlternateColorValue", "")
            .with_attr("Name", &self.name)
            .with_attr("ColorEditable", self.editable)
            .with_attr("ColorRemovable", self.removable)
            .with_attr("Visible", self.visible)
            .with_attr("SwatchCreatorID", "7937")
    }
}

/// A neutral `Ink` definition in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct InkDefinition {
    pub self_id: String,
    pub name: String,
    pub alias: Option<String>,
    pub angle: String,
    pub convert_to_process: bool,
    pub frequency: String,
    pub neutral_density: String,
    pub trap_order: usize,
    pub ink_type: String,
}

impl InkDefinition {
    fn to_el(&self) -> El {
        let mut el = El::new("Ink")
            .with_attr("Self", &self.self_id)
            .with_attr("Name", &self.name)
            .with_attr("Angle", &self.angle)
            .with_attr("ConvertToProcess", self.convert_to_process)
            .with_attr("Frequency", &self.frequency)
            .with_attr("NeutralDensity", &self.neutral_density)
            .with_attr("PrintInk", "true")
            .with_attr("TrapOrder", self.trap_order)
            .with_attr("InkType", &self.ink_type);
        if let Some(alias) = &self.alias {
            el.set_attr("AliasInkName", alias);
        }
        el
    }
}

/// A neutral `Tint` definition in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct TintDefinition {
    pub self_id: String,
    pub value: String,
    pub base_color: String,
    pub name: String,
}

impl TintDefinition {
    fn to_el(&self) -> El {
        El::new("Tint")
            .with_attr("Self", &self.self_id)
            .with_attr("TintValue", &self.value)
            .with_attr("BaseColor", &self.base_color)
            .with_attr("Name", &self.name)
            .with_attr("ColorOverride", "Normal")
            .with_attr("AlternateSpace", "NoAlternateColor")
            .with_attr("AlternateColorValue", "")
            .with_attr("ColorEditable", "true")
            .with_attr("ColorRemovable", "true")
            .with_attr("Visible", "true")
            .with_attr("SwatchCreatorID", "7937")
    }
}

/// A stop in a neutral `Gradient` definition.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStopDefinition {
    pub self_id: String,
    pub color_id: String,
    pub location: String,
    pub midpoint: Option<String>,
}

/// A neutral `Gradient` definition in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientDefinition {
    pub self_id: String,
    pub name: String,
    pub radial: bool,
    pub stops: Vec<GradientStopDefinition>,
}

impl GradientDefinition {
    fn to_el(&self) -> El {
        let mut el = El::new("Gradient")
            .with_attr("Self", &self.self_id)
            .with_attr("Type", if self.radial { "Radial" } else { "Linear" })
            .with_attr("Name", &self.name)
            .with_attr("ColorEditable", "true")
            .with_attr("ColorRemovable", "true")
            .with_attr("Visible", "true")
            .with_attr("SwatchCreatorID", "7937");
        for stop in &self.stops {
            let mut child =
                El::new("GradientStop").with_attr("Self", &stop.self_id).with_attr("StopColor", &stop.color_id).with_attr("Location", &stop.location);
            if let Some(midpoint) = &stop.midpoint {
                child.set_attr("Midpoint", midpoint);
            }
            el.push(child);
        }
        el
    }
}

/// A member reference in a neutral `ColorGroup` definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorGroupMember {
    pub self_id: String,
    pub swatch_id: String,
}

/// A neutral `ColorGroup` definition in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorGroupDefinition {
    pub self_id: String,
    pub name: String,
    pub members: Vec<ColorGroupMember>,
}

impl ColorGroupDefinition {
    fn to_el(&self) -> El {
        let mut el = El::new("ColorGroup").with_attr("Self", &self.self_id).with_attr("Name", &self.name).with_attr("IsRootColorGroup", "false");
        for member in &self.members {
            el.push(El::new("ColorGroupSwatch").with_attr("Self", &member.self_id).with_attr("SwatchItemRef", &member.swatch_id));
        }
        el
    }
}

/// A custom stroke style in `Graphic.xml`.
#[derive(Clone, Debug, PartialEq)]
pub enum StrokeDefinition {
    Striped { self_id: String, name: String, array: String },
    Dashed { self_id: String, name: String, array: String },
    Dotted { self_id: String, name: String, array: String },
    Named { self_id: String, name: String },
}

impl StrokeDefinition {
    fn to_el(&self) -> El {
        match self {
            Self::Striped { self_id, name, array } => {
                El::new("StripedStrokeStyle").with_attr("StripeArray", array).with_attr("Self", self_id).with_attr("Name", name)
            }
            Self::Dashed { self_id, name, array } => {
                El::new("DashedStrokeStyle").with_attr("DashArray", array).with_attr("Self", self_id).with_attr("Name", name)
            }
            Self::Dotted { self_id, name, array } => {
                El::new("DottedStrokeStyle").with_attr("DotArray", array).with_attr("Self", self_id).with_attr("Name", name)
            }
            Self::Named { self_id, name } => El::new("StrokeStyle").with_attr("Self", self_id).with_attr("Name", name),
        }
    }
}

/// Semantic values needed to write the `Graphic.xml` part. This type owns element names,
/// attributes, child ordering and the required None swatch; callers supply their mapped values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphicPart {
    pub colors: Vec<ColorDefinition>,
    pub inks: Vec<InkDefinition>,
    pub tints: Vec<TintDefinition>,
    pub gradients: Vec<GradientDefinition>,
    pub color_groups: Vec<ColorGroupDefinition>,
    pub stroke_styles: Vec<StrokeDefinition>,
}

impl GraphicPart {
    pub fn to_el(&self) -> El {
        let mut root = crate::part_root("Graphic");
        for color in &self.colors {
            root.push(color.to_el());
        }
        for ink in &self.inks {
            root.push(ink.to_el());
        }
        for tint in &self.tints {
            root.push(tint.to_el());
        }
        root.push(
            El::new("Swatch")
                .with_attr("Self", "Swatch/None")
                .with_attr("Name", "None")
                .with_attr("ColorEditable", "false")
                .with_attr("ColorRemovable", "false")
                .with_attr("Visible", "true")
                .with_attr("SwatchCreatorID", "7937"),
        );
        for gradient in &self.gradients {
            root.push(gradient.to_el());
        }
        for group in &self.color_groups {
            root.push(group.to_el());
        }
        for style in &self.stroke_styles {
            root.push(style.to_el());
        }
        root
    }
}

/// A composite-font entry, including the applied font family stored in `Properties`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositeFontEntryDefinition {
    pub self_id: String,
    pub name: String,
    pub characters: String,
    pub font_style: String,
    pub relative_size: String,
    pub horizontal_scale: String,
    pub vertical_scale: String,
    pub baseline_shift: String,
    pub scale_option: bool,
    pub applied_font: String,
}

/// A composite font in `Fonts.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositeFontDefinition {
    pub self_id: String,
    pub name: String,
    pub entries: Vec<CompositeFontEntryDefinition>,
}

/// A font face in a family in `Fonts.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct FontDefinition {
    pub self_id: String,
    pub family: String,
    pub name: String,
    pub postscript_name: String,
    pub style: String,
}

/// A family of font faces in `Fonts.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFamilyDefinition {
    pub self_id: String,
    pub name: String,
    pub fonts: Vec<FontDefinition>,
}

/// Neutral data for writing the complete `Fonts.xml` part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontsPart {
    pub composites: Vec<CompositeFontDefinition>,
    pub families: Vec<FontFamilyDefinition>,
}

impl FontsPart {
    pub fn to_el(&self) -> El {
        let mut root = crate::part_root("Fonts");
        for composite in &self.composites {
            let mut element = El::new("CompositeFont").with_attr("Self", &composite.self_id).with_attr("Name", &composite.name);
            for entry in &composite.entries {
                let mut node = El::new("CompositeFontEntry")
                    .with_attr("Self", &entry.self_id)
                    .with_attr("Name", &entry.name)
                    .with_attr("CustomCharacters", &entry.characters)
                    .with_attr("FontStyle", &entry.font_style)
                    .with_attr("RelativeSize", &entry.relative_size)
                    .with_attr("HorizontalScale", &entry.horizontal_scale)
                    .with_attr("VerticalScale", &entry.vertical_scale)
                    .with_attr("BaselineShift", &entry.baseline_shift)
                    .with_attr("ScaleOption", entry.scale_option);
                let mut properties = El::new("Properties");
                properties.push(crate::prop("AppliedFont", "string", &entry.applied_font));
                node.push(properties);
                element.push(node);
            }
            root.push(element);
        }
        for family in &self.families {
            let mut element = El::new("FontFamily").with_attr("Self", &family.self_id).with_attr("Name", &family.name);
            for font in &family.fonts {
                element.push(
                    El::new("Font")
                        .with_attr("Self", &font.self_id)
                        .with_attr("FontFamily", &font.family)
                        .with_attr("Name", &font.name)
                        .with_attr("PostScriptName", &font.postscript_name)
                        .with_attr("Status", "NotAvailable")
                        .with_attr("FontStyleName", &font.style)
                        .with_attr("FontType", "OpenTypeTT")
                        .with_attr("WritingScript", "0")
                        .with_attr("FullName", &font.name)
                        .with_attr("FullNameNative", &font.name)
                        .with_attr("FontStyleNameNative", &font.style)
                        .with_attr("PlatformName", "$ID/")
                        .with_attr("Version", "$ID/")
                        .with_attr("TypekitID", "$ID/"),
                );
            }
            root.push(element);
        }
        root
    }
}

impl Ink {
    pub fn name(&self) -> Option<&str> {
        self.values.name()
    }

    pub fn convert_to_process(&self) -> bool {
        self.values.get("ConvertToProcess") == Some("true")
    }

    pub fn alias(&self) -> Option<&str> {
        self.values.get("AliasInkName")
    }
}

/// Ink Manager entries among the document's top-level elements.
pub fn inks(top: &[El]) -> Vec<Ink> {
    top.iter().filter(|el| el.local() == "Ink").map(|el| Ink { values: Values::of(el) }).collect()
}

impl ColorGroup {
    pub fn to_el(&self) -> El {
        let mut el = self.values.element("ColorGroup");
        el.children.extend(self.rest.iter().cloned().map(Node::El));
        el
    }

    /// `IsRootColorGroup`.
    pub fn is_root(&self) -> bool {
        self.values.boolean("IsRootColorGroup") == Some(true)
    }

    /// The `Self`s of the swatches the group lists (`ColorGroupSwatch`'s `SwatchItemRef`), in
    /// order.
    pub fn members(&self) -> Vec<&str> {
        self.rest.iter().filter(|e| e.local() == "ColorGroupSwatch").filter_map(|e| e.attr("SwatchItemRef")).collect()
    }
}

/// The colour groups among `top`, in order.
pub fn color_groups(top: &[El]) -> Vec<ColorGroup> {
    top.iter().filter(|e| e.local() == "ColorGroup").map(|e| ColorGroup { values: Values::of(e), rest: others(e).map(tidy).collect() }).collect()
}

/// The values of every element named `name` among `top`, in order: `Ink`, `Layer`, `Section`,
/// `Condition`, `NumberingList`, ….
pub fn named(top: &[El], name: &str) -> Vec<Values> {
    top.iter().filter(|e| e.local() == name).map(Values::of).collect()
}

/// The first element named `name` among `top` (a preference: `DocumentPreference`,
/// `MarginPreference`, `TextFramePreference`, `GridPreference`, …), its values.
pub fn preference(top: &[El], name: &str) -> Option<Values> {
    top.iter().find(|e| e.local() == name).map(Values::of)
}
