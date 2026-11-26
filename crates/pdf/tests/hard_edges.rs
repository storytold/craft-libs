//! Hard edges and supersampling (`Options::hard_edges`, `raster::render_supersampled_into`):
//! black shapes that share an edge on a white page leave no grey seam.
#![cfg(feature = "raster")]
use craft_pdf::{
    open_slice,
    raster::{self, Error, Options, RenderCache},
};

/// A white-backed render of a one-page PDF `width` × `height` points drawing `content`, at one
/// pixel per point; `factor` 0 is a plain render, any other a supersampled one.
fn render(width: u32, height: u32, content: &str, options: Options, factor: u32) -> Result<Vec<u8>, Error> {
    render_with(width, height, content, "", &[], options, factor)
}

/// [`render`] with page `resources` and stream `objects`, numbered from 5, each its dictionary
/// entries and its data.
fn render_with(
    width: u32,
    height: u32,
    content: &str,
    resources: &str,
    objects: &[(&str, &str)],
    options: Options,
    factor: u32,
) -> Result<Vec<u8>, Error> {
    let extra: String = objects
        .iter()
        .enumerate()
        .map(|(i, (dict, data))| format!("{} 0 obj << {dict} /Length {} >> stream\n{data}\nendstream endobj\n", i + 5, data.len()))
        .collect();
    let bytes = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << {resources} >> /Contents 4 0 R >> endobj\n\
         4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n{extra}\
         trailer << /Root 1 0 R >>\n%%EOF",
        content.len()
    );
    let pdf = open_slice(bytes.as_bytes(), None).unwrap();
    let page = &pdf.pages()[0];
    let options = Options { background: [255; 4], ..options };
    let mut buffer = vec![0u8; (width * height * 4) as usize];
    let cache = RenderCache::new();
    if factor == 0 {
        raster::render_into(page, &cache, &Default::default(), &options, &mut buffer)?;
    } else {
        raster::render_supersampled_into(page, &cache, &Default::default(), &options, factor, &mut buffer)?;
    }
    Ok(buffer)
}

/// The lightest red value among pixels `xs` × `ys` of a `width`-pixel-wide raster: 0 where every
/// one is black.
fn lightest(rgba: &[u8], width: u32, xs: std::ops::Range<u32>, ys: std::ops::Range<u32>) -> u8 {
    ys.flat_map(|y| xs.clone().map(move |x| ((y * width + x) * 4) as usize)).map(|i| rgba[i]).max().unwrap()
}

/// Two black boxes on a 20 × 4 page meeting at `x`.
fn join(x: f32) -> String {
    format!("0 0 {x} 4 re f {x} 0 {} 4 re f", 20.0 - x)
}

