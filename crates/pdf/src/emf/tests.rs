use super::*;

/// A one-page 200 × 100 pt PDF drawing `content`, with `resources` as its resource dictionary.
fn pdf(content: &str, resources: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources {resources} /Contents 4 0 R >>"),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn picture(bytes: &[u8]) -> craft_metafile::Picture {
    let emf = Pictures::open(bytes, Limits::default()).unwrap().page(0, "test", "page").unwrap();
    assert!((emf.size.0 - 200.0).abs() < 0.5 && (emf.size.1 - 100.0).abs() < 0.5, "{:?}", emf.size);
    craft_metafile::parse(&emf.bytes).unwrap()
}

/// The `[x0, y0, x1, y1]` bounds of a path item's points.
fn bounds(item: &craft_metafile::Item) -> Option<[f32; 4]> {
    let craft_metafile::Item::Path { segs, .. } = item else { return None };
    let points: Vec<(f32, f32)> = segs
        .iter()
        .flat_map(|s| match s {
            craft_metafile::Seg::Move(x, y) | craft_metafile::Seg::Line(x, y) => vec![(*x, *y)],
            craft_metafile::Seg::Cubic(a, b, c, d, e, f) => vec![(*a, *b), (*c, *d), (*e, *f)],
            craft_metafile::Seg::Close => vec![],
        })
        .collect();
    let (xs, ys): (Vec<f32>, Vec<f32>) = points.into_iter().unzip();
    let min = |v: &[f32]| v.iter().copied().fold(f32::INFINITY, f32::min);
    let max = |v: &[f32]| v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    Some([min(&xs), min(&ys), max(&xs), max(&ys)])
}

#[test]
fn fills_strokes_and_text_become_vector_paths_in_page_space() {
    let font = "<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>";
    let content = "1 0 0 rg 10 60 30 20 re f 0 0 1 RG 4 w 100 10 m 190 10 l S BT /F1 24 Tf 0 g 10 20 Td (Hi) Tj ET";
    let items = picture(&pdf(content, font)).items;
    let paths: Vec<_> = items.iter().filter(|i| matches!(i, craft_metafile::Item::Path { .. })).collect();
    // The red rectangle: near the top of the y-down picture, where PDF's y = 60..80 lands.
    let red = paths.iter().find(|i| matches!(i, craft_metafile::Item::Path { fill: Some([255, 0, 0, _]), .. })).expect("the red fill");
    let [x0, y0, x1, y1] = bounds(red).unwrap();
    assert!(x0 >= 9.0 && x1 <= 41.0 && y0 >= 19.0 && y1 <= 41.0, "{:?}", [x0, y0, x1, y1]);
    // The blue line is a stroke with its width, not a filled outline.
    assert!(paths.iter().any(|i| matches!(i, craft_metafile::Item::Path { stroke: Some(([0, 0, 255, _], w)), .. } if *w > 3.0)));
    // The two glyphs are filled outlines: black paths with curves, below the rectangle.
    let glyphs: Vec<_> = paths.iter().filter(|i| matches!(i, craft_metafile::Item::Path { fill: Some([0, 0, 0, _]), .. })).collect();
    assert!(glyphs.len() >= 2, "{glyphs:?}");
    assert!(glyphs.iter().all(|g| bounds(g).is_some_and(|b| b[1] > 50.0 && b[3] < 100.0)));
}

#[test]
fn pages_gdi_cannot_draw_are_handed_back_for_raster_printing() {
    let translucent = "<< /ExtGState << /G1 << /ca 0.5 >> >> >>";
    let multiply = "<< /ExtGState << /G1 << /BM /Multiply >> >> >>";
    for (resources, content) in [(translucent, "/G1 gs 1 0 0 rg 10 10 50 50 re f"), (multiply, "/G1 gs 1 0 0 rg 10 10 50 50 re f")] {
        let result = Pictures::open(&pdf(content, resources), Limits::default()).and_then(|p| p.page(0, "test", "page"));
        assert!(matches!(result, Err(Error::NeedsRaster(_))), "{result:?}");
    }
    // The same drawing without the graphics state is drawn.
    assert!(Pictures::open(&pdf("1 0 0 rg 10 10 50 50 re f", "<< >>"), Limits::default()).and_then(|p| p.page(0, "test", "page")).is_ok());
}

#[test]
fn clips_are_kept_and_limits_hold() {
    let content = "q 0 0 50 50 re W n 0 g 0 0 200 100 re f Q 0 g 150 50 10 10 re f";
    let items = picture(&pdf(content, "<< >>")).items;
    let clipped = items.iter().filter(|i| matches!(i, craft_metafile::Item::Path { clips, .. } if !clips.is_empty())).count();
    let free = items.iter().filter(|i| matches!(i, craft_metafile::Item::Path { clips, .. } if clips.is_empty())).count();
    assert!(clipped >= 1 && free >= 1, "{items:?}");
    let tight = Limits { max_ops: 1, ..Limits::default() };
    assert!(matches!(Pictures::open(&pdf(content, "<< >>"), tight).and_then(|p| p.page(0, "test", "page")), Err(Error::OutputLimit)));
    assert!(matches!(
        Pictures::open(&pdf(content, "<< >>"), Limits::default()).and_then(|p| p.page(1, "test", "page")),
        Err(Error::PageIndex { .. })
    ));
}

#[test]
fn pictures_can_move_to_a_print_thread() {
    fn send<T: Send>() {}
    send::<Pictures>();
}

fn drawn(content: &str) -> Result<PageEmf, Error> {
    Pictures::open(&pdf(content, "<< >>"), Limits::default()).and_then(|p| p.page(0, "test", "page"))
}

#[test]
fn a_dashed_stroke_under_a_skew_is_handed_back_and_a_solid_one_is_filled() {
    let skewed = "1 0.5 0 1 0 0 cm 0 g 2 w";
    assert!(matches!(drawn(&format!("{skewed} [3 2] 0 d 10 10 m 150 10 l S")), Err(Error::NeedsRaster(_))));
    let items = picture(&pdf(&format!("{skewed} 10 10 m 150 10 l S"), "<< >>")).items;
    assert!(items.iter().any(|i| matches!(i, craft_metafile::Item::Path { fill: Some(_), stroke: None, .. })), "{items:?}");
    // Unskewed, the same dash is a pen of its own.
    assert!(drawn("0 g 2 w [3 2] 0 d 10 10 m 150 10 l S").is_ok());
}

#[test]
fn an_image_is_drawn_and_one_that_cannot_be_decoded_is_handed_back() {
    let good = "q 50 0 0 50 10 10 cm BI /W 2 /H 1 /CS /G /BPC 8 /F /AHx ID ff00> EI Q";
    let items = picture(&pdf(good, "<< >>")).items;
    assert!(items.iter().any(|i| !matches!(i, craft_metafile::Item::Path { .. })), "{items:?}");
    let broken = "q 50 0 0 50 10 10 cm BI /W 2 /H 1 /CS /G /BPC 8 /F /DCT ID notajpeg EI Q";
    assert!(matches!(drawn(broken), Err(Error::NeedsRaster(_))));
}
