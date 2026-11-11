//! Reads the geometry of an SVG document as Bézier paths in millimetres.
//!
//! For tools that need the shapes of a drawing and not its paint: cutters, plotters, CAD import.
//! [`import`] walks the document, applies transforms, viewport units, `<use>`/`<symbol>`
//! instances and the CSS cascade, and returns each drawable element as a [`kurbo::BezPath`] in
//! millimetres. The caller tags shapes through [`Kind`] (cut or engrave, layer, pen…) from each
//! element's computed style and class. Input is treated as hostile: size, node, depth and
//! instance limits, and cyclic references, are errors.
//!
//! Import only, and geometry only: this is not the shared SVG model of craft-libs#789. Depends on
//! no other Craft crate.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use kurbo::BezPath;

/// Why an SVG could not be read; the message starts with the name given to [`import`].
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

/// An element's computed properties, by name: the ones [`Kind::PROPERTIES`] lists and a few
/// presentation properties.
pub type StyleMap = std::collections::HashMap<String, String>;

/// The caller's tag on a shape, inherited down the element tree.
pub trait Kind: Clone + Default {
    /// Property names of the caller's own (`data-kind`, say) to follow through attributes, inline
    /// styles and stylesheets, as well as `display`, `visibility`, `fill`, `stroke` and
    /// `stroke-width`.
    const PROPERTIES: &'static [&'static str] = &[];

    /// The tag an element sets, from its computed style and `class` attribute; `None` inherits
    /// its parent's.
    fn of(style: &StyleMap, class: &str) -> Option<Self>;

    /// Whether an element of this kind with `stroke: none` and `fill: none` is left out, with
    /// its children.
    fn skips_unpainted(&self) -> bool {
        true
    }
}

/// For callers that only want the paths.
impl Kind for () {
    fn of(_: &StyleMap, _: &str) -> Option<Self> {
        None
    }
}

/// One drawable element, transformed into the document's millimetre space.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape<K> {
    pub path: BezPath,
    pub kind: K,
}

/// What [`import`] read. Shapes may lie outside the `width_mm` × `height_mm` viewport.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported<K> {
    pub width_mm: f64,
    pub height_mm: f64,
    pub shapes: Vec<Shape<K>>,
}

const MAX_SVG_BYTES: usize = 16 * 1024 * 1024;
const MAX_SVG_NODES: usize = 100_000;
const MAX_SVG_DEPTH: usize = 128;
const MAX_SVG_INSTANCES: usize = 10_000;

#[derive(Clone)]
struct ImportState<K> {
    transform: kurbo::Affine,
    kind: K,
    display_none: bool,
    visibility_hidden: bool,
    inherited_style: std::collections::HashMap<String, String>,
}

impl<K: Kind> Default for ImportState<K> {
    fn default() -> Self {
        Self {
            transform: kurbo::Affine::IDENTITY,
            kind: K::default(),
            display_none: false,
            visibility_hidden: false,
            inherited_style: std::collections::HashMap::new(),
        }
    }
}

/// Reads the drawable geometry of an SVG document, in document order.
///
/// Supported geometry: `<path>`, `<rect>` (including rounded corners), `<circle>`, `<ellipse>`,
/// `<line>`, `<polyline>` and `<polygon>`. Nested groups, transforms (`matrix`, `translate`,
/// `scale`, `rotate`, `skewX/Y`) and inherited presentation and inline styles (`display`,
/// `visibility`, CSS `style`, `class`) are applied. Embedded stylesheets support type, class, ID
/// and attribute selectors, descendant and child combinators, selector lists, and a bounded
/// presentation-property cascade by specificity. Curves stay curves: nothing is flattened.
///
/// Physical root dimensions accept unitless SVG px, `px`, `mm`, `cm`, `in`, `pt` and `pc`; a
/// `viewBox` maps its user coordinates onto that physical viewport, following
/// `preserveAspectRatio`. Without physical dimensions, `viewBox` units are read as millimetres,
/// and so is a fragment without an SVG root. Local `<use>` references instantiate supported
/// shapes, groups and `<symbol>` templates with viewBox sizing. External references, text,
/// gradients, clipping and percentage lengths are not expanded; an unsupported drawable element
/// is an error, not a silent loss.
///
/// `name` (a file name, say) prefixes every error message.
pub fn import<K: Kind>(svg: &str, name: &str) -> Result<Imported<K>, Error> {
    let path_name = name;
    if svg.len() > MAX_SVG_BYTES {
        return Err(svg_error(path_name, "SVG exceeds the 16 MiB import limit"));
    }
    let xml = roxmltree::Document::parse(svg).map_err(|e| svg_error(path_name, &format!("invalid XML: {e}")))?;
    let root = xml.root_element();
    let (width_mm, height_mm, viewport) = viewport_transform(root, path_name)?;

    let mut id_index = std::collections::HashMap::new();
    let mut xml_node_count = 0usize;
    for node in root.descendants() {
        xml_node_count += 1;
        if xml_node_count > MAX_SVG_NODES {
            return Err(svg_error(path_name, "SVG exceeds the 100,000-node import limit"));
        }
        if let Some(id) = node.attribute("id")
            && id_index.insert(id.to_string(), node).is_some()
        {
            return Err(svg_error(path_name, &format!("duplicate SVG id {id:?}")));
        }
    }
    let css_rules = parse_stylesheets(root, K::PROPERTIES, path_name)?;
    let mut context =
        ImportContext { visited: 0, path_name, id_index, css_rules, reference_stack: Vec::new(), instance_count: 0, shapes: Vec::new() };
    let base = ImportState { transform: viewport, ..ImportState::default() };
    import_node(root, base, 0, &mut context)?;
    Ok(Imported { width_mm, height_mm, shapes: context.shapes })
}

struct ImportContext<'a, 'input, K> {
    visited: usize,
    path_name: &'a str,
    id_index: std::collections::HashMap<String, roxmltree::Node<'a, 'input>>,
    css_rules: Vec<CssRule>,
    reference_stack: Vec<String>,
    instance_count: usize,
    shapes: Vec<Shape<K>>,
}

