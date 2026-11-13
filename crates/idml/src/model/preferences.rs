//! Typed views of IDML's reusable text-frame preference grammar.

use craft_xml::El;

use super::Values;

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
