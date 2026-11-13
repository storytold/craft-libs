//! Typed views of IDML's reusable text-frame preference grammar.

use craft_xml::El;

use super::Values;

/// Parsed values from IDML `DocumentPreference` elements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentPreferences {
    pub overprint_black: Option<bool>,
    pub page_width: Option<f64>,
    pub page_height: Option<f64>,
    pub facing_pages: Option<bool>,
    pub page_binding: Option<String>,
    pub bleed: [Option<f64>; 4],
    pub slug: [Option<f64>; 4],
    pub intent: Option<String>,
    pub primary_text_frame: Option<bool>,
    pub column_guides_locked: Option<bool>,
}

impl DocumentPreferences {
    /// Reads the document-level page, bleed, slug, and intent values.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            overprint_black: values.get("OverprintBlack").map(|value| value != "false"),
            page_width: values.num("PageWidth"),
            page_height: values.num("PageHeight"),
            facing_pages: values.boolean("FacingPages"),
            page_binding: values.prop("PageBinding"),
            bleed: [
                values.num("DocumentBleedTopOffset"),
                values.num("DocumentBleedBottomOffset"),
                values.num("DocumentBleedInsideOrLeftOffset"),
                values.num("DocumentBleedOutsideOrRightOffset"),
            ],
            slug: [
                values.num("SlugTopOffset"),
                values.num("SlugBottomOffset"),
                values.num("SlugInsideOrLeftOffset"),
                values.num("SlugRightOrOutsideOffset"),
            ],
            intent: values.prop("Intent"),
            primary_text_frame: values.boolean("CreatePrimaryTextFrame"),
            column_guides_locked: values.boolean("ColumnGuideLocked"),
        }
    }

    /// Full canonical attributes for an exported IDML `DocumentPreference`.
    pub fn to_export_attrs(&self) -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        for (key, value) in [("PageHeight", self.page_height), ("PageWidth", self.page_width)] {
            if let Some(value) = value {
                attrs.push((key.into(), crate::value::num(value)));
            }
        }
        attrs.push(("PagesPerDocument".into(), "1".into()));
        for (key, value) in [("FacingPages", self.facing_pages)] {
            if let Some(value) = value {
                attrs.push((key.into(), if value { "true" } else { "false" }.into()));
            }
        }
        for (key, value) in [
            ("DocumentBleedTopOffset", self.bleed[0]),
            ("DocumentBleedBottomOffset", self.bleed[1]),
            ("DocumentBleedInsideOrLeftOffset", self.bleed[2]),
            ("DocumentBleedOutsideOrRightOffset", self.bleed[3]),
        ] {
            if let Some(value) = value {
                attrs.push((key.into(), crate::value::num(value)));
            }
        }
        if self.bleed.iter().all(Option::is_some) {
            attrs.push((
                "DocumentBleedUniformSize".into(),
                bool_string(self.bleed.iter().flatten().all(|value| *value == self.bleed[0].unwrap_or_default())),
            ));
        }
        for (key, value) in [
            ("SlugTopOffset", self.slug[0]),
            ("SlugBottomOffset", self.slug[1]),
            ("SlugInsideOrLeftOffset", self.slug[2]),
            ("SlugRightOrOutsideOffset", self.slug[3]),
        ] {
            if let Some(value) = value {
                attrs.push((key.into(), crate::value::num(value)));
            }
        }
        if self.slug.iter().all(Option::is_some) {
            attrs.push((
                "DocumentSlugUniformSize".into(),
                bool_string(self.slug.iter().flatten().all(|value| *value == self.slug[0].unwrap_or_default())),
            ));
        }
        attrs.push(("PreserveLayoutWhenShuffling".into(), "true".into()));
        attrs.push(("AllowPageShuffle".into(), "true".into()));
        if let Some(value) = self.overprint_black {
            attrs.push(("OverprintBlack".into(), bool_string(value)));
        }
        if let Some(value) = &self.page_binding {
            attrs.push(("PageBinding".into(), value.clone()));
        }
        attrs.push(("ColumnDirection".into(), "Horizontal".into()));
        for (key, value) in [("Intent", &self.intent)] {
            if let Some(value) = value {
                attrs.push((key.into(), value.clone()));
            }
        }
        if let Some(value) = self.primary_text_frame {
            attrs.push(("CreatePrimaryTextFrame".into(), bool_string(value)));
        }
        if let Some(value) = self.column_guides_locked {
            attrs.push(("ColumnGuideLocked".into(), bool_string(value)));
        }
        if let Some(value) = self.primary_text_frame {
            attrs.push(("MasterTextFrame".into(), bool_string(value)));
        }
        attrs.push(("SnippetImportUsesOriginalLocation".into(), "false".into()));
        attrs
    }
}

