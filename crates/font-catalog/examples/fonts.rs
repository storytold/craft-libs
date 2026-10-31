//! Lists what the catalog finds on this computer, to check discovery by hand.
//!
//! ```text
//! cargo run -p craft-font-catalog --example fonts             # counts, folders, font store
//! cargo run -p craft-font-catalog --example fonts -- Arial     # a family's faces
//! cargo run -p craft-font-catalog --example fonts -- --watch   # report fonts installed or removed
//! cargo run -p craft-font-catalog --example fonts --release -- --time   # what rescans cost
//! ```

use std::time::{Duration, Instant};

use craft_font_catalog::{Catalog, font_dirs, font_store_dir};

fn main() {
    let catalog = Catalog::global();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let start = Instant::now();
    let families = catalog.families();
    let faces = catalog.faces();
    println!("{} families, {} faces, first scan {:?}", families.len(), faces.len(), start.elapsed());
    println!("font store: {:?}", font_store_dir(catalog.env()));
    for dir in font_dirs(catalog.env()).iter().filter(|d| d.is_dir()) {
        println!("folder: {}", dir.display());
    }
    let outside: Vec<_> =
        faces.iter().filter(|f| f.path.as_ref().is_some_and(|p| !font_dirs(catalog.env()).iter().any(|d| p.starts_with(d)))).collect();
    println!("{} faces outside the font folders (font service, store, registry):", outside.len());
    for f in outside.iter().take(40) {
        println!("  {} {} ({:?}) {}", f.family, f.style, f.source, f.path.as_deref().map(|p| p.display().to_string()).unwrap_or_default());
    }
    match args.first().map(String::as_str) {
        Some("--time") => {
            for _ in 0..3 {
                let start = Instant::now();
                catalog.rescan_if_changed();
                println!("rescan_if_changed: {:?}", start.elapsed());
            }
            for _ in 0..3 {
                let start = Instant::now();
                let faces = catalog.rescan();
                println!("rescan: {:?} ({faces} faces)", start.elapsed());
            }
            #[cfg(feature = "system")]
            for _ in 0..3 {
                let start = Instant::now();
                craft_font_catalog::system::refresh();
                println!("font service enumeration alone: {:?}", start.elapsed());
            }
        }
        Some("--watch") => {
            let _watch = catalog.watch(Duration::from_secs(2), |c| println!("changed: {} families", c.families().len()));
            println!("watching; install or remove a font, Ctrl+C to stop");
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        Some(name) => {
            for f in catalog.styles(name) {
                println!(
                    "{} / {}: weight {} width {} italic {} ps {:?} {:?} #{} {:?}",
                    f.family, f.style, f.weight, f.width, f.italic, f.postscript_name, f.path, f.index, f.source
                );
            }
            if let Some((f, m)) = catalog.resolve(name, args.get(1).map_or("Regular", String::as_str)) {
                println!("resolve: {} {} ({m:?})", f.family, f.style);
            }
        }
        None => {}
    }
}