fn import_node<'a, 'input, K: Kind>(
    node: roxmltree::Node<'a, 'input>,
    inherited: ImportState<K>,
    depth: usize,
    context: &mut ImportContext<'a, 'input, K>,
) -> Result<(), Error> {
    context.visited = context.visited.checked_add(1).ok_or_else(|| svg_error(context.path_name, "SVG node counter overflow"))?;
    if context.visited > MAX_SVG_NODES {
        return Err(svg_error(context.path_name, "SVG exceeds the 100,000-node import limit"));
    }
    if depth > MAX_SVG_DEPTH {
        return Err(svg_error(context.path_name, "SVG nesting exceeds 128 levels"));
    }
    let tag = node.tag_name().name();
    // Definitions are not drawable content unless instantiated via `<use>`;
    // references resolve directly into supported targets inside these containers.
    if matches!(tag, "defs" | "symbol" | "clipPath" | "mask" | "pattern") {
        return Ok(());
    }
    let mut state = inherited.clone();
    // SVG presentation attributes and inline declarations inherit. `display`
    // is different: a descendant cannot override an ancestor's display:none.
    if node.attribute("display") == Some("none") {
        state.display_none = true;
    }
    if let Some(transform) = node.attribute("transform") {
        let local = parse_transform(transform).map_err(|e| svg_error(context.path_name, &format!("<{}> transform: {e}", tag)))?;
        state.transform = compose(inherited.transform, local);
    }
    let style = computed_style(node, &state.inherited_style, &context.css_rules, K::PROPERTIES);
    state.inherited_style = style.iter().filter(|(key, _)| key.as_str() != "display").map(|(key, value)| (key.clone(), value.clone())).collect();
    if let Some(kind) = K::of(&style, node.attribute("class").unwrap_or("")) {
        state.kind = kind;
    }
    if style.get("display").is_some_and(|v| v == "none") {
        state.display_none = true;
    }
    if let Some(visibility) = style.get("visibility") {
        state.visibility_hidden = visibility == "hidden" || visibility == "collapse";
    }
    let color_property = style.get("stroke").or_else(|| style.get("fill"));
    if state.kind.skips_unpainted() && color_property.is_some_and(|v| *v == "none") {
        // A line with fill:none can still be a cut path; stroke:none plus
        // fill:none is empty and can safely be ignored.
        if style.get("stroke").is_some_and(|v| v == "none") && style.get("fill").is_some_and(|v| v == "none") {
            state.display_none = true;
        }
    }
    if tag == "use" {
        if !state.display_none {
            let href = node
                .attribute("href")
                .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")))
                .ok_or_else(|| svg_error(context.path_name, "<use> has no href reference"))?;
            let id = href
                .strip_prefix('#')
                .filter(|id| !id.is_empty())
                .ok_or_else(|| svg_error(context.path_name, "<use> supports only local fragment references"))?;
            if context.reference_stack.iter().any(|active| active == id) {
                return Err(svg_error(context.path_name, &format!("cyclic <use> reference to #{id}")));
            }
            let target =
                context.id_index.get(id).copied().ok_or_else(|| svg_error(context.path_name, &format!("<use> references missing id #{id}")))?;
            if !matches!(target.tag_name().name(), "g" | "symbol" | "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon") {
                return Err(svg_error(context.path_name, &format!("<use> reference #{id} targets unsupported <{}>", target.tag_name().name())));
            }
            let x = use_viewport_length(node, "x", 0.0, context.path_name)?;
            let y = use_viewport_length(node, "y", 0.0, context.path_name)?;
            if x > 1.0e9 || y > 1.0e9 {
                return Err(svg_error(context.path_name, "<use> coordinates exceed ±1e9"));
            }
            context.instance_count =
                context.instance_count.checked_add(1).ok_or_else(|| svg_error(context.path_name, "SVG use-instance counter overflow"))?;
            if context.instance_count > MAX_SVG_INSTANCES {
                return Err(svg_error(context.path_name, "SVG exceeds the 10,000 use-instance limit"));
            }
            if target.tag_name().name() == "symbol" {
                import_symbol(target, node, state, x, y, depth + 1, context)?;
                return Ok(());
            }
            state.transform = compose(state.transform, kurbo::Affine::translate((x, y)));
            context.reference_stack.push(id.to_string());
            let result = import_node(target, state, depth + 1, context);
            context.reference_stack.pop();
            return result;
        }
        return Ok(());
    }
    if !state.display_none
        && !state.visibility_hidden
        && let Some(path) = geometry_for(node, context.path_name)?
    {
        let transformed = transform_path(&path, state.transform);
        validate_import_path(&transformed, context.path_name)?;
        context.shapes.push(Shape { path: transformed, kind: state.kind.clone() });
    }
    for child in node.children().filter(|n| n.is_element()) {
        import_node(child, state.clone(), depth + 1, context)?;
    }
    Ok(())
}

fn import_symbol<'a, 'input, K: Kind>(
    symbol: roxmltree::Node<'a, 'input>,
    use_node: roxmltree::Node<'a, 'input>,
    mut state: ImportState<K>,
    x: f64,
    y: f64,
    depth: usize,
    context: &mut ImportContext<'a, 'input, K>,
) -> Result<(), Error> {
    if depth > MAX_SVG_DEPTH {
        return Err(svg_error(context.path_name, "SVG nesting exceeds 128 levels"));
    }
    let id = symbol.attribute("id").ok_or_else(|| svg_error(context.path_name, "<use> symbol target is missing its id"))?;
    if symbol.attribute("display") == Some("none") {
        return Ok(());
    }
    if let Some(transform) = symbol.attribute("transform") {
        let local = parse_transform(transform).map_err(|e| svg_error(context.path_name, &format!("<symbol> transform: {e}")))?;
        state.transform = compose(state.transform, local);
    }
    let style = computed_style(symbol, &state.inherited_style, &context.css_rules, K::PROPERTIES);
    if style.get("display").is_some_and(|value| value == "none") {
        return Ok(());
    }
    if let Some(visibility) = style.get("visibility") {
        state.visibility_hidden = visibility == "hidden" || visibility == "collapse";
    }
    if let Some(kind) = K::of(&style, symbol.attribute("class").unwrap_or("")) {
        state.kind = kind;
    }
    state.inherited_style = style.iter().filter(|(key, _)| key.as_str() != "display").map(|(key, value)| (key.clone(), value.clone())).collect();

    if let Some(raw_view_box) = symbol.attribute("viewBox") {
        let values = parse_number_list(raw_view_box)
            .filter(|values| values.len() == 4)
            .ok_or_else(|| svg_error(context.path_name, "<symbol> viewBox must contain four finite numbers"))?;
        let (min_x, min_y, vb_w, vb_h) = (values[0], values[1], values[2], values[3]);
        if values.iter().any(|value| value.abs() > 1.0e9) {
            return Err(svg_error(context.path_name, "<symbol> viewBox exceeds ±1e9"));
        }
        if vb_w <= 0.0 || vb_h <= 0.0 {
            return Err(svg_error(context.path_name, "<symbol> viewBox dimensions must be positive"));
        }
        let width = use_viewport_length(use_node, "width", vb_w, context.path_name)?;
        let height = use_viewport_length(use_node, "height", vb_h, context.path_name)?;
        if width == 0.0 || height == 0.0 {
            return Ok(());
        }
        let preserve = symbol.attribute("preserveAspectRatio").unwrap_or("xMidYMid meet");
        let viewport = preserve_viewbox_transform(min_x, min_y, vb_w, vb_h, width, height, preserve)
            .map_err(|message| svg_error(context.path_name, &format!("<symbol>: {message}")))?;
        if !viewport.as_coeffs().iter().all(|value| value.is_finite()) {
            return Err(svg_error(context.path_name, "<symbol> viewport transform is not finite"));
        }
        state.transform = compose(state.transform, kurbo::Affine::translate((x, y)));
        state.transform = compose(state.transform, viewport);
    } else {
        state.transform = compose(state.transform, kurbo::Affine::translate((x, y)));
    }

    context.reference_stack.push(id.to_string());
    let result = import_children(symbol, state, depth, context);
    context.reference_stack.pop();
    result
}

fn use_viewport_length(use_node: roxmltree::Node<'_, '_>, attribute: &str, fallback: f64, path_name: &str) -> Result<f64, Error> {
    let Some(raw) = use_node.attribute(attribute) else {
        return Ok(fallback);
    };
    let value = parse_number(raw).ok_or_else(|| svg_error(path_name, &format!("<use> {attribute} must be a unitless finite SVG length")))?;
    if value.abs() > 1.0e9 || (attribute != "x" && attribute != "y" && value < 0.0) {
        return Err(svg_error(path_name, &format!("<use> {attribute} is outside its supported range")));
    }
    Ok(value)
}

