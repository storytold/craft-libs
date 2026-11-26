// SPDX-License-Identifier: MIT OR Apache-2.0
//! Each table in `tests/rules/` defines the inputs and expected outputs for one rule of the compiled graph format.
//! Run the table checks with `cargo test -p craft-material-graph --lib rule_table_`.

use crate::lcg_rand::{self, Lcg};
use crate::tex::Tex;

/// One table: the column names and the rows, all as text.
pub(crate) struct Table {
    pub name: &'static str,
    columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// Reads a table: `#` lines are comments, the first other line names the columns, the rest are rows; fields are
    /// separated by tabs.
    pub fn parse(name: &'static str, text: &str) -> Table {
        let mut lines = text.lines().map(str::trim_end).filter(|l| !l.is_empty() && !l.starts_with('#'));
        let columns: Vec<String> = lines.next().unwrap_or_default().split('\t').map(str::to_string).collect();
        let rows: Vec<Vec<String>> = lines.map(|l| l.split('\t').map(str::to_string).collect()).collect();
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(r.len(), columns.len(), "{name}: row {} has {} fields, the header {}", i + 1, r.len(), columns.len());
        }
        assert!(!rows.is_empty(), "{name}: no rows");
        Table { name, columns, rows }
    }

    fn at(&self, column: &str) -> usize {
        self.columns.iter().position(|c| c == column).unwrap_or_else(|| panic!("{}: no column {column}", self.name))
    }

    /// The text in `column` of `row`.
    pub fn text<'a>(&self, row: &'a [String], column: &str) -> &'a str {
        row.get(self.at(column)).map_or("", String::as_str)
    }

    /// The number in `column` of `row` (`NaN` and `inf` allowed).
    pub fn num(&self, row: &[String], column: &str) -> f64 {
        let t = self.text(row, column);
        t.parse().unwrap_or_else(|_| panic!("{}: {column} = {t:?} is not a number", self.name))
    }

    /// The number in `column` read as a single float directly (no rounding through a double).
    pub fn f32(&self, row: &[String], column: &str) -> f32 {
        let t = self.text(row, column);
        t.parse().unwrap_or_else(|_| panic!("{}: {column} = {t:?} is not a number", self.name))
    }

    /// A whole number in `column`, decimal or `0x` hex.
    pub fn int(&self, row: &[String], column: &str) -> u64 {
        let t = self.text(row, column);
        let parsed = match t.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16),
            None => t.parse(),
        };
        parsed.unwrap_or_else(|_| panic!("{}: {column} = {t:?} is not a whole number", self.name))
    }

    /// A space-separated list of whole numbers in `column`.
    pub fn ints(&self, row: &[String], column: &str) -> Vec<u64> {
        self.text(row, column).split(' ').map(|t| t.parse().unwrap_or_else(|_| panic!("{}: {column} has {t:?}", self.name))).collect()
    }
}

/// The generator a table names, as ours has it.
fn generator(name: &str) -> Lcg {
    let site = |k: usize| lcg_rand::FX_SITES.get(k).copied().unwrap_or_else(|| panic!("no fx site {k}"));
    match name {
        "function" => lcg_rand::FUNCTION,
        "parameter" => lcg_rand::PARAMETER,
        "fx_luminance" => site(0),
        "fx_branch_offset" => site(1),
        "fx_pattern_offset" => site(2),
        "fx_size" => site(3),
        "fx_rotation" => site(4),
        "fx_variation" => site(5),
        "fx_switch" => site(6),
        "fx_iterate_count" => site(7),
        "fx_image_index" => site(8),
        "iteration" => lcg_rand::ITERATION,
        other => {
            let depth: usize = other.strip_prefix("quadrant_depth_").and_then(|d| d.parse().ok()).unwrap_or_else(|| panic!("no generator {other}"));
            lcg_rand::QUADRANT.get(depth - 1).copied().unwrap_or_else(|| panic!("no quadrant depth {depth}"))
        }
    }
}

#[test]
fn rule_table_node_hold() {
    let t = Table::parse("node_hold", include_str!("../tests/rules/node_hold.tsv"));
    for row in &t.rows {
        let v = t.f32(row, "value");
        let level_8 = (crate::ops::compiled_8bit(v) * 255.0).round() as u64;
        let level_16 = (crate::ops::compiled_16bit(v) * 65535.0).round() as u64;
        let half = crate::ops::compiled_half(v);
        assert_eq!(level_8, t.int(row, "level_8"), "node_hold {row:?}: 8 bits");
        assert_eq!(level_16, t.int(row, "level_16"), "node_hold {row:?}: 16 bits");
        assert_eq!(half.to_bits(), t.f32(row, "half").to_bits(), "node_hold {row:?}: half {half}");
    }
}

