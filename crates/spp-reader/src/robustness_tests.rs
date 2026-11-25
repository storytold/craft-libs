//! Never-panic checks: every decoder entry point is fed truncated, corrupted and random byte strings and must
//! return (`Ok` or `Err`) without panicking. Inputs come from a tiny deterministic generator, so a failure always
//! reproduces; seeds are the small valid records the unit tests already build, so the corruptions reach deep into
//! each decoder rather than failing on the first byte.

use crate::container::hdf5::Hdf5File;
use crate::container::qds::QDataStream;
use crate::container::qds::test_encoder::QdsWriter as W;
use crate::decode::assbin::test_writer::{TestMesh, compressed_file, file};
use crate::decode::assbin::{AssbinGeometry, AssbinHeader, AssbinScene, MeshMetadata, Normalization};
use crate::decode::document::{ClassCensus, DocumentHeader, DocumentMaterials, DocumentSettings, Visitor};
use crate::decode::image::{ImageHeader, ImageResource};
use crate::decode::resource_toc::ResourceToc;
use crate::decode::settings::ProjectSettings;
use crate::decode::texture_cache::{CacheHeader, CacheTexture};
use crate::model::ProjectDocument;
use crate::model::cameras::SavedCameras;
use crate::model::viewer::ViewerSettings;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Inputs generated per seed, on top of every truncation.
const VARIANTS_PER_SEED: usize = 300;

/// Knuth's MMIX linear congruential generator; the high bits are the usable ones.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, n: usize) -> usize {
        self.next() as usize % n.max(1)
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

/// Truncations of `seed` (every length for short seeds, evenly spaced ones for long ones), then
/// [`VARIANTS_PER_SEED`] corruptions: a few bytes overwritten, often followed by a cut, with some
/// entirely random strings mixed in.
fn variants(seed: &[u8], rng: &mut Lcg) -> Vec<Vec<u8>> {
    let step = (seed.len() / 256).max(1);
    let mut out: Vec<Vec<u8>> = (0..seed.len()).step_by(step).map(|end| seed[..end].to_vec()).collect();
    for i in 0..VARIANTS_PER_SEED {
        if i % 10 == 0 {
            let len = rng.below(256);
            out.push((0..len).map(|_| rng.byte()).collect());
            continue;
        }
        let mut v = seed.to_vec();
        for _ in 0..1 + rng.below(4) {
            if v.is_empty() {
                break;
            }
            // Half the edits land in the first 4 KiB, where the containers keep their headers.
            let at = match rng.below(2) {
                0 => rng.below(v.len().min(4096)),
                _ => rng.below(v.len()),
            };
            // Mostly small edits, sometimes the extremes that make lengths and counts huge.
            v[at] = match rng.below(4) {
                0 => 0xff,
                1 => 0x00,
                2 => v[at].wrapping_add(1 + rng.byte() % 4),
                _ => rng.byte(),
            };
        }
        if rng.below(2) == 0 {
            v.truncate(rng.below(v.len() + 1));
        }
        out.push(v);
    }
    out
}

/// Runs `decode` on every variant of every seed; fails naming the input if any call panics.
pub(crate) fn never_panics(name: &str, seeds: &[Vec<u8>], decode: impl Fn(&[u8])) {
    // Each entry point gets its own stream, so adding one does not change the others' inputs.
    let mut rng = Lcg(name.bytes().fold(0x5eed, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b))));
    for seed in seeds {
        for input in variants(seed, &mut rng) {
            if catch_unwind(AssertUnwindSafe(|| decode(&input))).is_err() {
                panic!("{name} panicked on {} bytes: {:02x?}", input.len(), &input[..input.len().min(512)]);
            }
        }
    }
}

fn int(v: i32) -> Vec<u8> {
    W::encode(|w| w.int(v))
}

fn assbin_seeds() -> Vec<Vec<u8>> {
    let meshes = [
        TestMesh { name: "Body", material_index: 0, vertices: 4, triangles: 2, uv_sets: 1, color_sets: 1 },
        TestMesh { name: "Trim", material_index: 1, vertices: 3, triangles: 1, uv_sets: 2, color_sets: 0 },
    ];
    vec![file(&meshes, &["Body", "Trim"]), compressed_file(&meshes, &["Body", "Trim"])]
}

