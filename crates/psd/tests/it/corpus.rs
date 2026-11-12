//! Corpus test (feature `corpus`): iterates
//! `corpus/psd/**/*.{psd,psb}` and `corpus/psd-tools/**/*.{psd,psb}` at the workspace root,
//! or under `CRAFT_PSD_CORPUS` (PhotoCraft's `cargo xtask corpus --all` fetches and verifies it). A missing corpus fails.
#![cfg(feature = "corpus")]

use std::path::{Path, PathBuf};

use craft_psd::PsdFile;

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("psd") || e.eq_ignore_ascii_case("psb")) {
            out.push(p);
        }
    }
}

/// The existing pinned ag-psd corpus includes public animation cases. Exercise typed syntax
/// against those without adding personal files or new corpus pins.
#[test]
fn corpus_animation_syntax() {
    use craft_psd::ResourceData;
    use craft_psd::animation::{frame_info, frame_sets, layer_frame_states};
    use craft_psd::metadata::parse_shmd;

    let root = crate::corpus_dir("psd/ag-psd/read-write");
    for (case, delay) in [("animation-frame", Some(30)), ("animation-offset", Some(50)), ("animation-effects", None)] {
        for name in ["src.psd", "expected.psd"] {
            let path = root.join(case).join(name);
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}; run cargo xtask corpus --psd", path.display()));
            let psd = PsdFile::from_bytes(&bytes).unwrap();
            let mut found = 0;
            let mut decoded_descriptors = 0;
            for resource in &psd.resources {
                if let Some(Ok(ResourceData::Animation(animation))) = resource.parsed() {
                    found += 1;
                    assert_eq!(animation.to_bytes().unwrap(), resource.data);
                    for record in &animation.records {
                        if let Some(d) = record.descriptor() {
                            let d = d.unwrap();
                            decoded_descriptors += 1;
                            let frames = frame_info(&d.descriptor).unwrap();
                            assert_eq!(frames.len(), 2, "{case}/{name}");
                            assert!(frames.iter().all(|f| f.delay_centiseconds == delay), "{case}/{name}");
                            let sets = frame_sets(&d.descriptor).unwrap();
                            assert_eq!(sets.len(), 1);
                            assert_eq!(sets[0].frame_ids.len(), 2);
                            assert!(sets[0].frame_ids.iter().all(|id| frames.iter().any(|f| f.id == *id)));
                        }
                    }
                }
            }
            assert_eq!(found, 1, "{case}/{name}");
            assert_eq!(decoded_descriptors, 1, "expected one decoded AnDs descriptor in {case}/{name}");
            let mut state_layers = 0;
            for layer in psd.layers() {
                for block in &layer.blocks {
                    if block.key == *b"shmd" {
                        for item in parse_shmd(&block.data).unwrap() {
                            if item.key == *b"mlst" {
                                let d = item.animation_descriptor().unwrap().unwrap();
                                let states = layer_frame_states(&d.descriptor).unwrap();
                                assert!(!states.states.is_empty());
                                state_layers += 1;
                            }
                        }
                    }
                }
            }
            assert_eq!(state_layers, 2, "{case}/{name}");
            assert_eq!(psd.to_bytes().unwrap(), bytes);
        }
    }
}

/// Files known to be invalid upstream, with the reason. They are reported but
/// do not fail the test.
const KNOWN_BAD: &[(&str, &str)] = &[(
    "group-divider-blend-mode.psd",
    "malformed merged image data; strict parsing rejects it, but the importer recovers its group (photocraft#6152)",
)];

#[test]
fn corpus_parse_and_byte_stable() {
    for dir in ["psd", "psd-tools"] {
        parse_and_byte_stable(&crate::corpus_dir(dir));
    }
}

fn parse_and_byte_stable(root: &Path) {
    assert!(root.is_dir(), "{} is missing: run `cargo xtask corpus --all`", root.display());
    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();
    let mut failures = Vec::new();
    for p in &files {
        let name = p.strip_prefix(root).unwrap_or(p).display().to_string();
        let bytes = match std::fs::read(p) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{name}: read error {e}"));
                continue;
            }
        };
        match PsdFile::from_bytes(&bytes) {
            Err(e) => {
                if let Some((_, why)) = KNOWN_BAD.iter().find(|(f, _)| name.ends_with(f)) {
                    eprintln!("KNOWN {name}: parse error: {e} ({why})");
                } else {
                    eprintln!("FAIL  {name}: parse error: {e}");
                    failures.push(format!("{name}: parse: {e}"));
                }
            }
            Ok(f) => match f.to_bytes() {
                Ok(out) if out == bytes => eprintln!("ok    {name} ({} layers)", f.layers().len()),
                Ok(out) => {
                    let first = out.iter().zip(&bytes).position(|(a, b)| a != b).unwrap_or(out.len().min(bytes.len()));
                    eprintln!("DIFF  {name}: first difference at {first} (len {} vs {})", out.len(), bytes.len());
                    failures.push(format!("{name}: not byte stable (first diff at {first})"));
                }
                Err(e) => failures.push(format!("{name}: write: {e}")),
            },
        }
    }
    eprintln!("corpus {}: {} files, {} failures", root.display(), files.len(), failures.len());
    assert!(failures.is_empty(), "corpus failures:\n{}", failures.join("\n"));
}