/// Parsed document and baseline-grid preference values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridPreferences {
    pub baseline_start: Option<f64>,
    pub baseline_division: Option<f64>,
    pub baseline_view_threshold: Option<f64>,
    pub baseline_relative_option: Option<String>,
    pub horizontal_division: Option<f64>,
    pub vertical_division: Option<f64>,
    pub horizontal_subdivision: Option<f64>,
    pub vertical_subdivision: Option<f64>,
    pub grids_in_back: Option<bool>,
}

impl GridPreferences {
    /// Reads baseline and document-grid values.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            baseline_start: values.num("BaselineStart"),
            baseline_division: values.num("BaselineDivision"),
            baseline_view_threshold: values.num("BaselineViewThreshold"),
            baseline_relative_option: values.prop("BaselineGridRelativeOption"),
            horizontal_division: values.num("HorizontalGridlineDivision"),
            vertical_division: values.num("VerticalGridlineDivision"),
            horizontal_subdivision: values.num("HorizontalGridSubdivision"),
            vertical_subdivision: values.num("VerticalGridSubdivision"),
            grids_in_back: values.boolean("GridsInBack"),
        }
    }

    /// IDML attributes represented by this grid preference record.
    pub fn to_attrs(&self) -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        for (key, value) in [
            ("HorizontalGridlineDivision", self.horizontal_division),
            ("VerticalGridlineDivision", self.vertical_division),
            ("HorizontalGridSubdivision", self.horizontal_subdivision),
            ("VerticalGridSubdivision", self.vertical_subdivision),
        ] {
            if let Some(value) = value {
                attrs.push((key.into(), crate::value::num(value)));
            }
        }
        if let Some(value) = self.grids_in_back {
            attrs.push(("GridsInBack".into(), if value { "true" } else { "false" }.into()));
        }
        for (key, value) in [
            ("BaselineStart", self.baseline_start),
            ("BaselineDivision", self.baseline_division),
            ("BaselineViewThreshold", self.baseline_view_threshold),
        ] {
            if let Some(value) = value {
                attrs.push((key.into(), crate::value::num(value)));
            }
        }
        if let Some(value) = self.baseline_relative_option.as_deref() {
            attrs.push(("BaselineGridRelativeOption".into(), value.into()));
        }
        attrs
    }
}

fn bool_string(value: bool) -> String {
    if value { "true" } else { "false" }.into()
}

/// Parsed `TextFramePreference` values in nearest-to-farthest precedence order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextFramePreferences {
    pub column_count: Option<f64>,
    pub column_gutter: Option<f64>,
    pub fixed_column_width: Option<f64>,
    pub fixed_width: bool,
    pub flexible_width: bool,
    pub balance_columns: bool,
    pub inset: Option<[f64; 4]>,
    pub vertical_justification: Option<String>,
    pub vertical_threshold: Option<f64>,
    pub first_baseline: Option<String>,
    pub minimum_first_baseline: Option<f64>,
    pub ignore_wrap: bool,
    pub auto_sizing_type: Option<String>,
    pub auto_sizing_reference: Option<String>,
    pub use_minimum_height: bool,
    pub minimum_height: Option<f64>,
    pub use_minimum_width: bool,
    pub minimum_width: Option<f64>,
    pub no_line_breaks: bool,
    pub column_rule_override: bool,
    pub column_rule_width: Option<f64>,
    pub column_rule_tint: Option<f64>,
    pub column_rule_offset: Option<f64>,
    pub column_rule_top_inset: Option<f64>,
    pub column_rule_bottom_inset: Option<f64>,
}

impl TextFramePreferences {
    /// Reads preference levels nearest first. A malformed explicit value still masks an inherited
    /// one, matching IDML's item-over-object-style precedence.
    pub fn from_elements(elements: &[&El]) -> Self {
        let prop = |key: &str| elements.iter().find_map(|el| Values::of(el).prop(key));
        let num = |key: &str| prop(key).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
        let boolean = |key: &str| matches!(prop(key).as_deref().map(str::trim), Some("true"));
        Self {
            column_count: num("TextColumnCount"),
            column_gutter: num("TextColumnGutter"),
            fixed_column_width: num("TextColumnFixedWidth"),
            fixed_width: boolean("UseFixedColumnWidth"),
            flexible_width: boolean("UseFlexibleColumnWidth"),
            balance_columns: boolean("VerticalBalanceColumns"),
            inset: inset(elements),
            vertical_justification: prop("VerticalJustification"),
            vertical_threshold: num("VerticalThreshold"),
            first_baseline: prop("FirstBaselineOffset"),
            minimum_first_baseline: num("MinimumFirstBaselineOffset"),
            ignore_wrap: boolean("IgnoreWrap"),
            auto_sizing_type: prop("AutoSizingType"),
            auto_sizing_reference: prop("AutoSizingReferencePoint"),
            use_minimum_height: boolean("UseMinimumHeightForAutoSizing"),
            minimum_height: num("MinimumHeightForAutoSizing"),
            use_minimum_width: boolean("UseMinimumWidthForAutoSizing"),
            minimum_width: num("MinimumWidthForAutoSizing"),
            no_line_breaks: boolean("UseNoLineBreaksForAutoSizing"),
            column_rule_override: boolean("ColumnRuleOverride"),
            column_rule_width: num("ColumnRuleStrokeWidth"),
            column_rule_tint: num("ColumnRuleStrokeTint"),
            column_rule_offset: num("ColumnRuleOffset"),
            column_rule_top_inset: num("ColumnRuleTopInset"),
            column_rule_bottom_inset: num("ColumnRuleBottomInset"),
        }
    }