fn import_children<'a, 'input, K: Kind>(
    parent: roxmltree::Node<'a, 'input>,
    state: ImportState<K>,
    depth: usize,
    context: &mut ImportContext<'a, 'input, K>,
) -> Result<(), Error> {
    if depth > MAX_SVG_DEPTH {
        return Err(svg_error(context.path_name, "SVG nesting exceeds 128 levels"));
    }
    for child in parent.children().filter(|node| node.is_element()) {
        import_node(child, state.clone(), depth + 1, context)?;
    }
    Ok(())
}

fn geometry_for(node: roxmltree::Node<'_, '_>, path_name: &str) -> Result<Option<BezPath>, Error> {
    let tag = node.tag_name().name();
    let number = |name: &str, default: f64| -> Result<f64, Error> {
        match node.attribute(name) {
            Some(value) => parse_number(value).ok_or_else(|| svg_error(path_name, &format!("<{tag}> has invalid {name}={value:?}"))),
            None => Ok(default),
        }
    };
    let path = match tag {
        "svg" | "g" | "defs" | "symbol" | "clipPath" | "mask" | "pattern" | "title" | "desc" | "metadata" | "style" | "stop" | "namedview"
        | "guide" | "perspective" => return Ok(None),
        "path" => {
            let d = node.attribute("d").ok_or_else(|| svg_error(path_name, "<path> has no d attribute"))?;
            BezPath::from_svg(d).map_err(|e| svg_error(path_name, &format!("bad path data: {e}")))?
        }
        "rect" => {
            let x = number("x", 0.0)?;
            let y = number("y", 0.0)?;
            let w = number("width", 0.0)?;
            let h = number("height", 0.0)?;
            if w < 0.0 || h < 0.0 {
                return Err(svg_error(path_name, "<rect> width/height cannot be negative"));
            }
            let mut rx = number("rx", number("ry", 0.0)?)?;
            let mut ry = number("ry", rx)?;
            if rx < 0.0 || ry < 0.0 {
                return Err(svg_error(path_name, "<rect> corner radii cannot be negative"));
            }
            if rx == 0.0 {
                rx = ry;
            }
            if ry == 0.0 {
                ry = rx;
            }
            rounded_rect(x, y, w, h, rx.min(w / 2.0), ry.min(h / 2.0))
        }
        "circle" => {
            let cx = number("cx", 0.0)?;
            let cy = number("cy", 0.0)?;
            let r = number("r", 0.0)?;
            if r < 0.0 {
                return Err(svg_error(path_name, "<circle> radius cannot be negative"));
            }
            ellipse_path(cx, cy, r, r)
        }
        "ellipse" => {
            let cx = number("cx", 0.0)?;
            let cy = number("cy", 0.0)?;
            let rx = number("rx", 0.0)?;
            let ry = number("ry", 0.0)?;
            if rx < 0.0 || ry < 0.0 {
                return Err(svg_error(path_name, "<ellipse> radii cannot be negative"));
            }
            ellipse_path(cx, cy, rx, ry)
        }
        "line" => {
            let mut p = BezPath::new();
            p.move_to(kurbo::Point::new(number("x1", 0.0)?, number("y1", 0.0)?));
            p.line_to(kurbo::Point::new(number("x2", 0.0)?, number("y2", 0.0)?));
            p
        }
        "polyline" | "polygon" => {
            let points = node.attribute("points").ok_or_else(|| svg_error(path_name, &format!("<{tag}> has no points attribute")))?;
            points_path(points, tag == "polygon", path_name)?
        }
        "text" | "tspan" | "image" | "foreignObject" => {
            return Err(svg_error(path_name, &format!("<{tag}> is not supported yet")));
        }
        name if is_supported_non_drawable(name) => return Ok(None),
        _ => {
            return Err(svg_error(path_name, &format!("unsupported SVG element <{tag}>; import stopped to avoid silently dropping art")));
        }
    };
    Ok(Some(path))
}

fn is_supported_non_drawable(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "switch"
            | "marker"
            | "linearGradient"
            | "radialGradient"
            | "filter"
            | "feGaussianBlur"
            | "feOffset"
            | "feBlend"
            | "feColorMatrix"
            | "feComponentTransfer"
            | "feComposite"
            | "feConvolveMatrix"
            | "feDiffuseLighting"
            | "feDisplacementMap"
            | "feDistantLight"
            | "feDropShadow"
            | "feFlood"
            | "feFuncA"
            | "feFuncB"
            | "feFuncG"
            | "feFuncR"
            | "feImage"
            | "feMerge"
            | "feMergeNode"
            | "feMorphology"
            | "fePointLight"
            | "feSpecularLighting"
            | "feSpotLight"
            | "feTile"
            | "feTurbulence"
            | "animate"
            | "animateMotion"
            | "animateTransform"
            | "set"
            | "view"
            | "cursor"
            | "color-profile"
            | "font"
            | "font-face"
            | "glyph"
            | "missing-glyph"
    )
}

fn points_path(points: &str, closed: bool, path_name: &str) -> Result<BezPath, Error> {
    let values = parse_number_list(points).ok_or_else(|| svg_error(path_name, "invalid points list"))?;
    if values.len() % 2 != 0 {
        return Err(svg_error(path_name, "points list must contain coordinate pairs"));
    }
    if values.len() / 2 > 500_000 {
        return Err(svg_error(path_name, "a polygon exceeds the 500,000-point limit"));
    }
    let mut path = BezPath::new();
    let mut pairs = values.as_chunks::<2>().0.iter();
    if let Some(first) = pairs.next() {
        path.move_to(kurbo::Point::new(first[0], first[1]));
        for pair in pairs {
            path.line_to(kurbo::Point::new(pair[0], pair[1]));
        }
        if closed {
            path.close_path();
        }
    }
    Ok(path)
}

