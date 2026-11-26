//! ISO 10303-21 (STEP / `.step` / `.stp`) boundary-representation reader and writer.
//!
//! Reads `CARTESIAN_POINT`, `POLY_LOOP`, and linear boundary-representation faces
//! (`VERTEX_POINT`, `EDGE_CURVE`, `ORIENTED_EDGE`, `EDGE_LOOP`, `FACE_OUTER_BOUND`) into a
//! [`Scene`] within [`Limits`]. Curved B-Rep edges that require analytic surface tessellation are
//! refused with [`Error::Unsupported`]. [`write`] serializes every mesh of a [`Scene`] into an
//! ISO 10303-21 ASCII stream: each triangle becomes a `POLY_LOOP` inside a `FACE_BOUND` and `FACE`,
//! and the faces form an `OPEN_SHELL` in a `SHELL_BASED_SURFACE_MODEL` with its shape representation.
//!
//! Moved from `encoderCraft`'s `encodercraft-spatial-io::step` module.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::{Corner, Error, Limits, Mesh, Part, Result, Scene, face_normal, is_mirror, malformed, too_large, transform_point};

const KIND: &str = "STEP";

/// Does `bytes` look like an ISO 10303-21 STEP file (`ISO-10303-21` in the first 1024 bytes)?
pub fn is_step(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(1024)).unwrap_or(bytes);
    head.windows(12).any(|w| w == b"ISO-10303-21")
}