/// A black square from 2 to 18 on a 20 × 20 page, filled as its two halves either side of the
/// diagonal.
const DIAGONAL: &str = "2 2 m 18 2 l 18 18 l h f 2 2 m 18 18 l 2 18 l h f";

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn default_rendering_antialiases_and_shows_the_join() {
    // The case the option exists for, pinned so a change of default shows: anti-aliased edges
    // are partial, and two of them composited in turn let the white page through.
    let edge = render(20, 4, "0 0 10.5 4 re f", Options::default(), 0).unwrap();
    assert!((64..192).contains(&edge[10 * 4]), "a half-covered pixel is grey: {}", edge[10 * 4]);
    for x in [10.3, 10.5] {
        let page = render(20, 4, &join(x), Options::default(), 0).unwrap();
        assert!(lightest(&page, 20, 0..20, 0..4) > 32, "join at {x}");
    }
    let page = render(20, 20, DIAGONAL, Options::default(), 0).unwrap();
    assert!(lightest(&page, 20, 2..18, 2..18) > 32);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn hard_edges_leave_no_join() {
    let hard = Options { hard_edges: true, ..Default::default() };
    for x in [10.0, 10.3, 10.5] {
        let page = render(20, 4, &join(x), hard, 0).unwrap();
        assert_eq!(lightest(&page, 20, 0..20, 0..4), 0, "join at {x}");
    }
    let page = render(20, 20, DIAGONAL, hard, 0).unwrap();
    assert_eq!(lightest(&page, 20, 2..18, 2..18), 0);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_supersampled_render_is_smooth_with_no_join() {
    for x in [10.3, 10.5] {
        let page = render(20, 4, &join(x), Options::default(), 4).unwrap();
        assert!(lightest(&page, 20, 0..20, 0..4) <= 8, "join at {x}");
    }
    let page = render(20, 20, DIAGONAL, Options::default(), 4).unwrap();
    assert!(lightest(&page, 20, 2..18, 2..18) <= 8);
    // A lone edge partway across a pixel still comes out partial, not stepped.
    let edge = render(20, 4, "0 0 10.5 4 re f", Options::default(), 4).unwrap();
    assert!((64..192).contains(&edge[10 * 4]), "a half-covered pixel is grey: {}", edge[10 * 4]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_supersampled_hairline_stays_one_output_pixel_wide() {
    // A zero-width line is drawn at the thinnest-stroke floor; measured in fine pixels it would
    // shrink to a quarter of an output pixel and print light grey.
    let page = render(20, 4, "0 w 10.5 0 m 10.5 4 l S", Options::default(), 4).unwrap();
    assert!(page[10 * 4] < 64, "hairline pixel: {}", page[10 * 4]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_fine_raster_past_the_limit_is_refused() {
    let side = raster::MAX_SIDE / 2;
    assert_eq!(render(side, 1, "", Options::default(), 4), Err(Error::OutputLimit));
    assert_eq!(render(4, 1, "", Options::default(), raster::MAX_SUPERSAMPLE + 1), Err(Error::InvalidOptions));
}

fn hard() -> Options {
    Options { hard_edges: true, ..Default::default() }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_tiling_pattern_cell_is_drawn_with_hard_edges() {
    // The cell is drawn by a renderer of its own, which must take the page's edges.
    let join = join(10.3);
    let pattern = ("/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 20 4] /XStep 20 /YStep 4 /Resources << >>", join.as_str());
    let page = render_with(20, 4, "/Pattern cs /P0 scn 0 0 20 4 re f", "/Pattern << /P0 5 0 R >>", &[pattern], hard(), 0).unwrap();
    assert_eq!(lightest(&page, 20, 0..20, 0..4), 0);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn shapes_drawn_after_an_image_keep_hard_edges() {
    // The image path sets a threshold of its own for the image, then must put back the page's.
    let image = "q 1 0 0 1 0 0 cm BI /W 1 /H 1 /CS /G /BPC 8 /F /AHx ID 00> EI Q ";
    let page = render(20, 4, &format!("{image}{}", join(10.3)), hard(), 0).unwrap();
    assert_eq!(lightest(&page, 20, 0..20, 0..4), 0);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_isolated_cmyk_group_is_drawn_with_hard_edges() {
    // An isolated group blending in DeviceCMYK is drawn as ink passes, each by a renderer of its
    // own. Its black converts through the CMYK profile, so the join must match the inside.
    let content = format!("0 0 0 1 k {}", join(10.3));
    let group =
        ("/Type /XObject /Subtype /Form /BBox [0 0 20 4] /Group << /S /Transparency /CS /DeviceCMYK /I true >> /Resources << >>", content.as_str());
    let page = render_with(20, 4, "/X0 Do", "/XObject << /X0 5 0 R >>", &[group], hard(), 0).unwrap();
    let inside = page[2 * 4];
    assert!(inside < 128, "the group is drawn: {inside}");
    assert_eq!(lightest(&page, 20, 0..20, 0..4), inside);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_hairline_on_a_pixel_boundary_doubles_with_hard_edges() {
    // A `0 w` line at x = 10 is widened to the one-pixel floor, 9.5 to 10.5: half of pixels 9 and
    // 10, and the threshold counts both as covered. Supersampling keeps one pixel of ink.
    let ink = |page: Vec<u8>| (0..20).map(|x| 255 - u32::from(page[x * 4])).sum::<u32>();
    let stepped = ink(render(20, 4, "0 w 10 0 m 10 4 l S", hard(), 0).unwrap());
    assert_eq!(stepped, 2 * 255, "two whole pixels");
    let smooth = ink(render(20, 4, "0 w 10 0 m 10 4 l S", Options::default(), 4).unwrap());
    assert!((192..320).contains(&smooth), "about one pixel: {smooth}");
}