    /// Writes one IDML `TextFramePreference`, including its inset list and column-rule
    /// properties. Enumeration strings are supplied by the caller because they are model policy.
    pub fn to_el(&self, column_rule_color: &str) -> El {
        use crate::value::num;

        let mut inset = El::new("InsetSpacing").with_attr("type", "list");
        for value in self.inset.unwrap_or([0.0; 4]) {
            inset.push(El::new("ListItem").with_attr("type", "unit").with_text(num(value)));
        }
        let el = El::new("TextFramePreference")
            .with_attr("TextColumnCount", num(self.column_count.unwrap_or(1.0).max(1.0)))
            .with_attr("TextColumnGutter", num(self.column_gutter.unwrap_or(0.0)))
            .with_attr("UseFixedColumnWidth", bool_text(self.fixed_width))
            .with_attr("UseFlexibleColumnWidth", bool_text(self.flexible_width))
            .with_attr("VerticalBalanceColumns", bool_text(self.balance_columns))
            .with_attr("FirstBaselineOffset", self.first_baseline.as_deref().unwrap_or("Ascent"))
            .with_attr("MinimumFirstBaselineOffset", num(self.minimum_first_baseline.unwrap_or(0.0)))
            .with_attr("VerticalJustification", self.vertical_justification.as_deref().unwrap_or("TopAlign"))
            .with_attr("VerticalThreshold", num(self.vertical_threshold.unwrap_or(0.0)))
            .with_attr("IgnoreWrap", bool_text(self.ignore_wrap))
            .with_attr("AutoSizingType", self.auto_sizing_type.as_deref().unwrap_or("Off"))
            .with_attr("AutoSizingReferencePoint", self.auto_sizing_reference.as_deref().unwrap_or("CenterPoint"))
            .with_attr("UseMinimumHeightForAutoSizing", bool_text(self.use_minimum_height))
            .with_attr("MinimumHeightForAutoSizing", num(self.minimum_height.unwrap_or(0.0)))
            .with_attr("UseMinimumWidthForAutoSizing", bool_text(self.use_minimum_width))
            .with_attr("MinimumWidthForAutoSizing", num(self.minimum_width.unwrap_or(0.0)))
            .with_attr("UseNoLineBreaksForAutoSizing", bool_text(self.no_line_breaks));
        let el = if let Some(width) = self.fixed_column_width.filter(|width| *width > 0.0) {
            el.with_attr("TextColumnFixedWidth", num(width))
        } else {
            el
        };
        let el = el
            .with_attr("ColumnRuleOverride", bool_text(self.column_rule_override))
            .with_attr("ColumnRuleStrokeWidth", num(self.column_rule_width.unwrap_or(0.0)))
            .with_attr("ColumnRuleStrokeColor", column_rule_color)
            .with_attr("ColumnRuleStrokeTint", num(self.column_rule_tint.unwrap_or(0.0)))
            .with_attr("ColumnRuleOffset", num(self.column_rule_offset.unwrap_or(0.0)))
            .with_attr("ColumnRuleTopInset", num(self.column_rule_top_inset.unwrap_or(0.0)))
            .with_attr("ColumnRuleBottomInset", num(self.column_rule_bottom_inset.unwrap_or(0.0)))
            .with_attr("ColumnRuleInsetChainOverride", bool_text(self.column_rule_top_inset == self.column_rule_bottom_inset));
        el.with_child(El::new("Properties").with_child(inset))
    }
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn inset(elements: &[&El]) -> Option<[f64; 4]> {
    let element = elements.iter().find(|el| Values::of(el).prop("InsetSpacing").is_some())?;
    let values = Values::of(element);
    let number = |s: &str| s.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let Some(property) = values.prop_el("InsetSpacing") else {
        return values.get("InsetSpacing").and_then(number).map(|v| [v; 4]);
    };
    let items: Vec<_> = property.elements().collect();
    if property.attr("type") == Some("list") || items.iter().any(|item| item.local() == "ListItem") {
        if items.len() != 4 {
            return None;
        }
        let mut result = [0.0; 4];
        for (slot, item) in result.iter_mut().zip(items) {
            if item.local() != "ListItem" || item.elements().next().is_some() {
                return None;
            }
            *slot = number(&item.deep_text())?;
        }
        Some(result)
    } else if items.is_empty() {
        number(&property.deep_text()).map(|v| [v; 4])
    } else {
        None
    }
}