/// Read an ISO 10303-21 STEP file into a [`Scene`] of one mesh, within [`Limits`].
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Scene> {
    if bytes.len() > limits.max_data_bytes {
        return Err(too_large(KIND, format!("{} bytes exceeds the {}-byte data limit", bytes.len(), limits.max_data_bytes)));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| malformed(KIND, "not valid UTF-8"))?;
    if !text.contains("ISO-10303-21") || !text.contains("DATA;") {
        return Err(malformed(KIND, "not a valid ISO-10303-21 STEP file (missing `ISO-10303-21` or `DATA;`)"));
    }

    // A triangle of B-Rep needs its points plus vertices, edges, curves and a face: budget the
    // supporting entities separately from the decoded geometry, which is checked as it is built.
    let max_entities = limits.max_vertices.saturating_add(limits.max_triangles.saturating_mul(8));
    let mut points: HashMap<u64, [f64; 3]> = HashMap::new();
    let mut poly_loops: Vec<(u64, Vec<u64>)> = Vec::new();
    let mut vertex_points: HashMap<u64, u64> = HashMap::new();
    let mut edge_curves: HashMap<u64, (u64, u64, u64)> = HashMap::new();
    let mut oriented_edges: HashMap<u64, (u64, bool)> = HashMap::new();
    let mut edge_loops: Vec<(u64, Vec<u64>)> = Vec::new();
    let mut outer_bounds: HashMap<u64, bool> = HashMap::new();
    let mut outer_bounds_by_id: HashMap<u64, (u64, bool)> = HashMap::new();
    let mut plain_bounds: HashMap<u64, (u64, bool)> = HashMap::new();
    let mut faces: Vec<Vec<u64>> = Vec::new();
    let mut curve_types: HashMap<u64, String> = HashMap::new();
    let mut entity_count = 0usize;

    for_each_statement(text, |statement| {
        let line = statement.trim();
        let Some((id_str, entity_body)) = line.split_once('=') else { return Ok(()) };
        let id_str = id_str.trim().trim_start_matches('#');
        let Ok(id) = id_str.parse::<u64>() else { return Ok(()) };
        entity_count = entity_count.saturating_add(1);
        if entity_count > max_entities {
            return Err(too_large(KIND, format!("more than {max_entities} entities")));
        }
        let entity_body = entity_body.trim();
        let entity_name = entity_body.split_once('(').map(|(name, _)| name.trim().to_ascii_uppercase()).unwrap_or_default();
        curve_types.insert(id, entity_name.clone());
        let args = step_arguments(entity_body);
        match entity_name.as_str() {
            "CARTESIAN_POINT" => {
                let coords = args.get(1).map(|v| v.trim().trim_start_matches('(').trim_end_matches(')'));
                let Some(coords) = coords else {
                    return Err(malformed(KIND, "Cartesian point has no coordinates"));
                };
                let values = step_arguments(coords);
                if values.len() < 3 {
                    return Err(malformed(KIND, "Cartesian point must have three coordinates"));
                }
                let parse_coord = |v: &str| {
                    v.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|c| c.is_finite())
                        .ok_or_else(|| malformed(KIND, format!("invalid Cartesian coordinate: {v}")))
                };
                if points.len() >= limits.max_vertices {
                    return Err(too_large(KIND, format!("more than {} Cartesian points", limits.max_vertices)));
                }
                points.insert(id, [parse_coord(values[0])?, parse_coord(values[1])?, parse_coord(values[2])?]);
            }
            "POLY_LOOP" => {
                if let Some(arg) = args.get(1) {
                    let refs = step_references(arg);
                    if refs.len() > limits.max_face_corners {
                        return Err(too_large(KIND, format!("a polygon loop has more than {} corners", limits.max_face_corners)));
                    }
                    if refs.len() >= 3 {
                        poly_loops.push((id, refs));
                    }
                }
            }
            "VERTEX_POINT" => {
                if let Some(point_id) = args.get(1).and_then(|arg| step_references(arg).first().copied()) {
                    vertex_points.insert(id, point_id);
                }
            }
            "EDGE_CURVE" => {
                if let (Some(start), Some(end), Some(curve)) = (
                    args.get(1).and_then(|arg| step_references(arg).first().copied()),
                    args.get(2).and_then(|arg| step_references(arg).first().copied()),
                    args.get(3).and_then(|arg| step_references(arg).first().copied()),
                ) {
                    edge_curves.insert(id, (start, end, curve));
                }
            }
            "ORIENTED_EDGE" => {
                if let (Some(edge), Some(orientation)) = (args.get(3).and_then(|arg| step_references(arg).first().copied()), args.get(4)) {
                    oriented_edges.insert(id, (edge, orientation.trim().eq_ignore_ascii_case(".T.")));
                }
            }
            "EDGE_LOOP" => {
                if let Some(arg) = args.get(1) {
                    let refs = step_references(arg);
                    if refs.len() > limits.max_face_corners {
                        return Err(too_large(KIND, format!("an edge loop has more than {} edges", limits.max_face_corners)));
                    }
                    if refs.len() >= 3 {
                        edge_loops.push((id, refs));
                    }
                }
            }
            "FACE_BOUND" => {
                if let (Some(loop_id), Some(orientation)) = (args.get(1).and_then(|arg| step_references(arg).first().copied()), args.get(2)) {
                    plain_bounds.insert(id, (loop_id, orientation.trim().eq_ignore_ascii_case(".T.")));
                }
            }
            "FACE" | "ADVANCED_FACE" | "FACE_SURFACE" => {
                if let Some(arg) = args.get(1) {
                    faces.push(step_references(arg));
                }
            }
            "FACE_OUTER_BOUND" => {
                if let (Some(loop_id), Some(orientation)) = (args.get(1).and_then(|arg| step_references(arg).first().copied()), args.get(2)) {
                    let forward = orientation.trim().eq_ignore_ascii_case(".T.");
                    outer_bounds.insert(loop_id, forward);
                    outer_bounds_by_id.insert(id, (loop_id, forward));
                }
            }
            _ => {}
        }
        Ok(())
    })?;

    if points.is_empty() {
        return Err(malformed(KIND, "no 3D coordinate points found"));
    }

    // A face with more than one bound has holes (or several outer rings); filling them would be
    // wrong, so refuse. A face's referenced bound sets the orientation of its loop.
    if !faces.is_empty() {
        outer_bounds.clear();
        for face in &faces {
            let rings = face.iter().filter(|b| outer_bounds_by_id.contains_key(b) || plain_bounds.contains_key(b)).count();
            if rings > 1 {
                return Err(Error::Unsupported("STEP: faces with inner boundaries are not supported".into()));
            }
            if let [bound] = face.as_slice()
                && let Some(&(loop_id, forward)) = outer_bounds_by_id.get(bound).or_else(|| plain_bounds.get(bound))
            {
                outer_bounds.insert(loop_id, forward);
            }
        }
    } else {
        for &(loop_id, forward) in plain_bounds.values() {
            outer_bounds.entry(loop_id).or_insert(forward);
        }
    }

    let mut polygons: Vec<Vec<u64>> = Vec::with_capacity(poly_loops.len() + edge_loops.len());
    for (loop_id, mut poly) in poly_loops {
        if outer_bounds.get(&loop_id) == Some(&false) {
            poly.reverse();
        }
        polygons.push(poly);
    }

    if !edge_loops.is_empty() {
        for (loop_id, edge_loop) in edge_loops {
            let outer_orientation = outer_bounds.get(&loop_id).copied();
            if !outer_bounds.is_empty() && outer_orientation.is_none() {
                continue;
            }
            let mut polygon = Vec::with_capacity(edge_loop.len());
            let mut expected_start = None;
            let mut first_vertex = None;
            for oriented_id in edge_loop {
                let Some((edge_id, forward)) = oriented_edges.get(&oriented_id).copied() else {
                    return Err(malformed(KIND, "edge loop references a missing oriented edge"));
                };
                let Some((start_vertex, end_vertex, curve_id)) = edge_curves.get(&edge_id).copied() else {
                    return Err(malformed(KIND, "oriented edge references a missing edge curve"));
                };
                if curve_types.get(&curve_id).is_none_or(|kind| kind != "LINE") {
                    return Err(Error::Unsupported("STEP: curved edges require analytic surface tessellation".into()));
                }
                let (start_vertex, end_vertex) = if forward { (start_vertex, end_vertex) } else { (end_vertex, start_vertex) };
                if expected_start.is_some_and(|expected| expected != start_vertex) {
                    return Err(malformed(KIND, "edge loop is not connected in declared order"));
                }
                first_vertex.get_or_insert(start_vertex);
                expected_start = Some(end_vertex);
                let point_id = vertex_points.get(&start_vertex).copied().ok_or_else(|| malformed(KIND, "edge references a missing vertex point"))?;
                polygon.push(point_id);
            }
            if expected_start != first_vertex {
                return Err(malformed(KIND, "edge loop is not closed"));
            }
            if outer_orientation == Some(false) {
                polygon.reverse();
            }
            if polygon.len() >= 3 {
                polygons.push(polygon);
            }
        }
    }

    if polygons.is_empty() {
        return Err(Error::Unsupported("STEP: B-Rep tessellation without polygon loops is not supported".into()));
    }

    let mut mesh = Mesh { name: "STEP".into(), ..Mesh::default() };
    let mut part = Part::default();
    let mut tri_count = 0usize;

    for polygon in polygons {
        let mut ring = Vec::with_capacity(polygon.len());
        for id in &polygon {
            let Some(&p) = points.get(id) else {
                return Err(malformed(KIND, "polygon references a missing point"));
            };
            ring.push(p);
        }
        // Triangulate in the declared winding; a fan is only right for convex polygons.
        let corners =
            (0..ring.len()).filter_map(|i| u32::try_from(i).ok()).map(|position| Corner { position, normal: crate::NONE, uv: crate::NONE }).collect();
        let face = Part { corners, face_sizes: vec![ring.len() as u32], ..Part::default() };
        for [c0, c1, c2] in face.triangles_in(&ring) {
            if tri_count >= limits.max_triangles {
                return Err(too_large(KIND, format!("more than {} triangles", limits.max_triangles)));
            }
            if mesh.positions.len().saturating_add(3) > limits.max_vertices {
                return Err(too_large(KIND, format!("more than {} vertices", limits.max_vertices)));
            }
            let tri = [c0, c1, c2].map(|c| ring.get(c.position as usize).copied());
            let [Some(p0), Some(p1), Some(p2)] = tri else {
                return Err(malformed(KIND, "polygon references a missing point"));
            };
            let (Ok(base), Ok(normal_idx)) = (u32::try_from(mesh.positions.len()), u32::try_from(mesh.normals.len())) else {
                return Err(too_large(KIND, "more vertices than a 32-bit index holds"));
            };
            let n = face_normal([p0, p1, p2]).unwrap_or([0.0, 0.0, 1.0]);
            mesh.normals.push(n);
            mesh.positions.extend([p0, p1, p2]);
            for k in 0..3u32 {
                part.corners.push(Corner { position: base + k, normal: normal_idx, uv: crate::NONE });
            }
            tri_count += 1;
        }
    }

    if part.corners.is_empty() {
        return Err(malformed(KIND, "failed to tessellate 3D geometry from STEP loops"));
    }
    mesh.parts.push(part);
    Ok(Scene::from_meshes(vec![mesh], Vec::new()))
}

