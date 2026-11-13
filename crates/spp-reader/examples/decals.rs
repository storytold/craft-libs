//! Prints every polygon fill hit of a project file (layer name, granularity, shape, symmetry):
//! `cargo run -p texturecraft-import-reader --features dev-tools --example decals -- <project file>`.

use std::error::Error;
use std::path::PathBuf;
use texturecraft_import_reader::model::ProjectLayer;
use texturecraft_import_reader::project::ProjectFile;

fn main() -> Result<(), Box<dyn Error>> {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        return Err("usage: decals <project file>".into());
    };
    let project = ProjectFile::open(&path)?;
    let mesh = texturecraft_import_reader::decode::assbin::AssbinGeometry::parse(&project.read(texturecraft_import_reader::project::paths::SCENE)?)?;
    for part in &mesh.parts {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in &part.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        println!("part {} {} vertices {} triangles, box {lo:?}..{hi:?}", part.name, part.positions.len(), part.triangles.len());
    }
    let (document, _) = project.model()?;
    for set in &document.texture_sets {
        walk(&set.layers);
    }
    Ok(())
}

fn walk(layers: &[ProjectLayer]) {
    for layer in layers {
        let text = format!("{layer:?}");
        if text.contains("DecalHit") {
            println!("== {} {}", layer.name, text);
        }
        if let texturecraft_import_reader::model::LayerKind::Group(children) = &layer.kind {
            walk(children);
        }
    }
}
