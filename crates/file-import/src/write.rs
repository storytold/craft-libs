//! Shared writer selection and serialization for neutral scenes and pixels.

use kurbo::{BezPath, PathEl, Point};

use crate::{CAPABILITIES, Error, ImportFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteFormat {
    Svg,
    Svgz,
    Dxf,
    Pdf,
    Png,
    Jpeg,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputLayer {
    /// Product-facing name. SVG stores it as `data-layer`.
    pub name: String,
    /// Already mapped/sanitized format name, supplied by the application adapter.
    pub dxf_name: String,
    pub visible: bool,
    pub dxf_color_index: u8,
    pub class: Option<String>,
    pub data_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputPath {
    pub path: BezPath,
    pub layer: usize,
    pub color_rgb: [u8; 3],
    pub stroke_width_mm: f64,
    pub class: Option<String>,
    pub data_kind: Option<String>,
    /// Close each open contour for formats that require closed cutting operations.
    pub close_open_contours: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputScene {
    pub width_mm: f64,
    pub height_mm: f64,
    pub view_box: [f64; 4],
    pub layers: Vec<OutputLayer>,
    pub paths: Vec<OutputPath>,
}

/// One page for the shared PDF path writer. Coordinates and stroke widths are millimetres.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfPage {
    pub width_mm: f64,
    pub height_mm: f64,
    pub view_box: [f64; 4],
    pub paths: Vec<PdfPath>,
}

/// A PDF path with either a fill or a stroke. `None` fills the path; `Some(width)` strokes it.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfPath {
    pub path: BezPath,
    pub color_rgb: [u8; 3],
    pub stroke_width_mm: Option<f64>,
}

/// Match a target extension to the centralized writer registry.
pub fn writer_format_for_extension(extension: &str) -> Option<WriteFormat> {
    let family = writer_family_for_extension(extension)?;
    let extension = extension.trim_start_matches('.');
    Some(match family {
        ImportFormat::Svg => WriteFormat::Svg,
        ImportFormat::Svgz => WriteFormat::Svgz,
        ImportFormat::Dxf => WriteFormat::Dxf,
        ImportFormat::Pdf => WriteFormat::Pdf,
        ImportFormat::Raster if extension.eq_ignore_ascii_case("png") => WriteFormat::Png,
        ImportFormat::Raster => WriteFormat::Jpeg,
        ImportFormat::Eps => return None,
    })
}

/// Product handler family for a writer extension.
pub fn writer_family_for_extension(extension: &str) -> Option<ImportFormat> {
    let extension = extension.trim_start_matches('.');
    for capability in CAPABILITIES {
        if !capability.write_extensions.iter().any(|candidate| candidate.eq_ignore_ascii_case(extension)) {
            continue;
        }
        return Some(capability.format);
    }
    None
}

/// Vector extensions the writer registry supports.
pub fn supported_vector_write_extensions() -> Vec<&'static str> {
    CAPABILITIES.iter().filter(|capability| capability.vector).flat_map(|capability| capability.write_extensions.iter().copied()).collect()
}

/// Serialize a neutral vector scene as SVG, SVGZ, DXF, or PDF according to the extension.
pub fn write_vector(extension: &str, scene: &OutputScene) -> Result<Vec<u8>, Error> {
    match writer_format_for_extension(extension).ok_or_else(|| Error::Write(format!("no writer for .{extension}")))? {
        WriteFormat::Svg => write_svg(scene).map(String::into_bytes),
        WriteFormat::Svgz => write_svg(scene).map(|svg| craft_svg::svgz::compress(svg.as_bytes())),
        WriteFormat::Dxf => write_dxf(scene).map(String::into_bytes),
        WriteFormat::Pdf => write_pdf(scene),
        WriteFormat::Png | WriteFormat::Jpeg => Err(Error::Write("raster output requires pixels".into())),
    }
}

/// Encode an RGBA image using the writer selected by its extension.
pub fn write_raster(extension: &str, width: u32, height: u32, rgba: &[u8], jpeg_quality: u8) -> Result<Vec<u8>, Error> {
    let pixels = craft_raster_codecs::common::Pixels::from_raw(width, height, rgba.to_vec()).ok_or(Error::InvalidGeometry)?;
    match writer_format_for_extension(extension).ok_or_else(|| Error::Write(format!("no writer for .{extension}")))? {
        WriteFormat::Png => craft_raster_codecs::common::encode_png(&pixels).map_err(|error| Error::Write(error.to_string())),
        WriteFormat::Jpeg => craft_raster_codecs::common::encode_jpeg(&pixels, jpeg_quality).map_err(|error| Error::Write(error.to_string())),
        _ => Err(Error::Write("vector output requires paths".into())),
    }
}

fn write_svg(scene: &OutputScene) -> Result<String, Error> {
    validate_scene(scene)?;
    let view_scale = (scene.width_mm / scene.view_box[2]).min(scene.height_mm / scene.view_box[3]);
    if !(view_scale.is_finite() && view_scale > 0.0) {
        return Err(Error::InvalidGeometry);
    }
    let paths_by_layer = paths_by_layer(scene);
    let elements = scene
        .layers
        .iter()
        .enumerate()
        .map(|(layer_index, layer)| {
            let paths = paths_by_layer[layer_index]
                .iter()
                .map(|path| {
                    let color = path.color_rgb;
                    let mut attributes = vec![
                        craft_svg::write::Attribute::text("fill", "none"),
                        craft_svg::write::Attribute::text("stroke", format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])),
                        craft_svg::write::Attribute::text("stroke-width", format_float(path.stroke_width_mm / view_scale)),
                    ];
                    if let Some(class) = &path.class {
                        attributes.push(craft_svg::write::Attribute::text("class", class.clone()));
                    }
                    if let Some(data_kind) = &path.data_kind {
                        attributes.push(craft_svg::write::Attribute::text("data-kind", data_kind.clone()));
                    }
                    craft_svg::write::Element::Path { attributes, path: path.path.clone() }
                })
                .collect::<Vec<_>>();
            let mut attributes = vec![
                craft_svg::write::Attribute::text("data-layer", layer.name.clone()),
                craft_svg::write::Attribute::text("style", if layer.visible { "display:inline" } else { "display:none" }),
            ];
            if let Some(class) = &layer.class {
                attributes.push(craft_svg::write::Attribute::text("class", class.clone()));
            }
            if let Some(data_kind) = &layer.data_kind {
                attributes.push(craft_svg::write::Attribute::text("data-kind", data_kind.clone()));
            }
            craft_svg::write::Element::Group { attributes, children: paths }
        })
        .collect();
    Ok(craft_svg::write::Scene { width_mm: scene.width_mm, height_mm: scene.height_mm, view_box: scene.view_box, elements }.write())
}