/// Stream the exchange structure into `on_statement` at each `;` outside a quoted string and
/// outside a `/* ... */` comment, dropping comments and empty statements (ISO 10303-21 §5.3, §6.4).
fn for_each_statement(text: &str, mut on_statement: impl FnMut(&str) -> Result<()>) -> Result<()> {
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            current.push(c);
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    current.push('\'');
                    chars.next();
                } else {
                    in_string = false;
                }
            }
            continue;
        }
        match c {
            '\'' => {
                in_string = true;
                current.push(c);
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut closed = false;
                while let Some(d) = chars.next() {
                    if d == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return Err(malformed(KIND, "unterminated comment"));
                }
                current.push(' ');
            }
            ';' => {
                if !current.trim().is_empty() {
                    on_statement(&current)?;
                }
                current.clear();
            }
            other => current.push(other),
        }
    }
    if in_string {
        return Err(malformed(KIND, "unterminated string"));
    }
    if !current.trim().is_empty() {
        on_statement(&current)?;
    }
    Ok(())
}

fn step_arguments(input: &str) -> Vec<&str> {
    let input = input.split_once('(').map(|(_, arguments)| arguments.rsplit_once(')').map_or(arguments, |(body, _)| body)).unwrap_or(input);
    let bytes = input.as_bytes();
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut index = 0usize;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'\'' {
            if in_string && bytes.get(index + 1) == Some(&b'\'') {
                index = index.saturating_add(2);
                continue;
            }
            in_string = !in_string;
        } else if !in_string {
            match byte {
                b'(' => depth = depth.saturating_add(1),
                b')' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => {
                    if let Some(arg) = input.get(start..index) {
                        args.push(arg.trim());
                    }
                    start = index.saturating_add(1);
                }
                _ => {}
            }
        }
        index = index.saturating_add(1);
    }
    if let Some(arg) = input.get(start..) {
        args.push(arg.trim());
    }
    args
}