fn rounded_rect(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> BezPath {
    if rx <= 0.0 || ry <= 0.0 {
        let mut p = BezPath::new();
        p.move_to(kurbo::Point::new(x, y));
        p.line_to(kurbo::Point::new(x + w, y));
        p.line_to(kurbo::Point::new(x + w, y + h));
        p.line_to(kurbo::Point::new(x, y + h));
        p.close_path();
        return p;
    }
    let k = 0.552_284_749_830_793_6;
    let mut p = BezPath::new();
    p.move_to(kurbo::Point::new(x + rx, y));
    p.line_to(kurbo::Point::new(x + w - rx, y));
    p.curve_to(kurbo::Point::new(x + w - rx + k * rx, y), kurbo::Point::new(x + w, y + ry - k * ry), kurbo::Point::new(x + w, y + ry));
    p.line_to(kurbo::Point::new(x + w, y + h - ry));
    p.curve_to(kurbo::Point::new(x + w, y + h - ry + k * ry), kurbo::Point::new(x + w - rx + k * rx, y + h), kurbo::Point::new(x + w - rx, y + h));
    p.line_to(kurbo::Point::new(x + rx, y + h));
    p.curve_to(kurbo::Point::new(x + rx - k * rx, y + h), kurbo::Point::new(x, y + h - ry + k * ry), kurbo::Point::new(x, y + h - ry));
    p.line_to(kurbo::Point::new(x, y + ry));
    p.curve_to(kurbo::Point::new(x, y + ry - k * ry), kurbo::Point::new(x + rx - k * rx, y), kurbo::Point::new(x + rx, y));
    p.close_path();
    p
}

fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> BezPath {
    let k = 0.552_284_749_830_793_6;
    let mut p = BezPath::new();
    p.move_to(kurbo::Point::new(cx + rx, cy));
    p.curve_to(kurbo::Point::new(cx + rx, cy + k * ry), kurbo::Point::new(cx + k * rx, cy + ry), kurbo::Point::new(cx, cy + ry));
    p.curve_to(kurbo::Point::new(cx - k * rx, cy + ry), kurbo::Point::new(cx - rx, cy + k * ry), kurbo::Point::new(cx - rx, cy));
    p.curve_to(kurbo::Point::new(cx - rx, cy - k * ry), kurbo::Point::new(cx - k * rx, cy - ry), kurbo::Point::new(cx, cy - ry));
    p.curve_to(kurbo::Point::new(cx + k * rx, cy - ry), kurbo::Point::new(cx + rx, cy - k * ry), kurbo::Point::new(cx + rx, cy));
    p.close_path();
    p
}

const MAX_CSS_BYTES: usize = 1024 * 1024;
const MAX_CSS_RULES: usize = 10_000;
const MAX_CSS_PROPERTIES: usize = 32;
const SUPPORTED_STYLE_PROPERTIES: &[&str] = &["display", "visibility", "fill", "stroke", "stroke-width"];

/// Whether the cascade tracks `property`: the presentation properties above and [`Kind::PROPERTIES`].
fn supported(extra: &[&str], property: &str) -> bool {
    SUPPORTED_STYLE_PROPERTIES.contains(&property) || extra.contains(&property)
}

type CssPriority = (bool, u32, u32, u32, usize);
type CascadeWinners = std::collections::HashMap<String, (CssPriority, String)>;

#[derive(Clone)]
struct CssRule {
    selectors: Vec<SimpleSelector>,
    declarations: StyleMap,
    important: std::collections::HashSet<String>,
    order: usize,
}

#[derive(Clone)]
struct SimpleSelector {
    compounds: Vec<CompoundSelector>,
    combinators: Vec<char>,
    specificity: (u32, u32, u32),
}

#[derive(Clone)]
struct CompoundSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attributes: Vec<(String, Option<String>)>,
}

impl CompoundSelector {
    fn matches(&self, node: roxmltree::Node<'_, '_>) -> bool {
        if self.tag.as_deref().is_some_and(|tag| tag != "*" && tag != node.tag_name().name()) {
            return false;
        }
        if self.id.as_deref().is_some_and(|id| node.attribute("id") != Some(id)) {
            return false;
        }
        let classes: Vec<&str> = node.attribute("class").unwrap_or("").split_ascii_whitespace().collect();
        if self.classes.iter().any(|class| !classes.contains(&class.as_str())) {
            return false;
        }
        self.attributes.iter().all(|(name, value)| match value {
            Some(value) => node.attribute(name.as_str()) == Some(value.as_str()),
            None => node.attribute(name.as_str()).is_some(),
        })
    }
}

impl SimpleSelector {
    fn matches(&self, node: roxmltree::Node<'_, '_>) -> bool {
        let Some(last) = self.compounds.last() else {
            return false;
        };
        if !last.matches(node) {
            return false;
        }
        let mut current = node;
        for index in (0..self.combinators.len()).rev() {
            let previous = &self.compounds[index];
            match self.combinators[index] {
                '>' => {
                    let Some(parent) = current.parent_element() else {
                        return false;
                    };
                    if !previous.matches(parent) {
                        return false;
                    }
                    current = parent;
                }
                ' ' => {
                    let mut ancestor = current.parent_element();
                    let mut found = None;
                    while let Some(candidate) = ancestor {
                        if previous.matches(candidate) {
                            found = Some(candidate);
                            break;
                        }
                        ancestor = candidate.parent_element();
                    }
                    let Some(parent) = found else {
                        return false;
                    };
                    current = parent;
                }
                _ => return false,
            }
        }
        true
    }
}

fn parse_stylesheets(root: roxmltree::Node<'_, '_>, extra: &[&str], path_name: &str) -> Result<Vec<CssRule>, Error> {
    let mut rules = Vec::new();
    for node in root.descendants().filter(|node| node.is_element() && node.tag_name().name() == "style") {
        if let Some(media) = node.attribute("media")
            && !matches!(media.trim(), "" | "all" | "screen")
        {
            continue;
        }
        let stylesheet = node.text().unwrap_or("");
        if stylesheet.len() > MAX_CSS_BYTES {
            return Err(svg_error(path_name, "CSS stylesheet exceeds the 1 MiB limit"));
        }
        let stylesheet = strip_css_comments(stylesheet);
        for raw_rule in stylesheet.split('}') {
            let Some((raw_selector, raw_declarations)) = raw_rule.split_once('{') else {
                if !raw_rule.trim().is_empty() {
                    return Err(svg_error(path_name, "malformed CSS rule"));
                }
                continue;
            };
            let selector_cleaned = strip_css_comments(raw_selector);
            let selector_text = selector_cleaned.trim();
            if selector_text.is_empty() {
                continue;
            }
            let (declarations, important) = parse_declarations(raw_declarations, extra, path_name)?;
            for selector in split_selector_list(selector_text) {
                let selector =
                    parse_selector(selector.trim()).ok_or_else(|| svg_error(path_name, &format!("unsupported CSS selector {selector:?}")))?;
                rules.push(CssRule {
                    selectors: vec![selector],
                    declarations: declarations.clone(),
                    important: important.clone(),
                    order: rules.len(),
                });
                if rules.len() > MAX_CSS_RULES {
                    return Err(svg_error(path_name, "CSS exceeds the 10,000-rule limit"));
                }
            }
        }
    }
    Ok(rules)
}

fn strip_css_comments(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("/*") {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start + 2..].find("*/") else {
            return output;
        };
        rest = &rest[start + end + 4..];
    }
    output.push_str(rest);
    output
}

