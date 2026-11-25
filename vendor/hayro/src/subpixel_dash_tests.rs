//! PdfCraft patch, storytold/pdfcraft#7311: a dash pattern whose period is under a tenth of a
//! device pixel is drawn solid (ISO 32000-2 §8.4.3.6 measures dashes in user space). Plans that
//! stroke 157,000 short segments dashed `[.001 .001]`, a period of 0.0027 px at 96 dpi, kept
//! each stroke under `MAX_DASHES_PER_STROKE` yet cut the page into some 10^8 pieces. Adapted
//! from PdfCraft's renderer test; `renderer::dash_tests` checks `usable_dash_pattern` itself.

use crate::{InterpreterSettings, Pdf, RenderCache, RenderSettings, render};
use vello_cpu::color::palette::css::WHITE;

/// A 40 × 40 pt page whose content is `content`, at one pixel a point over white.
fn render_page(content: &str) -> vello_cpu::Pixmap {
    let pdf = format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
trailer << /Root 1 0 R >>
%%EOF",
        content.len()
    );
    let pdf = Pdf::new(pdf.into_bytes()).expect("the fixture parses");
    let cache = RenderCache::new();
    let settings = RenderSettings {
        bg_color: WHITE,
        ..RenderSettings::default()
    };
    render(
        pdf.pages().get(0).expect("the fixture has one page"),
        &cache,
        &InterpreterSettings::default(),
        &settings,
    )
}

fn px(page: &vello_cpu::Pixmap, x: u16, y: u16) -> [u8; 4] {
    let c = page.sample(x, y);
    [c.r, c.g, c.b, c.a]
}

/// A pattern whose period is under a tenth of a device pixel is drawn solid, as PDFium and MuPDF
/// draw it, so the stroke is never cut; expanded, those dashes averaged to about half the ink.
/// A period of a tenth of a pixel or more keeps its dashes.
#[test]
fn dash_patterns_finer_than_a_tenth_of_a_pixel_are_drawn_solid() {
    let red = [255, 0, 0, 255];
    for (what, content) in [
        (
            "[.001 .001]",
            "1 0 0 RG 4 w [.001 .001] 0 d 10 20 m 30 20 l S".to_string(),
        ),
        (
            "[.001 .003] with a phase",
            "1 0 0 RG 4 w [.001 .003] .002 d 10 20 m 30 20 l S".to_string(),
        ),
        (
            "[.04], a period of 0.08 px",
            "1 0 0 RG 4 w [.04] 0 d 10 20 m 30 20 l S".to_string(),
        ),
        (
            "many short strokes",
            "1 0 0 RG 4 w [.001 .001] 0 d 10 20 m 30 20 l S ".repeat(200),
        ),
    ] {
        let page = render_page(&content);
        for x in [12, 20, 28] {
            assert_eq!(px(&page, x, 20), red, "{what}: ({x}, 20) is drawn solid");
        }
    }
    // `[.001 .001]` under a 100× matrix is a period of 0.2 px, half ink; `[.06]` is 0.12 px;
    // `[0.1]` is 0.2 px.
    for (what, content) in [
        (
            "[.001 .001] enlarged",
            "1 0 0 RG 100 0 0 100 0 0 cm 0.04 w [.001 .001] 0 d .1 .2 m .3 .2 l S",
        ),
        ("[.06]", "1 0 0 RG 4 w [.06] 0 d 10 20 m 30 20 l S"),
        ("[0.1]", "1 0 0 RG 4 w [0.1] 0 d 10 20 m 30 20 l S"),
    ] {
        let page = render_page(content);
        let p = px(&page, 20, 20);
        assert!(
            p[0] == 255 && (40..=215).contains(&p[1]),
            "{what}: still dashed, about half ink: {p:?}"
        );
    }
}