fn write_pdf(scene: &OutputScene) -> Result<Vec<u8>, Error> {
    validate_scene(scene)?;
    let paths_by_layer = paths_by_layer(scene);
    // Match SVG's painter order: layers in order, then paths in source order within each layer.
    let paths = scene
        .layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| layer.visible)
        .flat_map(|(layer_index, _)| paths_by_layer[layer_index].iter().copied())
        .map(|path| PdfPath { path: path.path.clone(), color_rgb: path.color_rgb, stroke_width_mm: Some(path.stroke_width_mm) })
        .collect();
    write_pdf_pages(&[PdfPage { width_mm: scene.width_mm, height_mm: scene.height_mm, view_box: scene.view_box, paths }])
}

/// Group the neutral paths once so writing does not rescan all paths for every layer.
fn paths_by_layer(scene: &OutputScene) -> Vec<Vec<&OutputPath>> {
    let mut grouped: Vec<Vec<&OutputPath>> = (0..scene.layers.len()).map(|_| Vec::new()).collect();
    for path in &scene.paths {
        if let Some(layer) = grouped.get_mut(path.layer) {
            layer.push(path);
        }
    }
    grouped
}

/// Encode a neutral list of laid-out pages with vector paths as PDF.
pub fn write_pdf_pages(pages: &[PdfPage]) -> Result<Vec<u8>, Error> {
    use craft_pdf::write::{Metadata, Writer, backend};
    const PT_PER_MM: f64 = 72.0 / 25.4;
    craft_pdf::write::guard(|| {
        let settings = backend::SerializeSettings {
            configuration: craft_pdf::write::configuration(backend::configure::PdfVersion::Pdf14, None)?,
            ..Default::default()
        };
        let mut writer = Writer::new(settings, Metadata::default());
        for source_page in pages {
            let [view_x, view_y, view_width, view_height] = source_page.view_box;
            if !(source_page.width_mm.is_finite()
                && source_page.height_mm.is_finite()
                && source_page.width_mm > 0.0
                && source_page.height_mm > 0.0
                && view_x.is_finite()
                && view_y.is_finite()
                && view_width.is_finite()
                && view_height.is_finite()
                && view_width > 0.0
                && view_height > 0.0)
            {
                return Err(Error::InvalidGeometry);
            }
            let width = source_page.width_mm * PT_PER_MM;
            let height = source_page.height_mm * PT_PER_MM;
            if !(width.is_finite()
                && height.is_finite()
                && width > 0.0
                && height > 0.0
                && width <= f64::from(f32::MAX)
                && height <= f64::from(f32::MAX))
            {
                return Err(Error::Write("page dimensions are outside the PDF range".into()));
            }
            let page_settings = backend::page::PageSettings::from_wh(width as f32, height as f32)
                .ok_or_else(|| Error::Write("page dimensions are not finite or positive".into()))?;
            let mut page = writer.document_mut().start_page_with(page_settings);
            let mut surface = page.surface();
            let view_scale = (source_page.width_mm / view_width).min(source_page.height_mm / view_height);
            let offset_x = (source_page.width_mm - view_width * view_scale) / 2.0;
            let offset_y = (source_page.height_mm - view_height * view_scale) / 2.0;
            if !(view_scale.is_finite() && view_scale > 0.0 && offset_x.is_finite() && offset_y.is_finite()) {
                return Err(Error::InvalidGeometry);
            }
            for source in &source_page.paths {
                let mut path = backend::geom::PathBuilder::new();
                for element in source.path.elements() {
                    let point = |p: Point| -> Result<(f32, f32), Error> {
                        let x = ((p.x - view_x) * view_scale + offset_x) * PT_PER_MM;
                        let y = ((p.y - view_y) * view_scale + offset_y) * PT_PER_MM;
                        if !(p.x.is_finite()
                            && p.y.is_finite()
                            && x.is_finite()
                            && y.is_finite()
                            && x.abs() <= f64::from(f32::MAX)
                            && y.abs() <= f64::from(f32::MAX))
                        {
                            return Err(Error::Write("path coordinate is outside the PDF range".into()));
                        }
                        Ok((x as f32, y as f32))
                    };
                    match *element {
                        PathEl::MoveTo(p) => {
                            let (x, y) = point(p)?;
                            path.move_to(x, y);
                        }
                        PathEl::LineTo(p) => {
                            let (x, y) = point(p)?;
                            path.line_to(x, y);
                        }
                        PathEl::QuadTo(a, b) => {
                            let (x1, y1) = point(a)?;
                            let (x, y) = point(b)?;
                            path.quad_to(x1, y1, x, y);
                        }
                        PathEl::CurveTo(a, b, c) => {
                            let (x1, y1) = point(a)?;
                            let (x2, y2) = point(b)?;
                            let (x, y) = point(c)?;
                            path.cubic_to(x1, y1, x2, y2, x, y);
                        }
                        PathEl::ClosePath => path.close(),
                    }
                }
                if let Some(path) = path.finish() {
                    let color = backend::color::rgb::Color::new(source.color_rgb[0], source.color_rgb[1], source.color_rgb[2]);
                    match source.stroke_width_mm {
                        Some(width_mm) => {
                            let width_pt = width_mm * PT_PER_MM;
                            if !(width_mm.is_finite() && width_mm > 0.0 && width_pt.is_finite() && width_pt <= f64::from(f32::MAX)) {
                                return Err(Error::Write("stroke width must be finite and positive".into()));
                            }
                            surface.set_fill(None);
                            surface.set_stroke(Some(backend::paint::Stroke { paint: color.into(), width: width_pt as f32, ..Default::default() }));
                        }
                        None => {
                            surface.set_stroke(None);
                            surface.set_fill(Some(backend::paint::Fill { paint: color.into(), ..Default::default() }));
                        }
                    }
                    surface.draw_path(&path);
                }
            }
            surface.finish();
            page.finish();
        }
        writer.finish().map_err(|error| Error::Write(error.to_string()))
    })
}