fn step_references(input: &str) -> Vec<u64> {
    let bytes = input.as_bytes();
    let mut refs = Vec::new();
    let mut index = 0usize;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'#' {
            let digits_start = index.saturating_add(1);
            let mut digits_end = digits_start;
            while bytes.get(digits_end).is_some_and(u8::is_ascii_digit) {
                digits_end = digits_end.saturating_add(1);
            }
            if let Some(digits) = input.get(digits_start..digits_end)
                && let Ok(id) = digits.parse()
            {
                refs.push(id);
            }
            index = digits_end;
        } else {
            index = index.saturating_add(1);
        }
    }
    refs
}

/// Serialize every mesh of `scene`, placed by its nodes, into an ISO 10303-21 STEP ASCII string.
pub fn write(scene: &Scene, name: &str, limits: &Limits) -> Result<String> {
    let clean_name: String = name.chars().filter(|c| c.is_ascii_graphic() && *c != '\'').collect();
    let clean_name = if clean_name.is_empty() { "model" } else { &clean_name };

    let mut out = String::with_capacity(4096);
    out.push_str("ISO-10303-21;\nHEADER;\n");
    out.push_str("FILE_DESCRIPTION(('Craft 3D CAD Export'),'2;1');\n");
    let _ = writeln!(out, "FILE_NAME('{clean_name}.step','2026-11-08',('Craft'),('storytold'),'','','');");
    out.push_str("FILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));\nENDSEC;\nDATA;\n");

    let mut entity_id: u64 = 10;
    let mut faces = Vec::new();
    let mut triangles = 0usize;
    // The shape's origin point is a Cartesian point too, and counts against the vertex limit.
    let mut points = 1usize;

    for (index, world) in scene.instances(limits)? {
        let Some(mesh) = scene.meshes.get(index) else { continue };
        let mirror = is_mirror(&world);
        for t in mesh.parts.iter().flat_map(|p| p.triangles_in(&mesh.positions)) {
            triangles = triangles.saturating_add(1);
            points = points.saturating_add(3);
            if triangles > limits.max_triangles {
                return Err(too_large(KIND, format!("more than {} triangles", limits.max_triangles)));
            }
            if points > limits.max_vertices {
                return Err(too_large(KIND, format!("more than {} Cartesian points", limits.max_vertices)));
            }
            let p = t.map(|c| mesh.positions.get(c.position as usize).copied());
            let [Some(a), Some(b), Some(c)] = p else { return Err(malformed(KIND, "a corner index is out of range")) };
            let v = if mirror { [a, c, b] } else { [a, b, c] }.map(|pt| transform_point(&world, pt));
            let mut ids = [0u64; 3];
            for (slot, pt) in ids.iter_mut().zip(v) {
                let _ = writeln!(out, "#{entity_id}=CARTESIAN_POINT('',({},{},{}));", real(pt[0]), real(pt[1]), real(pt[2]));
                *slot = entity_id;
                entity_id += 1;
            }
            let [p0, p1, p2] = ids;
            let (poly, bound, face) = (entity_id, entity_id + 1, entity_id + 2);
            let _ = writeln!(out, "#{poly}=POLY_LOOP('',(#{p0},#{p1},#{p2}));");
            let _ = writeln!(out, "#{bound}=FACE_BOUND('',#{poly},.T.);");
            let _ = writeln!(out, "#{face}=FACE('',(#{bound}));");
            entity_id += 3;
            faces.push(face);
        }
    }

    // The faces form an open shell inside a shell-based surface model, placed in a 3D context.
    let list = faces.iter().map(|f| format!("#{f}")).collect::<Vec<_>>().join(",");
    let [shell, model, origin, z, x, axis, length, angle, solid, accuracy, context, shape] = std::array::from_fn(|i| entity_id + i as u64);
    let _ = writeln!(out, "#{shell}=OPEN_SHELL('',({list}));");
    let _ = writeln!(out, "#{model}=SHELL_BASED_SURFACE_MODEL('{clean_name}',(#{shell}));");
    let _ = writeln!(out, "#{origin}=CARTESIAN_POINT('',(0.,0.,0.));");
    let _ = writeln!(out, "#{z}=DIRECTION('',(0.,0.,1.));");
    let _ = writeln!(out, "#{x}=DIRECTION('',(1.,0.,0.));");
    let _ = writeln!(out, "#{axis}=AXIS2_PLACEMENT_3D('',#{origin},#{z},#{x});");
    let _ = writeln!(out, "#{length}=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT($,.METRE.));");
    let _ = writeln!(out, "#{angle}=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));");
    let _ = writeln!(out, "#{solid}=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());");
    let _ = writeln!(out, "#{accuracy}=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-9),#{length},'distance_accuracy_value','');");
    let _ = writeln!(
        out,
        "#{context}=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{accuracy}))GLOBAL_UNIT_ASSIGNED_CONTEXT((#{length},#{angle},#{solid}))REPRESENTATION_CONTEXT('Context','3D'));"
    );
    let _ = writeln!(out, "#{shape}=SHAPE_REPRESENTATION('{clean_name}',(#{model},#{axis}),#{context});");

    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    Ok(out)
}

/// A STEP REAL: shortest decimal that parses back to `v`, always with a `.` and, when scientific,
/// an `E` exponent (`1.E-7`).
fn real(v: f64) -> String {
    let s = format!("{v:e}");
    let (mantissa, exponent) = s.split_once('e').unwrap_or((&s, "0"));
    let mantissa = if mantissa.contains('.') { mantissa.to_string() } else { format!("{mantissa}.") };
    if exponent == "0" { mantissa } else { format!("{mantissa}E{exponent}") }
}
