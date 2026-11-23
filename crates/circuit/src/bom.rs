//! Grouping parts into parts-list rows.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::Project;
use crate::natural::natural_cmp;

/// How parts are grouped into rows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BomOptions {
    /// Fields that must also match for two parts to share a row, besides value and footprint.
    pub group_by: Vec<String>,
    /// List parts marked as not belonging in the parts list too.
    pub include_excluded: bool,
}

/// Parts with the same value, footprint and grouping fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BomRow {
    pub quantity: usize,
    /// In designator order (`R2` before `R10`).
    pub designators: Vec<String>,
    pub value: String,
    pub footprint: String,
    /// The grouping fields' values, empty where a part does not have the field.
    pub fields: BTreeMap<String, String>,
}

/// The parts list, in the order of each row's first designator.
pub fn bom(project: &Project, options: &BomOptions) -> Vec<BomRow> {
    let mut rows: BTreeMap<(String, String, Vec<String>), BomRow> = BTreeMap::new();
    for part in project.parts.iter().filter(|p| p.in_bom || options.include_excluded) {
        let values: Vec<String> = options.group_by.iter().map(|f| part.fields.get(f).cloned().unwrap_or_default()).collect();
        let key = (part.value.clone(), part.footprint.clone(), values.clone());
        let row = rows.entry(key).or_insert_with(|| BomRow {
            quantity: 0,
            designators: Vec::new(),
            value: part.value.clone(),
            footprint: part.footprint.clone(),
            fields: options.group_by.iter().cloned().zip(values).collect(),
        });
        row.quantity += 1;
        row.designators.push(part.designator.clone());
    }
    let mut out: Vec<BomRow> = rows.into_values().collect();
    for row in &mut out {
        row.designators.sort_by(|a, b| natural_cmp(a, b));
    }
    out.sort_by(|a, b| {
        let first = |r: &BomRow| r.designators.first().cloned().unwrap_or_default();
        natural_cmp(&first(a), &first(b))
    });
    out
}

/// The parts list as CSV: `quantity,designators,value,footprint` and the grouping fields,
/// with a header row. Designators are joined with `, ` in one field.
pub fn bom_csv(rows: &[BomRow], options: &BomOptions) -> String {
    let mut out = String::new();
    let header: Vec<&str> =
        ["quantity", "designators", "value", "footprint"].into_iter().chain(options.group_by.iter().map(String::as_str)).collect();
    push_record(&mut out, &header);
    for row in rows {
        let quantity = row.quantity.to_string();
        let designators = row.designators.join(", ");
        let mut record: Vec<&str> = vec![&quantity, &designators, &row.value, &row.footprint];
        record.extend(options.group_by.iter().map(|f| row.fields.get(f).map_or("", String::as_str)));
        push_record(&mut out, &record);
    }
    out
}

fn push_record(out: &mut String, fields: &[&str]) {
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        craft_csv::push_field(out, field, ',');
    }
    out.push('\n');
}
