//! Developer tool: looks inside a project file or a graph package, the way the importer sees it.
//!
//! ```text
//! cargo run --release -p craft-spp-reader --features dev-tools --example import_inspect -- <command> …
//!
//! tree <project>                               every group and dataset in the file
//! doc <project> [dataset] [--full]             the document (or another object dataset) as indented text
//! model <project>                              the layer stacks as a tree, with the model report
//! compiled-graphs <project|package|dir> [graph] [--ir|--stats]
//!                                              compiled graphs of the packages as text; --ir: decoded nodes;
//!                                              --stats: op and expression decoding counts over all of them
//! decompile <project|package> <graph> <out>    a compiled graph as graph text for the graph runtime
//! ```
//!
//! A package is a file with the extension [`PACKAGE_EXT`]; anything else is opened as a project file. Its output is the
//! same as the original development tool's for these commands, so the two can be compared byte for byte.

mod model_text;

use craft_spp_reader::container::hdf5::EntryKind;
use craft_spp_reader::decompile::find_and_decompile;
use craft_spp_reader::format_names::{PACKAGE_EXT, PACKAGE_SUFFIX};
use craft_spp_reader::inspect::document_dump::LIST_PREVIEW;
use craft_spp_reader::inspect::{DocumentDump, compiled_graph_dump};
use craft_spp_reader::project::{ProjectFile, paths};
use model_text::ModelText;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: import_inspect tree|doc|model|compiled-graphs|decompile … (see the source header for details)";

/// The size decompiled graphs are written for.
const DECOMPILE_SIZE: usize = 512;

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let flags: Vec<&str> = raw.iter().map(String::as_str).filter(|a| a.starts_with("--")).collect();
    let words: Vec<&str> = raw.iter().map(String::as_str).filter(|a| !a.starts_with("--")).collect();
    let has = |f: &str| flags.contains(&f);
    let result = match words.as_slice() {
        ["tree", path] => tree(path),
        ["doc", path, rest @ ..] if rest.len() <= 1 => doc(path, rest.first().copied(), has("--full")),
        ["model", path] => model(path),
        ["compiled-graphs", path, rest @ ..] if rest.len() <= 1 => {
            compiled_graphs(path, rest.first().copied().unwrap_or(""), has("--ir"), has("--stats"))
        }
        ["decompile", path, graph, out] => decompile(path, graph, out),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn err(e: &dyn std::fmt::Display) -> String {
    e.to_string()
}

fn open(path: &str) -> Result<ProjectFile, String> {
    ProjectFile::open(Path::new(path)).map_err(|e| err(&e))
}

/// Every group and dataset in the file.
fn tree(path: &str) -> Result<(), String> {
    let project = open(path)?;
    for e in project.layout().entries() {
        match e.kind {
            EntryKind::Group => println!("{}/", e.path),
            EntryKind::Dataset { len } => println!("{:<90} {len:>12}", e.path),
        }
    }
    Ok(())
}

/// The document (or the dataset given) as indented text; `full` shows every list entry.
fn doc(path: &str, dataset: Option<&str>, full: bool) -> Result<(), String> {
    let project = open(path)?;
    let data = project.read(dataset.unwrap_or(paths::DOCUMENT)).map_err(|e| err(&e))?;
    let preview = if full { usize::MAX } else { LIST_PREVIEW };
    print!("{}", DocumentDump::render_with(&data, preview).map_err(|e| err(&e))?);
    Ok(())
}

/// The layer stacks as a tree, with the model report.
fn model(path: &str) -> Result<(), String> {
    let project = open(path)?;
    let (document, report) = project.model().map_err(|e| err(&e))?;
    print!("{}", ModelText { document: &document, report: &report });
    Ok(())
}

fn is_package(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == PACKAGE_EXT)
}

/// The packages at `path` with their names: every one in a folder, one file, or those a project embeds.
fn graph_packages(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut packages: Vec<(String, Vec<u8>)> = Vec::new();
    if path.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(path).map_err(|e| err(&e))?.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for f in files.iter().filter(|f| is_package(f)) {
            packages.push((f.display().to_string(), std::fs::read(f).map_err(|e| err(&e))?));
        }
    } else if is_package(path) {
        packages.push((path.display().to_string(), std::fs::read(path).map_err(|e| err(&e))?));
    } else {
        let project = ProjectFile::open(path).map_err(|e| err(&e))?;
        for r in project.resources().iter().filter(|r| r.file_name.ends_with(PACKAGE_SUFFIX)) {
            packages.push((r.file_name.clone(), project.read(&r.path).map_err(|e| err(&e))?));
        }
    }
    Ok(packages)
}

/// The compiled graphs whose name contains `filter` as text (`ir`: decoded nodes), or the decoding counts (`stats`).
fn compiled_graphs(path: &str, filter: &str, ir: bool, stats: bool) -> Result<(), String> {
    let packages = graph_packages(Path::new(path))?;
    if stats {
        print!("{}", compiled_graph_dump::stats(packages.iter().map(|(n, d)| (n.as_str(), d.as_slice()))));
        return Ok(());
    }
    let mut out = String::new();
    for (name, data) in &packages {
        let r = if ir { compiled_graph_dump::ir_package(data, filter, &mut out) } else { compiled_graph_dump::dump_package(data, filter, &mut out) };
        if let Err(e) = r {
            out.push_str(&format!("!! {name}: {e}\n"));
        }
    }
    print!("{out}");
    Ok(())
}

/// Compiled graph `graph` of a package (or of the packages a project embeds) as graph text, written to `out`.
fn decompile(path: &str, graph: &str, out: &str) -> Result<(), String> {
    let packages: Vec<Vec<u8>> = graph_packages(Path::new(path))?.into_iter().map(|(_, d)| d).collect();
    let found = find_and_decompile(&packages, graph, DECOMPILE_SIZE).map_err(|e| err(&e))?;
    let Some(d) = found else { return Err(format!("no graph named {graph}")) };
    std::fs::write(out, d.text.as_bytes()).map_err(|e| err(&e))?;
    for w in &d.warnings {
        eprintln!("warning: {w}");
    }
    println!("wrote {out}: images {:?}, outputs {:?}, {} warnings", d.images, d.outputs, d.warnings.len());
    Ok(())
}
