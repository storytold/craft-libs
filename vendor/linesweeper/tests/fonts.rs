use std::path::PathBuf;

use kurbo::BezPath;
use libtest_mimic::{Arguments, Failed, Trial};
use linesweeper::topology::Topology;
use linesweeper_util::outline_to_bezpath;
use skrifa::{FontRef, MetadataProvider};

fn main() {
    let args = Arguments::from_args();
    let tests = glyph_tests();

    libtest_mimic::run(&args, tests).exit();
}

fn glyph_tests() -> Vec<Trial> {
    let ws = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let paths = glob::glob(&format!("{ws}/tests/fonts/**/*.ttf")).unwrap();
    paths
        .into_iter()
        .flat_map(|p| {
            let p = p.unwrap();
            let name = p.strip_prefix(&ws).unwrap().display().to_string();
            font_glyph_tests(name, p)
        })
        .collect()
}

fn font_glyph_tests(name: String, path: PathBuf) -> Vec<Trial> {
    let data = std::fs::read(&path).unwrap();
    let font_ref = FontRef::new(&data).unwrap();
    let names = font_ref.glyph_names();
    font_ref
        .outline_glyphs()
        .iter()
        .zip(font_ref.outline_glyphs().iter().skip(1))
        .map(|((id1, outline1), (_id2, outline2))| {
            let glyph_name = names.get(id1).unwrap();
            let path1 = outline_to_bezpath(outline1);
            let path2 = outline_to_bezpath(outline2);
            Trial::test(format!("{name}-{glyph_name}"), || {
                test_outline(path1, path2)
            })
        })
        .collect()
}

fn test_outline(path1: BezPath, path2: BezPath) -> Result<(), Failed> {
    if path1.is_empty() || path2.is_empty() {
        return Ok(());
    }

    let top = Topology::from_paths_binary(&path1, &path2, 1e-3).unwrap();
    let _out_paths = top.compute_positions();
    Ok(())
}