fn qds_seed() -> Vec<u8> {
    let mut w = W::new();
    w.raw_map(&[
        ("versionAtLastSave/major", int(9)),
        ("versionAtCreation/major", int(8)),
        ("list", W::encode(|w| w.string_list(&["a", "bc"]))),
        ("center", W::encode(|w| w.vector3d([0.0, 1.0, 2.0]))),
        ("radius", W::encode(|w| w.float(2.5))),
        ("origin", W::encode(|w| w.url("file:///C:/a.obj"))),
        ("nested", W::encode(|w| w.map(&[("x", W::encode(|w| w.double(1.0)))]))),
        ("bytes", W::encode(|w| w.byte_array(b"{}"))),
    ])
    .raw_bytes(&W::encode_list(&[int(1), W::encode(|w| w.null_string())]));
    w.finish()
}

fn image_seed() -> Vec<u8> {
    let payload = lz4_flex::block::compress(&(0..8).collect::<Vec<u8>>());
    let mut w = W::new();
    w.raw_map(&[
        ("width", W::encode(|w| w.uint(2))),
        ("version", W::encode(|w| w.string("v2"))),
        ("pitch", W::encode(|w| w.long_long(4))),
        ("height", W::encode(|w| w.uint(2))),
        ("format", W::encode(|w| w.string("LUM16"))),
        ("dataSizeRaw", W::encode(|w| w.ulong_long(8))),
        ("compression", W::encode(|w| w.string("lz4"))),
        ("dataSizeLZ4", W::encode(|w| w.ulong_long(payload.len() as u64))),
    ])
    .raw_bytes(&payload);
    w.finish()
}

fn cache_seed() -> Vec<u8> {
    let payload = lz4_flex::block::compress(&[10, 20, 30, 40]);
    let mut w = W::new();
    w.raw_map(&[
        ("width", W::encode(|w| w.uint(2))),
        ("version", W::encode(|w| w.string("v1"))),
        ("internalFormat", W::encode(|w| w.int(0x822B))),
        ("height", W::encode(|w| w.uint(1))),
        ("compression", W::encode(|w| w.string("lz4"))),
        ("bufferSize", W::encode(|w| w.uint(4))),
    ])
    .raw_u32(payload.len() as u32)
    .raw_bytes(&payload);
    w.finish()
}

fn toc_seed() -> Vec<u8> {
    let mut w = W::new();
    w.raw_u32(2);
    w.raw_string("images").raw_u32(2).raw_string("/a?version=1").raw_string("/b");
    w.raw_string("brushes").raw_u32(0);
    w.finish()
}

/// Counts objects, so the walker does real work for every callback.
struct Counter(usize);

impl Visitor for Counter {
    fn object(&mut self, _class: &str) {
        self.0 += 1;
    }

    fn unknown_tag(&mut self, _tag: u32, _offset: usize) {}
}

#[test]
fn inflate_never_panics() {
    // A fixed block, a stored block and a dynamic block (the deflate test's stream).
    let seeds = [
        vec![0x4b, 0x4c, 0x4a, 0x06, 0x00],
        vec![0x01, 3, 0, 0xfc, 0xff, b'a', b'b', b'c'],
        vec![
            11, 72, 204, 204, 43, 73, 45, 82, 200, 78, 77, 45, 40, 86, 200, 44, 41, 86, 200, 77, 45, 206, 80, 72, 73, 77, 203, 73, 44, 73, 77, 177,
            86, 8, 24, 85, 48, 170, 128, 190, 10, 0,
        ],
    ];
    never_panics("inflate", &seeds, |d| {
        let _ = crate::inflate::inflate(d);
    });
}

#[test]
fn assbin_decoders_never_panic() {
    let seeds = assbin_seeds();
    never_panics("AssbinHeader::parse", &seeds, |d| {
        let _ = AssbinHeader::parse(d);
    });
    never_panics("AssbinScene::parse", &seeds, |d| {
        let _ = AssbinScene::parse(d);
    });
    never_panics("AssbinGeometry::parse", &seeds, |d| {
        let _ = AssbinGeometry::parse(d);
    });
    never_panics("Normalization::of_scene", &seeds, |d| {
        let _ = Normalization::of_scene(d, None);
    });
}

