//! Prints every polygon fill hit of a project file (layer name, granularity, shape, symmetry):
//! `cargo run -p craft-spp-reader --features dev-tools --example decals -- <project file>`.

use craft_spp_reader::model::{ProjectEffect, ProjectLayer};
use craft_spp_reader::project::ProjectFile;
use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        return Err("usage: decals <project file>".into());
    };
    let project = ProjectFile::open(&path)?;
    let mesh = craft_spp_reader::decode::assbin::AssbinGeometry::parse(&project.read(craft_spp_reader::project::paths::SCENE)?)?;
    for part in &mesh.parts {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in &part.positions {
            for ((low, high), v) in lo.iter_mut().zip(hi.iter_mut()).zip(p) {
                *low = low.min(*v);
                *high = high.max(*v);
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
        hits(&layer.name, &layer.content);
        if let Some(mask) = &layer.mask {
            hits(&layer.name, &mask.effects);
        }
        if let craft_spp_reader::model::LayerKind::Group(children) = &layer.kind {
            walk(children);
        }
    }
}

/// The polygon fill hits of the effect stack `stack` (and of the effect groups inside), under the name of the layer that owns them.
fn hits(layer: &str, stack: &[ProjectEffect]) {
    for effect in stack {
        match effect {
            ProjectEffect::PolygonFill(fill) => {
                println!("== {layer}: {} hits, {} unread", fill.hits.len(), fill.unread);
                for hit in &fill.hits {
                    println!("{hit:?}");
                }
            }
            ProjectEffect::Group(group) => hits(layer, &group.effects),
            _ => {}
        }
    }
}