fn split_selector_list(raw: &str) -> Vec<&str> {
    let mut selectors = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in raw.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                selectors.push(raw[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    selectors.push(raw[start..].trim());
    selectors
}

fn parse_selector(raw: &str) -> Option<SimpleSelector> {
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut compound = String::new();
    let mut bracket_depth = 0usize;
    let mut pending_space = false;
    for ch in raw.chars() {
        if ch == '[' {
            bracket_depth += 1;
            compound.push(ch);
        } else if ch == ']' {
            bracket_depth = bracket_depth.saturating_sub(1);
            compound.push(ch);
        } else if bracket_depth == 0 && ch.is_ascii_whitespace() {
            if !compound.is_empty() {
                compounds.push(parse_compound(&compound)?);
                compound.clear();
                pending_space = true;
            }
        } else if bracket_depth == 0 && ch == '>' {
            if !compound.is_empty() {
                compounds.push(parse_compound(&compound)?);
                compound.clear();
            } else if compounds.is_empty() || combinators.len() == compounds.len() {
                return None;
            }
            if pending_space {
                pending_space = false;
            }
            combinators.push('>');
        } else {
            if pending_space {
                combinators.push(' ');
                pending_space = false;
            }
            compound.push(ch);
        }
    }
    if !compound.is_empty() {
        compounds.push(parse_compound(&compound)?);
    }
    if compounds.is_empty() || combinators.len() + 1 != compounds.len() {
        return None;
    }
    let specificity = compounds.iter().fold((0, 0, 0), |mut total, part| {
        total.0 += u32::from(part.id.is_some());
        total.1 += part.classes.len() as u32 + part.attributes.len() as u32;
        total.2 += u32::from(part.tag.as_deref().is_some_and(|tag| tag != "*"));
        total
    });
    Some(SimpleSelector { compounds, combinators, specificity })
}

fn parse_compound(raw: &str) -> Option<CompoundSelector> {
    let mut selector = CompoundSelector { tag: None, id: None, classes: Vec::new(), attributes: Vec::new() };
    let chars: Vec<char> = raw.chars().collect();
    let mut index = 0usize;
    if chars.first().is_some_and(|ch| *ch == '*' || ch.is_ascii_alphabetic() || *ch == '_') {
        let start = index;
        index += 1;
        while chars.get(index).is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '*')) {
            index += 1;
        }
        selector.tag = Some(chars[start..index].iter().collect());
    }
    while index < chars.len() {
        match chars[index] {
            '#' | '.' => {
                let marker = chars[index];
                index += 1;
                let start = index;
                while chars.get(index).is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':')) {
                    index += 1;
                }
                if start == index {
                    return None;
                }
                let value: String = chars[start..index].iter().collect();
                if marker == '#' {
                    if selector.id.replace(value).is_some() {
                        return None;
                    }
                } else {
                    selector.classes.push(value);
                }
            }
            '[' => {
                index += 1;
                let start = index;
                while chars.get(index).is_some_and(|ch| *ch != ']') {
                    index += 1;
                }
                if index == chars.len() {
                    return None;
                }
                let content: String = chars[start..index].iter().collect();
                index += 1;
                if let Some((name, value)) = content.split_once('=') {
                    let value = value.trim().trim_matches(['"', '\'']);
                    selector.attributes.push((name.trim().to_string(), Some(value.to_string())));
                } else {
                    selector.attributes.push((content.trim().to_string(), None));
                }
            }
            _ => return None,
        }
    }
    (selector.tag.is_some() || selector.id.is_some() || !selector.classes.is_empty() || !selector.attributes.is_empty()).then_some(selector)
}

fn parse_declarations(raw: &str, extra: &[&str], path_name: &str) -> Result<(StyleMap, std::collections::HashSet<String>), Error> {
    let mut declarations = StyleMap::new();
    let mut important = std::collections::HashSet::new();
    for declaration in strip_css_comments(raw).split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            if !declaration.trim().is_empty() {
                return Err(svg_error(path_name, "malformed CSS declaration"));
            }
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        if !supported(extra, property.as_str()) {
            continue;
        }
        if declarations.len() >= MAX_CSS_PROPERTIES {
            return Err(svg_error(path_name, "CSS rule has too many declarations"));
        }
        let (value, is_important) = strip_important(value.trim());
        if is_important {
            important.insert(property.clone());
        }
        declarations.insert(property, value.trim().to_string());
    }
    Ok((declarations, important))
}

fn computed_style(node: roxmltree::Node<'_, '_>, inherited: &StyleMap, rules: &[CssRule], extra: &[&str]) -> StyleMap {
    let mut result = inherited.clone();
    result.remove("display");
    let mut winners = CascadeWinners::new();
    // SVG presentation attributes participate below every author stylesheet rule.
    for (property, value) in style_properties(node, extra) {
        result.insert(property, value);
    }
    for rule in rules {
        for selector in &rule.selectors {
            if selector.matches(node) {
                let (ids, classes, tags) = selector.specificity;
                for (property, value) in &rule.declarations {
                    let priority = (rule.important.contains(property), ids, classes, tags, rule.order);
                    if winners.get(property).is_none_or(|(old_priority, _)| priority >= *old_priority) {
                        winners.insert(property.clone(), (priority, value.clone()));
                    }
                }
            }
        }
    }
    let mut important_winners = std::collections::HashSet::new();
    for (property, (priority, value)) in &winners {
        result.insert(property.clone(), value.clone());
        if priority.0 {
            important_winners.insert(property.clone());
        }
    }
    let mut inline_winners = std::collections::HashMap::<String, (bool, String)>::new();
    if let Some(inline) = node.attribute("style") {
        for declaration in inline.split(';') {
            let Some((property, value)) = declaration.split_once(':') else {
                continue;
            };
            let property = property.trim().to_ascii_lowercase();
            if !supported(extra, property.as_str()) {
                continue;
            }
            let (value, important) = strip_important(value.trim());
            if inline_winners.get(&property).is_none_or(|(old_important, _)| important || !old_important) {
                inline_winners.insert(property, (important, value.to_string()));
            }
        }
    }
    for (property, (important, value)) in inline_winners {
        if important || !important_winners.contains(&property) {
            result.insert(property, value);
        }
    }
    result
}

fn strip_important(value: &str) -> (&str, bool) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("!important") { (&trimmed[..trimmed.len() - "!important".len()], true) } else { (trimmed, false) }
}

fn style_properties(node: roxmltree::Node<'_, '_>, extra: &[&str]) -> StyleMap {
    let mut properties = StyleMap::new();
    for key in SUPPORTED_STYLE_PROPERTIES.iter().chain(extra) {
        if let Some(value) = node.attribute(*key) {
            properties.insert((*key).to_string(), value.trim().to_string());
        }
    }
    if let Some(inline) = node.attribute("style") {
        for declaration in inline.split(';') {
            if let Some((property, value)) = declaration.split_once(':') {
                let property = property.trim().to_ascii_lowercase();
                if supported(extra, property.as_str()) {
                    let (value, _) = strip_important(value.trim());
                    properties.insert(property, value.trim().to_string());
                }
            }
        }
    }
    properties
}

