//! Hostile-input hammer: truncation, bit flips and extreme field values must never panic or hang.

use super::*;

fn exercise(d: &[u8]) {
    let Ok(f) = CompoundFile::open(d) else { return };
    let _ = f.root();
    let _ = f.sector_size();
    for id in 0..2000 {
        if f.entry(id).is_none() {
            break;
        }
        let _ = f.read(id);
        let _ = f.child(id, "s0");
    }
    let _ = f.read(usize::MAX);
    let _ = f.find("Header-2/Content-3b03/s4");
    let _ = f.read_path("Mobs-19010/properties");
}

#[test]
fn hostile_input_never_panics() {
    for (i, v) in [Version::V3, Version::V4].into_iter().enumerate() {
        let (bytes, _) = sample(v, 5);
        craft_media_testkit::hostile::hammer(&format!("cfb seed {i}"), &bytes, 15_000, exercise);
        // The header and directory are where the structure lives: hammer a prefix harder.
        let head = &bytes[..bytes.len().min(2048)];
        craft_media_testkit::hostile::hammer(&format!("cfb head {i}"), head, 15_000, exercise);
    }
}