#[test]
fn document_decoders_never_panic() {
    let seeds = [crate::decode::document::test_writer::sample()];
    never_panics("DocumentHeader::parse", &seeds, |d| {
        let _ = DocumentHeader::parse(d);
    });
    never_panics("walk", &seeds, |d| {
        let _ = crate::decode::document::walk(d, &mut Counter(0));
    });
    never_panics("ClassCensus::scan", &seeds, |d| {
        let _ = ClassCensus::scan(d);
    });
    never_panics("DocumentMaterials::scan", &seeds, |d| {
        let _ = DocumentMaterials::scan(d);
    });
    never_panics("DocumentSettings::scan", &seeds, |d| {
        let _ = DocumentSettings::scan(d);
    });
    never_panics("ProjectDocument::from_bytes", &seeds, |d| {
        let _ = ProjectDocument::from_bytes(d);
    });
    never_panics("SavedCameras::parse", &seeds, |d| {
        let _ = SavedCameras::parse(d);
    });
    never_panics("ViewerSettings::parse", &seeds, |d| {
        let _ = ViewerSettings::parse(d);
    });
}

#[test]
fn qdatastream_and_record_decoders_never_panic() {
    let seeds = [qds_seed(), image_seed(), cache_seed(), toc_seed()];
    never_panics("QDataStream", &seeds, |d| {
        let _ = QDataStream::new(d).read_variant_map();
        let _ = QDataStream::new(d).read_variant_list();
        let _ = QDataStream::new(d).read_variant();
        let _ = QDataStream::new(d).read_string_list();
        let mut s = QDataStream::new(d);
        while s.read_string().is_ok() && !s.is_at_end() {}
    });
    never_panics("ProjectSettings::parse", &seeds, |d| {
        let _ = ProjectSettings::parse(d);
    });
    never_panics("MeshMetadata::parse", &seeds, |d| {
        let _ = MeshMetadata::parse(d);
    });
    never_panics("ResourceToc::parse", &seeds, |d| {
        let _ = ResourceToc::parse(d);
    });
    never_panics("ImageHeader::parse", &seeds, |d| {
        let _ = ImageHeader::parse(d);
    });
    never_panics("ImageResource", &seeds, |d| {
        if let Ok(image) = ImageResource::parse(d) {
            let _ = image.decompress();
            let _ = image.decompress_padded();
        }
    });
    never_panics("CacheHeader::parse", &seeds, |d| {
        let _ = CacheHeader::parse(d);
    });
    never_panics("CacheTexture", &seeds, |d| {
        if let Ok(texture) = CacheTexture::parse(d) {
            let _ = texture.decompress();
            let _ = texture.decompress_padded();
        }
    });
}

#[test]
fn hdf5_reader_never_panics() {
    let seeds = [include_bytes!("../tests/fixtures/small.h5").to_vec()];
    never_panics("Hdf5File", &seeds, |d| {
        let Ok(file) = Hdf5File::from_bytes(d.to_vec()) else {
            return;
        };
        for entry in file.entries().unwrap_or_default() {
            let _ = file.read_prefix(&entry.path, 1 << 20);
        }
    });
}

/// One compiled graph: a uniform colour node with its size in the tag (as the decompiler's tests build it).
fn compiled_graph_seed() -> Vec<u8> {
    let words: [u32; 13] = [0, 0x0c, 0, 0x0108_480c, 0x3f00_0000, 8, 8, 0x0001_0001, 0xcccc, 8, 0xbbbb, 1, 0x04];
    let mut section: Vec<u32> = words.to_vec();
    section.extend([0x08, 0x14]);
    let total = (0x34 + 4 * section.len()) as u32;
    let magic = u32::from_le_bytes(*crate::format_names::COMPILED_GRAPH_MAGIC);
    let header = [magic, 4 << 16, 11, 22, total, 0x1c, 0, total - 0x1c, 0x10002, 0, 1, 0x1c, 2];
    header.iter().chain(&section).flat_map(|w| w.to_le_bytes()).collect()
}

fn description_seed() -> Vec<u8> {
    crate::decode::graph_package::root_xml(
        r#"<?xml version="1.0"?><ROOT formatversion="2.1" asmuid="7"><graphs count="1"><graph pkgurl="pkg://g" label="G">
<outputs count="1"><output uid="5" identifier="out" format="28" usertag="colorspace=$working"/></outputs>
<inputs count="3"><input uid="3135" identifier="$outputsize" type="8" default="8,8"/>
<input uid="4660" identifier="invert" type="4" default="0"/>
<input uid="7" identifier="mask" type="5" usertag="defaultcolor=(128,128,255)"/></inputs></graph></graphs></ROOT>"#,
    )
    .into_bytes()
}

