//! Stories of `GlyphsPanel`: the panel over Ubuntu Light and Hack with three recent glyphs, while text is edited and
//! while it is not. The glyph source draws the shipped faces' outlines with skrifa.

use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, ColorImage, Ui};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider as _};

use super::{GlyphSource, GlyphsPanel, GlyphsPanelSettings};

const UBUNTU: &[u8] = epaint_default_fonts::UBUNTU_LIGHT;
const HACK: &[u8] = epaint_default_fonts::HACK_REGULAR;

/// What a story keeps between passes.
#[derive(Clone, Debug, PartialEq)]
struct Demo {
    family: String,
    style: String,
    settings: GlyphsPanelSettings,
}

fn show_panel(ui: &mut Ui, data: &mut StoryData, editing: bool) -> StoryOutput {
    let init = Demo {
        family: "Ubuntu".to_owned(),
        style: "Light".to_owned(),
        settings: GlyphsPanelSettings { recent: ["€", "→", "ß"].map(str::to_owned).to_vec(), ..GlyphsPanelSettings::default() },
    };
    let Demo { family, style, settings } = data.value("glyphs", init);
    let families = ["Ubuntu".to_owned(), "Hack".to_owned()];
    let styles = [if family == "Hack" { "Regular" } else { "Light" }.to_owned()];
    style.clone_from(&styles[0]);
    GlyphsPanel::new("Glyphs", &FaceGlyphs, Bind::bound(family), Bind::bound(style), Bind::bound(settings))
        .editing(editing)
        .families(&families)
        .styles(&styles)
        .show(ui)
        .into()
}

#[story(GlyphsPanel<'static>, covers = [Default, Many], drive = Drive::Never)]
fn glyphs_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_panel(ui, data, false)
}

#[story(GlyphsPanel<'static>, covers = [], drive = Drive::Never)]
fn glyphs_panel_editing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_panel(ui, data, true)
}

/// Glyphs of the shipped faces, drawn by skrifa's outlines.
struct FaceGlyphs;

impl FaceGlyphs {
    fn font(family: &str) -> Option<FontRef<'static>> {
        FontRef::new(if family == "Hack" { HACK } else { UBUNTU }).ok()
    }
}

impl GlyphSource for FaceGlyphs {
    fn charmap(&self, family: &str, _style: &str) -> Vec<char> {
        let Some(font) = Self::font(family) else { return Vec::new() };
        let mut chars: Vec<char> = font.charmap().mappings().filter_map(|(code, _)| char::from_u32(code)).filter(|c| !c.is_control()).collect();
        chars.sort_unstable();
        chars.dedup();
        chars
    }

    fn glyph(&self, family: &str, _style: &str, c: char, px: u32) -> Option<ColorImage> {
        let font = Self::font(family)?;
        let id = font.charmap().map(c)?;
        let em = px as f32 * 0.8;
        let size = Size::new(em);
        let advance = font.glyph_metrics(size, LocationRef::default()).advance_width(id).unwrap_or(em);
        let baseline = px as f32 * 0.78;
        let mut pen = Outline { edges: Vec::new(), start: (0.0, 0.0), at: (0.0, 0.0), dx: (px as f32 - advance) / 2.0, baseline };
        font.outline_glyphs().get(id)?.draw(DrawSettings::unhinted(size, LocationRef::default()), &mut pen).ok()?;
        Some(rasterize(&pen.edges, px as usize))
    }

    fn in_category(&self, c: char, category: &str) -> bool {
        let code = c as u32;
        match category {
            "Punctuation" => c.is_ascii_punctuation() && !"$+<=>^`|~".contains(c) || "¡¿«»·".contains(c) || (0x2010..=0x2027).contains(&code),
            "Numbers" => c.is_numeric(),
            "Currency" => "$¢£¤¥".contains(c) || (0x20A0..=0x20CF).contains(&code),
            "Symbols" => "©®°§¶†‡•".contains(c) || (0x2100..=0x2BFF).contains(&code) && !(0x2190..=0x22FF).contains(&code),
            "Math Symbols" => "+−×÷±<=>¬".contains(c) || (0x2200..=0x22FF).contains(&code),
            "Arrows" => (0x2190..=0x21FF).contains(&code),
            "Latin" => c.is_alphabetic() && code < 0x250,
            "Greek" => (0x370..=0x3FF).contains(&code),
            "Cyrillic" => (0x400..=0x52F).contains(&code),
            "Letterlike Symbols" => (0x2100..=0x214F).contains(&code),
            "Superscripts and Subscripts" => "¹²³".contains(c) || (0x2070..=0x209F).contains(&code),
            "Ornaments" => (0x2766..=0x2767).contains(&code) || code == 0x2042,
            _ => false,
        }
    }
}

type Segment = ((f32, f32), (f32, f32));

/// An outline flattened to line segments in image pixels, y down.
struct Outline {
    edges: Vec<Segment>,
    start: (f32, f32),
    at: (f32, f32),
    dx: f32,
    baseline: f32,
}

impl Outline {
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (x + self.dx, self.baseline - y)
    }

    fn line(&mut self, to: (f32, f32)) {
        self.edges.push((self.at, to));
        self.at = to;
    }
}

const FLATTEN: usize = 8;

impl OutlinePen for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.at = self.point(x, y);
        self.start = self.at;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.point(x, y);
        self.line(to);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.at, self.point(cx0, cy0), self.point(x, y));
        for i in 1..=FLATTEN {
            let t = i as f32 / FLATTEN as f32;
            let u = 1.0 - t;
            self.line((u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0, u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1));
        }
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.at, self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y));
        for i in 1..=FLATTEN {
            let t = i as f32 / FLATTEN as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.line((a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1));
        }
    }

    fn close(&mut self) {
        let start = self.start;
        self.line(start);
    }
}

/// Sub-scanlines per pixel row; coverage across a row is exact.
const SUB_ROWS: usize = 4;

/// White with the nonzero-winding coverage of `edges` in the alpha channel.
fn rasterize(edges: &[Segment], side: usize) -> ColorImage {
    let mut alpha = vec![0.0f32; side * side];
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for row in 0..side {
        let line = &mut alpha[row * side..(row + 1) * side];
        for sub in 0..SUB_ROWS {
            let y = row as f32 + (sub as f32 + 0.5) / SUB_ROWS as f32;
            crossings.clear();
            for &((x0, y0), (x1, y1)) in edges {
                if (y0 <= y && y < y1) || (y1 <= y && y < y0) {
                    crossings.push((x0 + (y - y0) * (x1 - x0) / (y1 - y0), if y1 > y0 { 1 } else { -1 }));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for pair in crossings.windows(2) {
                winding += pair[0].1;
                if winding == 0 {
                    continue;
                }
                let (from, to) = (pair[0].0.clamp(0.0, side as f32), pair[1].0.clamp(0.0, side as f32));
                let mut x = from.floor() as usize;
                while (x as f32) < to && x < side {
                    let covered = (to.min(x as f32 + 1.0) - from.max(x as f32)).max(0.0);
                    line[x] += covered / SUB_ROWS as f32;
                    x += 1;
                }
            }
        }
    }
    ColorImage::new([side, side], alpha.iter().map(|a| Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8)).collect::<Vec<_>>())
}
