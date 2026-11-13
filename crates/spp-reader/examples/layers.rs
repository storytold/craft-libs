//! Prints the texture sets and layer tree of a project file: `cargo run -p texturecraft-import-reader --features dev-tools --example layers -- <project file>`.

use std::error::Error;
use std::path::PathBuf;
use texturecraft_import_reader::decode::assbin::AssbinGeometry;
use texturecraft_import_reader::model::{LayerKind, ProjectLayer};
use texturecraft_import_reader::project::{ProjectFile, paths};

fn main() -> Result<(), Box<dyn Error>> {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        return Err("usage: layers <project file>".into());
    };
    let project = ProjectFile::open(&path)?;
    let mesh = AssbinGeometry::parse(&project.read(paths::SCENE)?)?;
    println!("{} mesh parts, materials {:?}", mesh.parts.len(), mesh.materials);
    let (document, report) = project.model()?;
    println!("document format {}, {} classes kept raw", document.format_version, report.raw_classes().len());
    for set in &document.texture_sets {
        println!("texture set \"{}\" ({} channels)", set.name, set.channels.len());
        print_layers(&set.layers, 1);
    }
    Ok(())
}

/// Top of the stack first, as the reference app's layer panel shows it.
fn print_layers(layers: &[ProjectLayer], depth: usize) {
    for layer in layers.iter().rev() {
        let kind = match &layer.kind {
            LayerKind::Fill => "fill",
            LayerKind::Paint => "paint",
            LayerKind::Group(_) => "folder",
            LayerKind::Other(class) => class.as_str(),
        };
        let mask = if layer.mask.is_some() { ", masked" } else { "" };
        let hidden = if layer.visible { "" } else { ", hidden" };
        println!("{}{} [{kind}{mask}{hidden}] {} effects", "  ".repeat(depth), layer.name, layer.content.len());
        if let LayerKind::Group(children) = &layer.kind {
            print_layers(children, depth + 1);
        }
    }
}