/// A graph package as the signature scan finds it: a compiled graph and its description in unknown bytes.
fn package_seed() -> Vec<u8> {
    let mut file = b"7z junk header".to_vec();
    file.extend(compiled_graph_seed());
    file.extend(description_seed());
    file.extend(b"trailing header");
    file
}

#[test]
fn graph_package_and_decompiler_entry_points_never_panic() {
    use crate::decode::graph_package::compiled::Assembly;
    use crate::decode::graph_package::{Description, GraphPackage};
    use crate::inspect::compiled_graph_dump;
    never_panics("GraphPackage", &[package_seed()], |d| {
        if let Ok(package) = GraphPackage::read(d) {
            for g in package.graphs() {
                if let (Ok(desc), Ok(asm)) = (g.description(), g.assembly()) {
                    let _ = crate::decompile::decompile(&desc, &asm, 32);
                }
            }
        }
        let _ = crate::decompile::find_and_decompile(&[d.to_vec()], "g", 32);
        let mut out = String::new();
        let _ = compiled_graph_dump::dump_package(d, "", &mut out);
        let _ = compiled_graph_dump::ir_package(d, "", &mut out);
        let _ = compiled_graph_dump::stats([("p", d)].into_iter());
    });
    never_panics("Description::parse", &[description_seed()], |d| {
        let _ = Description::parse(d);
    });
    let desc = Description::parse(&description_seed()).ok();
    never_panics("Assembly::parse + decompile", &[compiled_graph_seed()], |d| {
        if let Ok(asm) = Assembly::parse(d) {
            let _ = asm.atomic_nodes();
            if let Some(desc) = &desc {
                let _ = crate::decompile::decompile(desc, &asm, 32);
            }
        }
    });
}

#[test]
fn jpeg_decoders_never_panic() {
    // the decompiler's 8×8 greyscale ramp
    let ramp = "ffd8ffe000104a46494600010100000100010000ffdb0043000201010101010201010102020202020403020202020504040304060506060605060606070908060709070606080b08090a0a0a0a0a06080b0c0b0a0c090a0a0affc0000b080008000801011100ffc4001f0000010501010101010100000000000000000102030405060708090a0bffc400b5100002010303020403050504040000017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9faffda0008010100003f00aff0dbfe0db8ff0057ff001417a7fcbb7ff5abffd9";
    let ramp: Vec<u8> = (0..ramp.len() / 2).map(|i| u8::from_str_radix(&ramp[2 * i..2 * i + 2], 16).unwrap()).collect();
    let good = ramp.clone();
    never_panics("jpeg", &[ramp], |d| {
        let _ = crate::decode::jpeg::decode(d);
        let _ = crate::decode::jpeg::decode_with_alpha(d, &good);
        let _ = crate::decode::jpeg::decode_with_alpha(&good, d);
        let _ = crate::decompile::bitmaps::jpeg_to_png(d);
    });
}

#[test]
fn document_dumps_and_side_records_never_panic() {
    use crate::inspect::DocumentDump;
    use crate::model::baking::ProjectBakeSettings;
    use crate::model::preset::ProjectPreset;
    let seeds = [crate::decode::document::test_writer::sample()];
    never_panics("DocumentDump", &seeds, |d| {
        let _ = DocumentDump::render(d);
        let _ = DocumentDump::render_with(d, usize::MAX);
    });
    never_panics("ProjectPreset::from_bytes", &seeds, |d| {
        let _ = ProjectPreset::from_bytes(d);
    });
    never_panics("ProjectBakeSettings::parse", &seeds, |d| {
        let _ = ProjectBakeSettings::parse(d);
    });
}

#[test]
fn project_file_from_bytes_never_panics() {
    use crate::project::ProjectFile;
    let seeds = [include_bytes!("../tests/fixtures/small.h5").to_vec()];
    never_panics("ProjectFile::from_bytes", &seeds, |d| {
        let Ok(project) = ProjectFile::from_bytes(d.to_vec()) else {
            return;
        };
        let _ = project.settings();
        let _ = project.resource_toc();
        let _ = project.cache_entries();
        let _ = project.mesh();
        let _ = project.mesh_metadata();
        let _ = project.document_census();
        let _ = project.document_materials();
        let _ = project.document_settings();
        let _ = project.model();
        let _ = crate::inspect::ProjectReport::of_project(&project, std::path::Path::new(""), 0, Default::default());
    });
}
