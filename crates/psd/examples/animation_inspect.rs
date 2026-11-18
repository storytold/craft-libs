//! Inspect a local PSD/PSB's animation structure without printing layer names, text, or metadata
//! values. No asset is bundled: pass a path explicitly. Does not decode pixels or modify the file.
//! `cargo run -p craft-psd --example animation_inspect -- /path/to/input.psd`

#[cfg(not(target_arch = "wasm32"))]
fn inspect() -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::BTreeMap;
    use std::io::Read;

    use craft_psd::animation::{frame_info, frame_sets, layer_frame_states};
    use craft_psd::metadata::parse_shmd;
    use craft_psd::{PsdFile, ResourceData};

    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: animation_inspect <input.psd|input.psb>")?;
    if args.next().is_some() {
        return Err("expected exactly one input path".into());
    }
    // Diagnostic tool, not the production large-file importer. Bound even a file growing while read.
    const MAX_BYTES: u64 = 128 * 1024 * 1024;
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES {
        return Err("expected a regular PSD/PSB no larger than 128 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("input exceeds the diagnostic tool's 128 MiB limit".into());
    }
    let psd = PsdFile::from_bytes(&bytes)?;
    println!("canvas={}x{} depth={} layers={}", psd.header.width, psd.header.height, psd.header.depth, psd.layers().len());
    let mut animations = 0;
    for resource in &psd.resources {
        match resource.parsed() {
            Some(Ok(ResourceData::Animation(animation))) => {
                animations += 1;
                let back = animation.to_bytes()?;
                if back != resource.data {
                    return Err("animation resource failed its byte-exact round trip".into());
                }
                println!("animation resource: {} records; byte-exact round trip", animation.records.len());
                for record in &animation.records {
                    if let Some(d) = record.descriptor() {
                        let d = d?;
                        let frames = frame_info(&d.descriptor)?;
                        let sets = frame_sets(&d.descriptor)?;
                        let mut delays = BTreeMap::new();
                        for frame in &frames {
                            *delays.entry(frame.delay_centiseconds).or_insert(0usize) += 1;
                        }
                        println!("  AnDs: {} definitions, {} frame sets", frames.len(), sets.len());
                        println!("  stored delays (centiseconds -> frame count): {delays:?}");
                        for set in sets {
                            let unresolved = set.frame_ids.iter().filter(|id| !frames.iter().any(|f| f.id == **id)).count();
                            println!("  frame set: {} ordered references, {unresolved} unresolved", set.frame_ids.len());
                        }
                    }
                }
            }
            Some(Ok(ResourceData::TimelineInfo(d))) => println!("document timeline: {} descriptor fields", d.descriptor.items.len()),
            Some(Err(e)) if resource.id == 1075 || (4000..=4999).contains(&resource.id) && resource.data.starts_with(b"mani") => return Err(e.into()),
            _ => {}
        }
    }
    let (mut states, mut tracks) = (0usize, 0usize);
    for layer in psd.layers() {
        for block in &layer.blocks {
            if block.key != *b"shmd" {
                continue;
            }
            let items = parse_shmd(&block.data)?;
            for item in &items {
                if let Some(d) = item.animation_descriptor() {
                    let d = d?;
                    if item.key == *b"mlst" {
                        layer_frame_states(&d.descriptor)?;
                        states += 1;
                    } else {
                        tracks += 1;
                    }
                }
            }
            if craft_psd::metadata::write_shmd(&items) != block.data {
                return Err("layer metadata failed its byte-exact round trip".into());
            }
        }
    }
    if psd.to_bytes()? != bytes {
        return Err("PSD/PSB failed its byte-exact round trip".into());
    }
    println!("animation resources={animations}; layer frame-state descriptors={states}; layer timeline descriptors={tracks}");
    println!("whole PSD/PSB: byte-exact round trip; input unchanged");
    Ok(())
}

fn main() -> std::process::ExitCode {
    #[cfg(not(target_arch = "wasm32"))]
    if let Err(e) = inspect() {
        eprintln!("animation inspection failed: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