fn viewport_transform(root: roxmltree::Node<'_, '_>, path_name: &str) -> Result<(f64, f64, kurbo::Affine), Error> {
    if root.tag_name().name() == "symbol" {
        return Err(svg_error(path_name, "a standalone <symbol> cannot be an SVG root"));
    }
    let view_box = match root.attribute("viewBox") {
        Some(raw) => {
            let values =
                parse_number_list(raw).filter(|v| v.len() == 4).ok_or_else(|| svg_error(path_name, "viewBox must contain four finite numbers"))?;
            if values[2] <= 0.0 || values[3] <= 0.0 {
                return Err(svg_error(path_name, "viewBox width and height must be positive"));
            }
            Some((values[0], values[1], values[2], values[3]))
        }
        None => None,
    };
    let width_attr = root.attribute("width");
    let height_attr = root.attribute("height");
    let explicit_viewport = width_attr.is_some() || height_attr.is_some();
    let fallback = view_box.map(|(_, _, w, h)| (w, h)).unwrap_or((100.0, 150.0));
    let width = width_attr.map(svg_length_mm).transpose().map_err(|e| svg_error(path_name, &format!("root width: {e}")))?.unwrap_or(fallback.0);
    let height = height_attr.map(svg_length_mm).transpose().map_err(|e| svg_error(path_name, &format!("root height: {e}")))?.unwrap_or(fallback.1);
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 || width > 1.0e9 || height > 1.0e9 {
        return Err(svg_error(path_name, "viewport dimensions must be positive, finite, and no larger than 1e9 mm"));
    }
    let Some((min_x, min_y, vb_w, vb_h)) = view_box else {
        // Without viewBox, physical SVG user units are CSS px (96/in). For a
        // bare SVG fragment there is no viewport and paths remain mm-based.
        let sx = if explicit_viewport { 25.4 / 96.0 } else { 1.0 };
        let sy = sx;
        return Ok((width, height, kurbo::Affine::scale_non_uniform(sx, sy)));
    };
    let preserve = root.attribute("preserveAspectRatio").unwrap_or("xMidYMid meet");
    let transform =
        preserve_viewbox_transform(min_x, min_y, vb_w, vb_h, width, height, preserve).map_err(|message| svg_error(path_name, &message))?;
    if !transform.as_coeffs().iter().all(|value| value.is_finite()) {
        return Err(svg_error(path_name, "viewBox transform is not finite"));
    }
    Ok((width, height, transform))
}

fn preserve_viewbox_transform(
    min_x: f64,
    min_y: f64,
    vb_w: f64,
    vb_h: f64,
    width: f64,
    height: f64,
    preserve: &str,
) -> Result<kurbo::Affine, String> {
    let tokens: Vec<&str> = preserve.split_ascii_whitespace().collect();
    let align = tokens.first().copied().unwrap_or("xMidYMid");
    let mode = tokens.get(1).copied().unwrap_or("meet");
    if tokens.len() > 2 {
        return Err("preserveAspectRatio has unexpected trailing tokens".to_string());
    }
    if align == "none" {
        if tokens.len() > 1 {
            return Err("preserveAspectRatio `none` does not accept a meet/slice mode".to_string());
        }
        return Ok(kurbo::Affine::new([width / vb_w, 0.0, 0.0, height / vb_h, -min_x * width / vb_w, -min_y * height / vb_h]));
    }
    if !matches!(mode, "meet" | "slice") {
        return Err("preserveAspectRatio supports only meet, slice, or none".to_string());
    }
    let (x_align, y_align) = match align {
        "xMinYMin" => (0.0, 0.0),
        "xMidYMin" => (0.5, 0.0),
        "xMaxYMin" => (1.0, 0.0),
        "xMinYMid" => (0.0, 0.5),
        "xMidYMid" => (0.5, 0.5),
        "xMaxYMid" => (1.0, 0.5),
        "xMinYMax" => (0.0, 1.0),
        "xMidYMax" => (0.5, 1.0),
        "xMaxYMax" => (1.0, 1.0),
        _ => return Err("unsupported preserveAspectRatio alignment".to_string()),
    };
    let sx = width / vb_w;
    let sy = height / vb_h;
    let scale = if mode == "meet" { sx.min(sy) } else { sx.max(sy) };
    let tx = (width - vb_w * scale) * x_align - min_x * scale;
    let ty = (height - vb_h * scale) * y_align - min_y * scale;
    let transform = kurbo::Affine::new([scale, 0.0, 0.0, scale, tx, ty]);
    if !transform.as_coeffs().iter().all(|value| value.is_finite()) {
        return Err("preserveAspectRatio produces a non-finite transform".to_string());
    }
    Ok(transform)
}

fn svg_length_mm(raw: &str) -> Result<f64, String> {
    let raw = raw.trim();
    let split = raw.find(|c: char| !(c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))).unwrap_or(raw.len());
    let value = raw[..split].parse::<f64>().map_err(|_| format!("invalid length {raw:?}"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("length {raw:?} must be positive and finite"));
    }
    let unit = raw[split..].trim();
    let scale = match unit {
        "" | "px" => 25.4 / 96.0,
        "mm" => 1.0,
        "cm" => 10.0,
        "in" => 25.4,
        "pt" => 25.4 / 72.0,
        "pc" => 25.4 / 6.0,
        _ => return Err(format!("unsupported length unit {unit:?}")),
    };
    Ok(value * scale)
}

fn parse_number(raw: &str) -> Option<f64> {
    let value = raw.trim().parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn parse_number_list(raw: &str) -> Option<Vec<f64>> {
    let mut values = Vec::new();
    let mut token = String::new();
    let mut comma_pending = false;
    let mut saw_any = false;
    for ch in raw.chars() {
        if ch.is_ascii_whitespace() {
            if !token.is_empty() {
                values.push(parse_number(&token)?);
                token.clear();
                saw_any = true;
            }
        } else if ch == ',' {
            if !token.is_empty() {
                values.push(parse_number(&token)?);
                token.clear();
                saw_any = true;
                comma_pending = true;
            } else if !comma_pending && saw_any {
                // Whitespace before a comma is valid; consecutive commas or
                // a leading comma are not part of SVG's number-list grammar.
                comma_pending = true;
            } else {
                return None;
            }
        } else if (ch == '+' || ch == '-') && !token.is_empty() && !token.ends_with('e') && !token.ends_with('E') {
            values.push(parse_number(&token)?);
            token.clear();
            token.push(ch);
            saw_any = true;
            comma_pending = false;
        } else if ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'e' | 'E') {
            token.push(ch);
            comma_pending = false;
        } else {
            return None;
        }
        if values.len() > 200_000 {
            return None;
        }
    }
    if comma_pending {
        return None;
    }
    if !token.is_empty() {
        values.push(parse_number(&token)?);
    }
    Some(values)
}

fn parse_transform(raw: &str) -> Result<kurbo::Affine, String> {
    let mut rest = raw.trim();
    let mut result = kurbo::Affine::IDENTITY;
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            break;
        }
        let open = rest.find('(').ok_or_else(|| "expected '(' after transform name".to_string())?;
        let name = rest[..open].trim();
        let close = rest[open + 1..].find(')').map(|i| i + open + 1).ok_or_else(|| "unterminated transform".to_string())?;
        let values = parse_number_list(&rest[open + 1..close]).ok_or_else(|| "transform has invalid numeric arguments".to_string())?;
        let local = match (name, values.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => kurbo::Affine::new([*a, *b, *c, *d, *e, *f]),
            ("translate", [x]) => kurbo::Affine::translate((*x, 0.0)),
            ("translate", [x, y]) => kurbo::Affine::translate((*x, *y)),
            ("scale", [s]) => kurbo::Affine::scale(*s),
            ("scale", [x, y]) => kurbo::Affine::scale_non_uniform(*x, *y),
            ("rotate", [deg]) => kurbo::Affine::rotate(deg.to_radians()),
            ("rotate", [deg, cx, cy]) => {
                let to = kurbo::Affine::translate((*cx, *cy));
                let rot = kurbo::Affine::rotate(deg.to_radians());
                let from = kurbo::Affine::translate((-*cx, -*cy));
                compose(compose(to, rot), from)
            }
            ("skewX", [deg]) => kurbo::Affine::new([1.0, 0.0, deg.to_radians().tan(), 1.0, 0.0, 0.0]),
            ("skewY", [deg]) => kurbo::Affine::new([1.0, deg.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => return Err(format!("unsupported or malformed transform {name:?}")),
        };
        if !local.as_coeffs().iter().all(|v| v.is_finite()) {
            return Err("transform contains non-finite values".to_string());
        }
        result = compose(result, local);
        rest = &rest[close + 1..];
    }
    Ok(result)
}