fn write_dxf(scene: &OutputScene) -> Result<String, Error> {
    validate_scene(scene)?;
    let mut drawing = craft_dxf::model::write::Drawing::default();
    drawing.format = craft_dxf::model::write::Format::R14;
    drawing.insunits = 4;
    drawing.metric = true;
    drawing.extents = [0.0, 0.0, scene.width_mm, scene.height_mm];
    drawing.limits = drawing.extents;
    for layer in &scene.layers {
        drawing.layers.push(craft_dxf::model::Layer {
            handle: None,
            name: layer.dxf_name.clone(),
            color: craft_dxf::model::Color::Index(layer.dxf_color_index),
            on: layer.visible,
            frozen: false,
            vp_freeze_new: false,
            locked: false,
            plot: true,
            linetype: "Continuous".into(),
            lineweight: craft_dxf::model::Lineweight::Default,
            plot_style: None,
            transparency: craft_dxf::model::Transparency::ByLayer,
            description: None,
        });
    }
    for output in &scene.paths {
        let Some(layer) = scene.layers.get(output.layer) else {
            return Err(Error::InvalidGeometry);
        };
        let view_scale = (scene.width_mm / scene.view_box[2]).min(scene.height_mm / scene.view_box[3]);
        let offset_x = (scene.width_mm - scene.view_box[2] * view_scale) / 2.0;
        let offset_y = (scene.height_mm - scene.view_box[3] * view_scale) / 2.0;
        let mut page_path = output.path.clone();
        page_path.apply_affine(kurbo::Affine::new([
            view_scale,
            0.0,
            0.0,
            view_scale,
            offset_x - scene.view_box[0] * view_scale,
            offset_y - scene.view_box[1] * view_scale,
        ]));
        for (points, closed) in flatten(&page_path)? {
            if points.len() < 2 {
                continue;
            }
            let mut vertices = Vec::with_capacity(points.len());
            for point in points {
                if !(point.x.is_finite() && point.y.is_finite()) {
                    return Err(Error::InvalidGeometry);
                }
                vertices.push(craft_dxf::model::LwVertex { x: point.x, y: scene.height_mm - point.y, ..Default::default() });
            }
            drawing.entities.push(craft_dxf::model::Entity {
                common: craft_dxf::model::Common {
                    handle: None,
                    owner: None,
                    layer: layer.dxf_name.clone(),
                    linetype: None,
                    color: craft_dxf::model::Color::ByLayer,
                    lineweight: craft_dxf::model::Lineweight::ByLayer,
                    ltscale: 1.0,
                    visible: true,
                    paper_space: false,
                    transparency: craft_dxf::model::Transparency::ByLayer,
                    thickness: 0.0,
                    extrusion: [0.0, 0.0, 1.0],
                    plot_style: None,
                },
                kind: craft_dxf::model::EntityKind::LwPolyline(craft_dxf::model::LwPolyline {
                    vertices,
                    closed: closed || output.close_open_contours,
                    plinegen: false,
                    const_width: 0.0,
                    elevation: 0.0,
                }),
            });
        }
    }
    let (tags, skipped) = drawing.tags();
    if skipped > 0 {
        return Err(Error::Write(format!("DXF writer skipped {skipped} entities")));
    }
    Ok(craft_dxf::write_ascii(&tags))
}