#[test]
fn rule_table_bilinear_read() {
    let t = Table::parse("bilinear_read", include_str!("../tests/rules/bilinear_read.tsv"));
    for row in &t.rows {
        let (a, b) = (t.int(row, "a_16") as f32 / 65535.0, t.int(row, "b_16") as f32 / 65535.0);
        let image = Tex { w: 2, h: 1, px: vec![[a, a, a, 1.0], [b, b, b, 1.0]] };
        // the read at a position `fraction` of the way from texel 0's centre to texel 1's
        let u = (0.5 + t.f32(row, "fraction")) / 2.0;
        let mut read = Tex { w: 1, h: 1, px: vec![crate::atomic::sample_stepped(&image, u, 0.5)] };
        crate::ops::round_to_16bit_read(&mut read);
        let got = (read.px.first().map_or(0.0, |p| p[0]) * 65535.0).round() as u64;
        assert_eq!(got, t.int(row, "ours_16"), "bilinear_read {row:?}");
        // the integer rule, and where ours differs from it: only at an exact tie, which ours, in single precision, rounds down
        let w = t.int(row, "weight_256");
        assert_eq!(w, (f64::from(t.f32(row, "fraction")) * 256.0).round() as u64, "bilinear_read {row:?}: weight");
        let exact = t.int(row, "a_16") * (256 - w) + t.int(row, "b_16") * w;
        assert_eq!((exact + 128) >> 8, t.int(row, "rule_16"), "bilinear_read {row:?}: rule");
        assert!(got == t.int(row, "rule_16") || (exact % 256 == 128 && got + 1 == t.int(row, "rule_16")), "bilinear_read {row:?}");
    }
}

#[test]
fn rule_table_box_blur_plan() {
    let t = Table::parse("box_blur_plan", include_str!("../tests/rules/box_blur_plan.tsv"));
    for row in &t.rows {
        let (format, steps) = match t.text(row, "format") {
            "8" => (crate::measured::COMPILED_8_BIT, 255.0),
            "16" => (crate::measured::COMPILED_16_BIT, 65535.0),
            other => panic!("box_blur_plan: no format {other}"),
        };
        let input = t.ints(row, "input");
        let image = Tex { w: input.len(), h: 1, px: input.iter().map(|&k| [k as f32 / steps; 4]).collect() };
        let mut out = crate::atomic::compiled_box_axis(&image, t.f32(row, "radius"), true, format);
        crate::ops::apply_format(&mut out, format);
        let got: Vec<u64> = out.px.iter().map(|p| (p[0] * steps).round() as u64).collect();
        assert_eq!(got, t.ints(row, "output"), "box_blur_plan: format {} radius {}", t.text(row, "format"), t.text(row, "radius"));
    }
}

#[test]
fn rule_table_lcg_draws() {
    let t = Table::parse("lcg_draws", include_str!("../tests/rules/lcg_draws.tsv"));
    for row in &t.rows {
        let lcg = (t.int(row, "a") as u32, t.int(row, "c") as u32);
        assert_eq!(generator(t.text(row, "generator")), lcg, "lcg_draws {row:?}: ours has another generator");
        let mut state = t.int(row, "start") as u32;
        for _ in 0..t.int(row, "draw") {
            state = lcg_rand::step(lcg, state);
        }
        assert_eq!(state, t.int(row, "state") as u32, "lcg_draws {row:?}");
        assert_eq!(lcg_rand::output(state), t.num(row, "value"), "lcg_draws {row:?}");
    }
}

#[test]
fn rule_table_pixel_state() {
    let t = Table::parse("pixel_state", include_str!("../tests/rules/pixel_state.tsv"));
    for row in &t.rows {
        let (x, y, w, h) = (t.int(row, "x") as f32, t.int(row, "y") as f32, t.int(row, "width") as f32, t.int(row, "height") as f32);
        let state = lcg_rand::pixel_state(t.int(row, "seed") as u32, (x + 0.5) / w, (y + 0.5) / h);
        assert_eq!(state, t.int(row, "state") as u32, "pixel_state {row:?}");
        assert_eq!(lcg_rand::output(lcg_rand::step(lcg_rand::FUNCTION, state)), t.num(row, "first_value"), "pixel_state {row:?}");
    }
}

#[test]
fn rule_table_fx_child_state() {
    let t = Table::parse("fx_child_state", include_str!("../tests/rules/fx_child_state.tsv"));
    for row in &t.rows {
        let got = lcg_rand::child_state(t.int(row, "parent") as u32, t.int(row, "child") as usize, t.int(row, "depth") as usize);
        assert_eq!(got, t.int(row, "state") as u32, "fx_child_state {row:?}");
    }
}

#[test]
fn rule_table_pattern_shape() {
    let t = Table::parse("pattern_shape", include_str!("../tests/rules/pattern_shape.tsv"));
    for row in &t.rows {
        let got = crate::patterns::shape(t.int(row, "pattern") as i32, t.f32(row, "x"), t.f32(row, "y"), t.f32(row, "variation"));
        match t.text(row, "value") {
            "none" => assert_eq!(got, None, "pattern_shape {row:?}"),
            _ => {
                let (got, want) = (got.unwrap_or(f32::NAN), t.f32(row, "value"));
                assert!((got - want).abs() <= 1e-6, "pattern_shape {row:?}: got {got}");
            }
        }
    }
}