/// Matrix composition `parent(local(point))` for kurbo's [a,b,c,d,e,f]
/// affine coefficient order.
fn compose(parent: kurbo::Affine, local: kurbo::Affine) -> kurbo::Affine {
    let [a, b, c, d, e, f] = parent.as_coeffs();
    let [g, h, i, j, k, l] = local.as_coeffs();
    kurbo::Affine::new([a * g + c * h, b * g + d * h, a * i + c * j, b * i + d * j, a * k + c * l + e, b * k + d * l + f])
}

fn transform_path(path: &BezPath, affine: kurbo::Affine) -> BezPath {
    let mut out = BezPath::new();
    for elem in path.elements() {
        match *elem {
            kurbo::PathEl::MoveTo(p) => out.move_to(affine * p),
            kurbo::PathEl::LineTo(p) => out.line_to(affine * p),
            kurbo::PathEl::QuadTo(c, p) => out.quad_to(affine * c, affine * p),
            kurbo::PathEl::CurveTo(c1, c2, p) => out.curve_to(affine * c1, affine * c2, affine * p),
            kurbo::PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

fn validate_import_path(path: &BezPath, path_name: &str) -> Result<(), Error> {
    if path.elements().len() > 1_000_000 {
        return Err(svg_error(path_name, "a shape exceeds the 1,000,000-segment limit"));
    }
    let valid = |p: kurbo::Point| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 1.0e9 && p.y.abs() <= 1.0e9;
    for element in path.elements() {
        let safe = match *element {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => valid(p),
            kurbo::PathEl::QuadTo(c, p) => valid(c) && valid(p),
            kurbo::PathEl::CurveTo(c1, c2, p) => valid(c1) && valid(c2) && valid(p),
            kurbo::PathEl::ClosePath => true,
        };
        if !safe {
            return Err(svg_error(path_name, "geometry coordinates must be finite and within ±1e9 mm"));
        }
    }
    Ok(())
}

fn svg_error(path_name: &str, message: &str) -> Error {
    Error(format!("{path_name}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape as _;

    /// A cutter's two kinds, set by `data-kind` or by class.
    #[derive(Debug, Clone, Copy, Default, PartialEq)]
    enum Tag {
        #[default]
        Cut,
        Engrave,
    }

    impl Kind for Tag {
        const PROPERTIES: &'static [&'static str] = &["data-kind"];

        fn of(style: &StyleMap, class: &str) -> Option<Tag> {
            match style.get("data-kind").map(String::as_str) {
                Some("cut") => Some(Tag::Cut),
                Some("engrave") => Some(Tag::Engrave),
                _ if class.contains("engrave") => Some(Tag::Engrave),
                _ => None,
            }
        }

        fn skips_unpainted(&self) -> bool {
            *self == Tag::Cut
        }
    }

    fn read(svg: &str) -> Imported<Tag> {
        import(svg, "test.svg").unwrap()
    }

    fn fails(svg: &str, with: &str) {
        match import::<Tag>(svg, "test.svg") {
            Err(Error(m)) => assert!(m.starts_with("test.svg: ") && m.contains(with), "{m}"),
            Ok(_) => panic!("{svg} was read"),
        }
    }

    fn bounds(imported: &Imported<Tag>, i: usize) -> kurbo::Rect {
        imported.shapes[i].path.bounding_box()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_fragment_is_read_in_millimetres_and_groups_are_walked() {
        let got = read(r#"<svg><g class="x"><path d="M 0 0 L 5 0 L 5 5 Z"/></g></svg>"#);
        assert_eq!(got.shapes.len(), 1);
        assert_eq!(got.shapes[0].path.to_svg(), "M0,0 L5,0 L5,5 Z");
        // The unit kind reads the same paths.
        assert_eq!(import::<()>("<path d=\"M 0 0 L 5 0 L 5 5 Z\"/>", "p.svg").unwrap().shapes[0].path.to_svg(), "M0,0 L5,0 L5,5 Z");
    }

    #[test]
    fn errors_name_the_file_and_the_problem() {
        fails("<path></path>", "d attribute");
        fails("<path d=\"M 0 0 Q\"/>", "");
        fails("<g transform=\"scale(nope)\"><path d=\"M0 0 L1 0 Z\"/></g>", "transform");
        fails(r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10">hello</text></svg>"#, "<text> is not supported");
        fails("<svg><g></svg>", "invalid XML");
    }

    #[test]
    fn reads_all_common_basic_shapes() {
        let got = read(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
          <rect x="1" y="2" width="10" height="8"/>
          <circle cx="20" cy="20" r="5"/>
          <ellipse cx="40" cy="20" rx="8" ry="4"/>
          <line x1="1" y1="30" x2="20" y2="30"/>
          <polyline points="1,40 10,45 20,40"/>
          <polygon points="30,40 40,40 35,50"/>
          <path d="M50 50 C55 45 60 55 65 50 Z"/>
        </svg>"#,
        );
        assert_eq!(got.shapes.len(), 7);
        assert!(got.shapes[0].path.elements().len() >= 5);
        assert!(got.shapes[1].path.elements().len() >= 6);
        assert_eq!((got.width_mm, got.height_mm), (100.0, 100.0));
    }

    #[test]
    fn nested_group_transforms_apply_and_the_kind_inherits() {
        let got = read(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
          <g transform="translate(10,20)" data-kind="engrave">
            <g transform="scale(2)"><rect x="1" y="2" width="3" height="4"/></g>
          </g>
          <rect class="engrave-me" width="1" height="1"/>
          <rect width="1" height="1"/>
        </svg>"#,
        );
        assert_eq!(got.shapes.iter().map(|s| s.kind).collect::<Vec<_>>(), [Tag::Engrave, Tag::Engrave, Tag::Cut]);
        let b = bounds(&got, 0);
        assert!(near(b.x0, 12.0) && near(b.y0, 24.0) && near(b.width(), 6.0), "{b:?}");
    }

    #[test]
    fn local_use_instances_take_x_y_transform_and_symbol_viewboxes() {
        let got = read(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
          <defs>
            <g id="shape" data-kind="engrave" transform="scale(2)">
              <rect x="1" y="2" width="3" height="4"/>
            </g>
            <symbol id="badge" viewBox="10 20 20 10" preserveAspectRatio="xMidYMid meet" data-kind="engrave">
              <rect x="10" y="20" width="20" height="10"/>
            </symbol>
          </defs>
          <use href="#shape" x="10" y="20" transform="translate(5,0)"/>
          <use xlink:href="#shape" x="30" y="5" xmlns:xlink="http://www.w3.org/1999/xlink"/>
          <use href="#badge" x="50" y="10" width="40" height="40"/>
        </svg>"##,
        );
        assert_eq!(got.shapes.len(), 3);
        assert!(got.shapes.iter().all(|s| s.kind == Tag::Engrave));
        let first = bounds(&got, 0);
        assert!(near(first.x0, 17.0) && near(first.y0, 24.0) && near(first.width(), 6.0), "{first:?}");
        let second = bounds(&got, 1);
        assert!(near(second.x0, 32.0) && near(second.y0, 9.0), "{second:?}");
        let badge = bounds(&got, 2);
        assert!(near(badge.x0, 50.0) && near(badge.y0, 20.0) && near(badge.width(), 40.0) && near(badge.height(), 20.0), "{badge:?}");
    }

    #[test]
    fn use_cycles_missing_and_external_references_are_errors() {
        for svg in [
            r##"<svg><defs><g id="a"><use href="#b"/></g><g id="b"><use href="#a"/></g></defs><use href="#a"/></svg>"##,
            r##"<svg><use href="#missing"/></svg>"##,
            r#"<svg><use href="other.svg#shape"/></svg>"#,
            r##"<svg><defs><text id="label">no</text></defs><use href="#label"/></svg>"##,
            r##"<svg><rect id="same"/><circle id="same"/></svg>"##,
            r##"<svg><defs><symbol id="cycle"><use href="#cycle"/></symbol></defs><use href="#cycle"/></svg>"##,
            r##"<svg><defs><symbol id="bad" viewBox="0 0 0 4"><rect width="2" height="2"/></symbol></defs><use href="#bad"/></svg>"##,
            r##"<svg><defs><symbol id="zero" viewBox="0 0 4 4"><rect width="2" height="2"/></symbol></defs><use href="#zero" width="-1"/></svg>"##,
        ] {
            fails(svg, "");
        }
    }

    #[test]
    fn a_viewbox_maps_onto_physical_units() {
        for preserve in ["xMidYMid meet extra", "notxMidYMid"] {
            fails(&format!(r#"<svg width="200mm" height="100mm" viewBox="0 0 100 100" preserveAspectRatio="{preserve}"/>"#), "");
        }
        let got = read(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="100mm"
          viewBox="0 0 100 100" preserveAspectRatio="none">
          <rect x="10" y="20" width="5" height="4"/>
        </svg>"#,
        );
        assert_eq!((got.width_mm, got.height_mm), (200.0, 100.0));
        let b = bounds(&got, 0);
        assert!(near(b.x0, 20.0) && near(b.y0, 20.0) && near(b.width(), 10.0) && near(b.height(), 4.0), "{b:?}");

        let px = read(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="96px" height="48px">
          <g transform="translate(10 0) scale(2)"><line x1="0" y1="0" x2="4" y2="0"/></g>
        </svg>"#,
        );
        assert!(near(px.width_mm, 25.4) && near(px.height_mm, 12.7));
        let b = bounds(&px, 0);
        assert!(near(b.x0, 10.0 * 25.4 / 96.0) && near(b.width(), 8.0 * 25.4 / 96.0), "{b:?}");
    }

    #[test]
    fn hidden_and_undisplayed_elements_are_left_out() {
        let got = read(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
          <g style="display:none"><rect width="10" height="10"/></g>
          <g style="visibility:hidden" data-kind="engrave">
            <line style="visibility:visible" x1="1" y1="2" x2="3" y2="4"/>
            <line x1="5" y1="6" x2="7" y2="8"/>
          </g>
        </svg>"#,
        );
        assert_eq!(got.shapes.len(), 1);
        assert_eq!(got.shapes[0].kind, Tag::Engrave);
    }

    #[test]
    fn an_unpainted_element_is_left_out_only_for_a_kind_that_asks() {
        let svg = |kind: &str| format!(r#"<svg><rect data-kind="{kind}" stroke="none" fill="none" width="2" height="2"/></svg>"#);
        assert_eq!(read(&svg("cut")).shapes.len(), 0);
        assert_eq!(read(&svg("engrave")).shapes.len(), 1);
        // Only one of the two set to none still draws.
        assert_eq!(read(r#"<svg><line fill="none" x1="0" y1="0" x2="1" y2="1"/></svg>"#).shapes.len(), 1);
    }

    #[test]
    fn the_css_cascade_follows_specificity_importance_and_the_kinds_own_property() {
        let got = read(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
          <style>
            /* supported selector subset */
            path { data-kind: cut; }
            .mark { data-kind: engrave; visibility: hidden; }
            g#source > path.mark { visibility: visible; }
            path.mark[id="first"] { display: none; }
            path.mark[data-kind="engrave"] { stroke: red; }
            .mark { data-kind: cut !important; }
          </style>
          <g id="source">
            <path id="first" class="mark" d="M1 1L2 1"/>
            <path id="second" class="mark" style="data-kind:engrave" d="M3 3L4 3"/>
          </g>
        </svg>"##,
        );
        assert_eq!(got.shapes.len(), 1);
        assert_eq!(got.shapes[0].kind, Tag::Cut);
        assert_eq!(bounds(&got, 0).x0, 3.0);
        // A property the kind doesn't list is not followed: `()` ignores `data-kind` entirely.
        assert_eq!(import::<()>(r#"<svg><path style="data-kind:engrave" d="M0 0L1 1"/></svg>"#, "p.svg").unwrap().shapes.len(), 1);
    }

    #[test]
    fn unsupported_selectors_and_oversized_stylesheets_are_errors() {
        fails(r##"<svg><style>path:hover { display:none; }</style><path d="M0 0L1 1"/></svg>"##, "");
        fails(&format!("<svg><style>{}</style></svg>", " ".repeat(MAX_CSS_BYTES + 1)), "");
    }

    #[test]
    fn nonfinite_geometry_and_malformed_number_lists_are_errors() {
        fails(r#"<svg viewBox="0 0 NaN 100"/>"#, "");
        fails(r#"<svg><polygon points="0,0,,1,1"/></svg>"#, "");
        fails(r#"<svg><rect x="1e400" width="10" height="10"/></svg>"#, "");
    }

    #[test]
    fn resource_limits_are_errors_not_panics() {
        fails(&format!("<svg>{}</svg>", " ".repeat(MAX_SVG_BYTES)), "16 MiB");
        let deep = format!("{}<path d=\"M0 0L1 0\"/>{}", "<g>".repeat(MAX_SVG_DEPTH + 2), "</g>".repeat(MAX_SVG_DEPTH + 2));
        fails(&deep, "nesting exceeds");
        fails(&format!("<svg>{}</svg>", "<g/>".repeat(MAX_SVG_NODES + 1)), "100,000-node");
    }

    #[test]
    fn deterministically_mutated_svg_never_panics_and_stays_in_range() {
        let seed = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><rect x="2" y="3" width="7" height="8"/></svg>"#;
        let bytes = seed.as_bytes();
        let mut state = 0x51_6f_9a_u64;
        for _ in 0..512 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let index = (state as usize) % bytes.len();
            let mut mutated = bytes.to_vec();
            mutated[index] = (state >> 32) as u8;
            if let Ok(text) = std::str::from_utf8(&mutated)
                && let Ok(got) = import::<Tag>(text, "mutated.svg")
            {
                assert!(got.width_mm.is_finite() && got.height_mm.is_finite());
                for shape in &got.shapes {
                    let b = shape.path.bounding_box();
                    assert!(b.x0.abs() <= 1.0e9 && b.x1.abs() <= 1.0e9 && b.y0.abs() <= 1.0e9 && b.y1.abs() <= 1.0e9, "{text}");
                }
            }
        }
    }
}