fn flatten(path: &BezPath) -> Result<Vec<(Vec<Point>, bool)>, Error> {
    const MAX_POINTS: usize = 2_000_000;
    let mut contours: Vec<(Vec<Point>, bool)> = Vec::new();
    let mut count = 0usize;
    vectorcraft_geom::flatten(path.iter(), 0.05, |element| {
        count = count.saturating_add(1);
        if count > MAX_POINTS {
            return;
        }
        match element {
            PathEl::MoveTo(point) => contours.push((vec![point], false)),
            PathEl::LineTo(point) => {
                if let Some((points, _)) = contours.last_mut() {
                    points.push(point);
                }
            }
            PathEl::ClosePath => {
                if let Some((_, closed)) = contours.last_mut() {
                    *closed = true;
                }
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
        }
    });
    if count > MAX_POINTS {
        return Err(Error::Write("path exceeds the DXF flattening limit".into()));
    }
    Ok(contours)
}

fn validate_scene(scene: &OutputScene) -> Result<(), Error> {
    if !(scene.width_mm.is_finite() && scene.height_mm.is_finite() && scene.width_mm > 0.0 && scene.height_mm > 0.0) {
        return Err(Error::InvalidGeometry);
    }
    if !(scene.view_box.iter().all(|value| value.is_finite()) && scene.view_box[2] > 0.0 && scene.view_box[3] > 0.0)
        || scene.paths.len() > crate::MAX_SHAPES
        || scene.paths.iter().any(|path| {
            path.layer >= scene.layers.len()
                || !(path.stroke_width_mm.is_finite() && path.stroke_width_mm > 0.0)
                || path.path.elements().iter().any(|element| match *element {
                    PathEl::MoveTo(point) | PathEl::LineTo(point) => !point_is_finite(point),
                    PathEl::QuadTo(a, b) => !point_is_finite(a) || !point_is_finite(b),
                    PathEl::CurveTo(a, b, c) => !point_is_finite(a) || !point_is_finite(b) || !point_is_finite(c),
                    PathEl::ClosePath => false,
                })
        })
    {
        return Err(Error::InvalidGeometry);
    }
    Ok(())
}

fn point_is_finite(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

fn format_float(value: f64) -> String {
    value.to_string()
}
