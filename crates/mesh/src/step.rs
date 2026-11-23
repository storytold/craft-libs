//! ISO 10303-21 (STEP / `.step` / `.stp`) boundary-representation reader and writer.
//!
//! Reads `CARTESIAN_POINT`, `POLY_LOOP`, and linear boundary-representation faces
//! (`VERTEX_POINT`, `EDGE_CURVE`, `ORIENTED_EDGE`, `EDGE_LOOP`, `FACE_OUTER_BOUND`) into a
//! [`Scene`] within [`Limits`]. Curved B-Rep edges that require analytic surface tessellation are
//! refused with [`Error::Unsupported`]. [`write`] serializes every mesh of a [`Scene`] into an
//! ISO 10303-21 ASCII stream of `CARTESIAN_POINT` and `POLY_LOOP` entities.
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

    let max_entities = limits.max_vertices.max(limits.max_triangles);
    let mut points: HashMap<u64, [f64; 3]> = HashMap::new();
    let mut poly_loops: Vec<Vec<u64>> = Vec::new();
    let mut vertex_points: HashMap<u64, u64> = HashMap::new();
    let mut edge_curves: HashMap<u64, (u64, u64, u64)> = HashMap::new();
    let mut oriented_edges: HashMap<u64, (u64, bool)> = HashMap::new();
    let mut edge_loops: Vec<(u64, Vec<u64>)> = Vec::new();
    let mut outer_bounds: HashMap<u64, bool> = HashMap::new();
    let mut curve_types: HashMap<u64, String> = HashMap::new();
    let mut entity_count = 0usize;

    for raw_line in text.split(';') {
        let line = raw_line.trim();
        let Some((id_str, entity_body)) = line.split_once('=') else { continue };
        let id_str = id_str.trim().trim_start_matches('#');
        let Ok(id) = id_str.parse::<u64>() else { continue };
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
                        poly_loops.push(refs);
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
            "FACE_OUTER_BOUND" => {
                if let (Some(loop_id), Some(orientation)) = (args.get(1).and_then(|arg| step_references(arg).first().copied()), args.get(2)) {
                    outer_bounds.insert(loop_id, orientation.trim().eq_ignore_ascii_case(".T."));
                }
            }
            _ => {}
        }
    }

    if points.is_empty() {
        return Err(malformed(KIND, "no 3D coordinate points found"));
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
                poly_loops.push(polygon);
            }
        }
    }

    if poly_loops.is_empty() {
        return Err(Error::Unsupported("STEP: B-Rep tessellation without polygon loops is not supported".into()));
    }

    let mut mesh = Mesh { name: "STEP".into(), ..Mesh::default() };
    let mut part = Part::default();
    let mut tri_count = 0usize;

    for polygon in poly_loops {
        let Some(&p0) = points.get(&polygon[0]) else {
            return Err(malformed(KIND, "polygon references a missing point"));
        };
        for i in 1..polygon.len() - 1 {
            if tri_count >= limits.max_triangles {
                return Err(too_large(KIND, format!("more than {} triangles", limits.max_triangles)));
            }
            if mesh.positions.len().saturating_add(3) > limits.max_vertices {
                return Err(too_large(KIND, format!("more than {} vertices", limits.max_vertices)));
            }
            let Some(&p1) = points.get(&polygon[i]) else {
                return Err(malformed(KIND, "polygon references a missing point"));
            };
            let Some(&p2) = points.get(&polygon[i + 1]) else {
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
    let mut loops = Vec::new();

    for (index, world) in scene.instances(limits)? {
        let Some(mesh) = scene.meshes.get(index) else { continue };
        let mirror = is_mirror(&world);
        for t in mesh.parts.iter().flat_map(|p| p.triangles_in(&mesh.positions)) {
            let p = t.map(|c| mesh.positions.get(c.position as usize).copied());
            let [Some(a), Some(b), Some(c)] = p else { return Err(malformed(KIND, "a corner index is out of range")) };
            let v = if mirror { [a, c, b] } else { [a, b, c] }.map(|pt| transform_point(&world, pt));
            let mut ids = [0u64; 3];
            for (slot, pt) in ids.iter_mut().zip(v) {
                let _ = writeln!(out, "#{entity_id}=CARTESIAN_POINT('',({:.6},{:.6},{:.6}));", pt[0], pt[1], pt[2]);
                *slot = entity_id;
                entity_id += 1;
            }
            loops.push(ids);
        }
    }

    for [p0, p1, p2] in loops {
        let _ = writeln!(out, "#{entity_id}=POLY_LOOP('',(#{p0},#{p1},#{p2}));");
        entity_id += 1;
    }

    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    Ok(out)
}
