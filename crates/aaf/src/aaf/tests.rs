//! Structural checks of written AAF files against the required properties and rules of the AAF
//! Object Specification v1.1 and the Edit Protocol, read back through the compound file reader.

use std::collections::HashSet;

use craft_timeline::time::FrameRate;

use super::ids::{self, cls, ddef, def, path, pid};
use super::store::{self, Obj, Value};
use super::*;
use crate::model::{CItem, CKind};
use crate::testdoc::{IDENTITY, clip, composition, picture_and_sound_cut, source, track};

fn document() -> Document {
    // two 48-frame clips from source frames 10 and 200, a 12-frame transition
    picture_and_sound_cut(FrameRate::FPS_24, 48, 12, (10, 200))
}

fn written(doc: &Document, small_sectors: bool) -> Vec<u8> {
    let mut report = Report::default();
    let bytes = write(doc, &WriteOptions { identity: IDENTITY, small_sectors }, &mut report).unwrap();
    assert!(!report.has_warnings(), "{report}");
    bytes
}

fn read_clean(bytes: &[u8]) -> Document {
    let mut report = Report::default();
    let doc = read(bytes, &mut report).unwrap();
    assert!(!report.has_warnings(), "{report}");
    doc
}

fn walk<'a>(o: &'a Obj, out: &mut Vec<&'a Obj>) {
    out.push(o);
    for (_, v) in &o.props {
        match v {
            Value::Strong(c) => walk(c, out),
            Value::StrongVec(v) | Value::StrongSet(v, _) => v.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
}

fn keys(dict: &Obj, p: u16) -> HashSet<Vec<u8>> {
    dict.objs(p).iter().filter_map(|d| d.data(pid::IDENTIFICATION).map(<[u8]>::to_vec)).collect()
}

/// filmcraft#323: readers that load the meta-dictionary (pyaaf2) need its ClassDefinitions and
/// TypeDefinitions sets. They are empty: everything written uses the baseline classes and types
/// those readers already know.
#[test]
fn the_meta_dictionary_has_its_class_and_type_definition_sets() {
    for small in [false, true] {
        let bytes = written(&document(), small);
        let cf = craft_cfb::CompoundFile::open(&bytes).unwrap();
        assert_eq!(cf.entries[cf.find("MetaDictionary-1").unwrap()].clsid, ids::META_DICTIONARY);
        for name in ["ClassDefinitions-3", "TypeDefinitions-4"] {
            // u32 count, u32 first free key, u32 last free key, u16 key pid, u8 key size
            let index = cf.read_path(&format!("MetaDictionary-1/{name} index")).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(index.len(), 15, "{name}: an empty set");
            assert_eq!(index[0..4], 0u32.to_le_bytes(), "{name}: no entries");
            assert_eq!(u16::from_le_bytes([index[12], index[13]]), pid::META_IDENTIFICATION, "{name}: keyed by the definition's AUID");
            assert_eq!(index[14], 16, "{name}: 16-byte keys");
        }
        let root = store::read(&bytes).unwrap();
        let meta = root.strong(pid::ROOT_META_DICTIONARY).unwrap();
        for set in [pid::META_CLASS_DEFINITIONS, pid::META_TYPE_DEFINITIONS] {
            assert!(matches!(meta.get(set), Some(Value::StrongSet(items, k)) if items.is_empty() && *k == pid::META_IDENTIFICATION), "{set:#06x}");
        }
    }
}

#[test]
fn written_files_have_the_required_structure() {
    for small in [false, true] {
        let bytes = written(&document(), small);
        assert_written_file(&bytes, small);
    }
}

/// The top-level composition is tagged `Usage_TopLevel`, which the SMPTE Labels register gives as
/// urn:smpte:ul:060e2b34.04010101.0d010102.01010700, stored as the AUID
/// {0d010102-0101-0700-060e-2b3404010101}. Readers that look for it (LibAAF) find no sequence
/// without it.
#[test]
fn the_top_level_composition_has_the_registered_usage_code() {
    let bytes = written(&document(), false);
    let root = store::read(&bytes).unwrap();
    let content = root.strong(pid::ROOT_HEADER).unwrap().strong(pid::CONTENT).unwrap();
    let comp = content.objs(pid::MOBS).iter().find(|m| m.class == cls::COMPOSITION_MOB).unwrap();
    let usage = [0x02, 0x01, 0x01, 0x0D, 0x01, 0x01, 0x00, 0x07, 0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01];
    assert_eq!(comp.data(pid::USAGE_CODE), Some(&usage[..]));
}

/// Files from FilmCraft 0.16 and earlier tag the top-level composition with the unregistered label
/// 060e2b34.04010105.0d011201.02030000. They still import: the composition no other one uses is
/// the edit.
#[test]
fn files_with_the_old_usage_code_still_import() {
    let bytes = written(&document(), false);
    let mut root = store::read(&bytes).unwrap();
    let content = strong_mut(strong_mut(&mut root, pid::ROOT_HEADER), pid::CONTENT);
    for mob in objs_mut(content, pid::MOBS).iter_mut().filter(|m| m.class == cls::COMPOSITION_MOB) {
        mob.set(pid::USAGE_CODE, Value::Data(def::USAGE_TOP_LEVEL_OLD.to_vec()));
    }
    let old_file = store::write(&root, craft_cfb::Version::V4).unwrap();
    let got = read_clean(&old_file);
    assert_eq!(got.compositions.len(), 1);
    assert_eq!(got, read_clean(&bytes));
}

/// A file that holds one edit tagged with each label (a tool merged a composition from an older
/// file into a newer one) imports both edits.
#[test]
fn edits_tagged_with_either_usage_code_both_import() {
    let bytes = written(&document(), false);
    let mut root = store::read(&bytes).unwrap();
    let content = strong_mut(strong_mut(&mut root, pid::ROOT_HEADER), pid::CONTENT);
    let mobs = objs_mut(content, pid::MOBS);
    let mut old = mobs.iter().find(|m| m.class == cls::COMPOSITION_MOB).unwrap().clone();
    let mut id = old.mob_id(pid::MOB_ID).unwrap();
    id[31] ^= 0xFF;
    old.set(pid::MOB_ID, Value::Data(id.to_vec()));
    old.set(pid::MOB_NAME, Value::Data(store::utf16z("Old")));
    old.set(pid::USAGE_CODE, Value::Data(def::USAGE_TOP_LEVEL_OLD.to_vec()));
    mobs.push(old);
    let mixed = store::write(&root, craft_cfb::Version::V4).unwrap();
    let document = read_clean(&mixed);
    let mut ids: Vec<_> = document.compositions.iter().map(|c| c.aaf_mob_id.as_deref().unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())));
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2, "each composition keeps its own MobID");
    let mut names: Vec<String> = document.compositions.into_iter().map(|c| c.name).collect();
    names.sort();
    assert_eq!(names, ["Old", "S"]);
}

fn assert_written_file(bytes: &[u8], small: bool) {
    assert_compound_file(bytes, small);
    let root = store::read(bytes).unwrap();
    assert!(root.strong(pid::ROOT_META_DICTIONARY).is_some());
    let header = root.strong(pid::ROOT_HEADER).unwrap();
    let content = assert_header(header);
    let dict = header.strong(pid::DICTIONARY).unwrap();
    let definitions = Definitions {
        data: keys(dict, pid::DATA_DEFINITIONS),
        operations: keys(dict, pid::OPERATION_DEFINITIONS),
        interpolation: keys(dict, pid::INTERPOLATION_DEFINITIONS),
        containers: keys(dict, pid::CONTAINER_DEFINITIONS),
        parameters: keys(dict, pid::PARAMETER_DEFINITIONS),
    };
    let mobs = content.objs(pid::MOBS);
    let comp = assert_mob_set(content, mobs);
    for mob in mobs {
        assert_mob(mob, mobs, &definitions);
    }
    assert_composition(comp);
}

fn assert_compound_file(bytes: &[u8], small: bool) {
    let cf = craft_cfb::CompoundFile::open(bytes).unwrap();
    assert_eq!(cf.version, if small { 3 } else { 4 });
    assert_eq!(cf.root().clsid, ids::ROOT);
    let references = cf.read_path("referenced properties").unwrap();
    let pids: Vec<u16> = references[7..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert!(pids.windows(3).any(|w| w == path::DATA_DEFS));
    let header = cf.find("Header-2").unwrap();
    assert_eq!(cf.entries[header].clsid, cls::HEADER);
    assert!(cf.read_path("Header-2/properties").unwrap().starts_with(&[0x4C, 0x20]));
}

fn assert_header(header: &Obj) -> &Obj {
    for req in [pid::BYTE_ORDER, pid::LAST_MODIFIED, pid::CONTENT, pid::DICTIONARY, pid::VERSION, pid::IDENTIFICATION_LIST, pid::OBJECT_MODEL_VERSION]
    {
        assert!(header.get(req).is_some(), "Header property {req:#06x}");
    }
    assert_eq!(header.auid(pid::OPERATIONAL_PATTERN), Some(def::OP_EDIT_PROTOCOL));
    let ident = &header.objs(pid::IDENTIFICATION_LIST)[0];
    for req in [pid::COMPANY_NAME, pid::PRODUCT_NAME, pid::PRODUCT_VERSION_STRING, pid::PRODUCT_ID, pid::DATE, pid::GENERATION_AUID] {
        assert!(ident.get(req).is_some(), "Identification property {req:#06x}");
    }
    header.strong(pid::CONTENT).unwrap()
}

struct Definitions {
    data: HashSet<Vec<u8>>,
    operations: HashSet<Vec<u8>>,
    interpolation: HashSet<Vec<u8>>,
    containers: HashSet<Vec<u8>>,
    parameters: HashSet<Vec<u8>>,
}

fn assert_mob_set<'a>(content: &Obj, mobs: &'a [Obj]) -> &'a Obj {
    let mob_ids: Vec<[u8; 32]> = mobs.iter().map(|m| m.mob_id(pid::MOB_ID).unwrap()).collect();
    assert_eq!(mob_ids.iter().collect::<HashSet<_>>().len(), mob_ids.len(), "unique mob ids");
    assert!(matches!(content.get(pid::MOBS), Some(Value::StrongSet(_, k)) if *k == pid::MOB_ID));
    let compositions: Vec<&Obj> = mobs.iter().filter(|m| m.class == cls::COMPOSITION_MOB).collect();
    assert_eq!(compositions.len(), 1);
    assert_eq!(compositions[0].auid(pid::USAGE_CODE), Some(def::USAGE_TOP_LEVEL));
    assert_eq!(mobs.iter().filter(|m| m.class == cls::MASTER_MOB).count(), 1);
    assert_eq!(mobs.iter().filter(|m| m.class == cls::SOURCE_MOB).count(), 3, "picture file, sound file, tape");
    compositions[0]
}

fn assert_mob(mob: &Obj, mobs: &[Obj], definitions: &Definitions) {
    for req in [pid::MOB_ID, pid::MOB_NAME, pid::SLOTS, pid::MOB_LAST_MODIFIED, pid::MOB_CREATION_TIME] {
        assert!(mob.get(req).is_some(), "Mob property {req:#06x}");
    }
    assert_source_descriptor(mob, &definitions.containers);
    for slot in mob.objs(pid::SLOTS) {
        assert!(slot.get(pid::SLOT_ID).is_some() && slot.get(pid::SEGMENT).is_some());
        if slot.class == cls::TIMELINE_MOB_SLOT {
            assert!(slot.rational(pid::EDIT_RATE).is_some_and(|r| r.0 > 0 && r.1 > 0));
            assert!(slot.get(pid::ORIGIN).is_some());
        }
    }
    let mut all = Vec::new();
    walk(mob, &mut all);
    for component in all {
        assert_component(component, mobs, definitions);
    }
}

fn assert_source_descriptor(mob: &Obj, containers: &HashSet<Vec<u8>>) {
    if mob.class != cls::SOURCE_MOB {
        return;
    }
    let descriptor = mob.strong(pid::ESSENCE_DESCRIPTION).expect("SourceMob::EssenceDescription");
    if descriptor.class != cls::TAPE_DESCRIPTOR {
        for req in [pid::SAMPLE_RATE, pid::FILE_LENGTH, pid::CONTAINER_FORMAT] {
            assert!(descriptor.get(req).is_some(), "FileDescriptor property {req:#06x}");
        }
        assert!(containers.contains(descriptor.weak_key(pid::CONTAINER_FORMAT).unwrap()));
        let url = descriptor.objs(pid::LOCATOR)[0].string(pid::URL_STRING).unwrap();
        assert_eq!(url, "file:///m/a.mov");
    }
    if descriptor.class == cls::PCM_DESCRIPTOR {
        assert_eq!(descriptor.u32(pid::CHANNELS), Some(2));
        assert_eq!(descriptor.u32(pid::QUANTIZATION_BITS), Some(24));
        assert_eq!(descriptor.u16(pid::BLOCK_ALIGN), Some(6));
    }
}

fn assert_component(o: &Obj, mobs: &[Obj], definitions: &Definitions) {
    if is_component(o) {
        let dd = o.weak_key(pid::DATA_DEFINITION).expect("Component::DataDefinition");
        assert!(definitions.data.contains(dd), "data definition in the dictionary");
        let event_seq = o.class == cls::SEQUENCE && dd == ddef::DESCRIPTIVE_METADATA;
        assert!(event_seq || o.get(pid::LENGTH).is_some(), "Component::Length");
    }
    if o.class == cls::SOURCE_CLIP {
        assert_source_clip(o, mobs);
    } else if o.class == cls::OPERATION_GROUP {
        assert_operation_group(o, definitions);
    } else if o.class == cls::SEQUENCE && o.get(pid::LENGTH).is_some() {
        assert_sequence_length(o);
    }
}

fn is_component(o: &Obj) -> bool {
    [cls::FILLER, cls::SOURCE_CLIP, cls::SEQUENCE, cls::TRANSITION, cls::OPERATION_GROUP, cls::TIMECODE, cls::COMMENT_MARKER].contains(&o.class)
}

fn assert_source_clip(clip: &Obj, mobs: &[Obj]) {
    let id = clip.mob_id(pid::SOURCE_ID).unwrap();
    let slot = clip.u32(pid::SOURCE_MOB_SLOT_ID).unwrap();
    assert!(clip.get(pid::START_TIME).is_some());
    if id != [0; 32] {
        let target = mobs.iter().find(|m| m.mob_id(pid::MOB_ID) == Some(id)).expect("source clip target in the file");
        assert!(target.objs(pid::SLOTS).iter().any(|s| s.u32(pid::SLOT_ID) == Some(slot)), "target slot exists");
    }
}

fn assert_operation_group(group: &Obj, definitions: &Definitions) {
    assert!(definitions.operations.contains(group.weak_key(pid::OPERATION).unwrap()));
    for parameter in group.objs(pid::PARAMETERS) {
        assert!(definitions.parameters.contains(parameter.auid(pid::PARAMETER_DEFINITION).unwrap().as_slice()));
        if parameter.class == cls::VARYING_VALUE {
            assert!(definitions.interpolation.contains(parameter.weak_key(pid::INTERPOLATION).unwrap()));
            assert!(!parameter.objs(pid::POINT_LIST).is_empty());
        }
    }
}

fn assert_sequence_length(sequence: &Obj) {
    let components = sequence.objs(pid::COMPONENTS);
    let sum = components
        .iter()
        .enumerate()
        .map(|(i, component)| {
            let len = component.i64(pid::LENGTH).unwrap();
            if component.class == cls::TRANSITION {
                assert!(i > 0 && i + 1 < components.len(), "transition at a sequence end");
                assert!(components[i - 1].class != cls::TRANSITION && components[i + 1].class != cls::TRANSITION);
                assert!(components[i - 1].i64(pid::LENGTH).unwrap() >= len && components[i + 1].i64(pid::LENGTH).unwrap() >= len);
                let cut = component.i64(pid::CUT_POINT).unwrap();
                assert!((0..=len).contains(&cut));
                assert!(component.strong(pid::OPERATION_GROUP).is_some());
                -len
            } else {
                len
            }
        })
        .sum::<i64>();
    assert_eq!(sum, sequence.i64(pid::LENGTH).unwrap(), "sequence length = segments − transitions");
}

fn assert_composition(comp: &Obj) {
    let slots = comp.objs(pid::SLOTS);
    let tc = slots[0].strong(pid::SEGMENT).unwrap();
    assert_eq!(tc.class, cls::TIMECODE);
    assert_eq!(tc.u16(pid::TC_FPS), Some(24));
    let video = slots[1].strong(pid::SEGMENT).unwrap().objs(pid::COMPONENTS);
    assert_eq!(video.iter().map(|c| c.class).collect::<Vec<_>>(), vec![cls::SOURCE_CLIP, cls::TRANSITION, cls::SOURCE_CLIP]);
    assert_eq!(video[0].i64(pid::LENGTH), Some(54), "outgoing clip extends to the transition end");
    assert_eq!(video[2].i64(pid::LENGTH), Some(54), "incoming clip starts at the transition start");
    assert_eq!(video[2].i64(pid::START_TIME), Some(194));
    assert_eq!(video[1].i64(pid::CUT_POINT), Some(6));
    assert_eq!(video[1].strong(pid::OPERATION_GROUP).unwrap().weak_key(pid::OPERATION), Some(&def::VIDEO_DISSOLVE[..]));
    assert_audio_composition_slot(&slots[2]);
}

fn assert_audio_composition_slot(slot: &Obj) {
    let audio = slot.strong(pid::SEGMENT).unwrap().objs(pid::COMPONENTS);
    assert_eq!(slot.rational(pid::EDIT_RATE), Some((48_000, 1)));
    assert_eq!(audio[2].class, cls::OPERATION_GROUP);
    assert_eq!(audio[2].weak_key(pid::OPERATION), Some(&def::MONO_AUDIO_GAIN[..]));
    let amplitude = audio[2].objs(pid::PARAMETERS)[0].data(pid::CONSTANT_VALUE).unwrap();
    let (numerator, denominator) = store::rational_of(&amplitude[17..]).unwrap();
    assert!((numerator as f64 / denominator as f64 - 10f64.powf(-3.0 / 20.0)).abs() < 1e-4);
    assert_eq!(audio[1].strong(pid::OPERATION_GROUP).unwrap().weak_key(pid::OPERATION), Some(&def::MONO_AUDIO_DISSOLVE[..]));
}

#[test]
fn store_round_trips_every_stored_form() {
    let leaf = |n: u8| Obj::new([n; 16]).data_prop(0x1B01, vec![n; 16]);
    let root = Obj::new(ids::ROOT)
        .data_prop(0x0003, vec![1, 2, 3])
        .with(0x0004, Value::Strong(Box::new(leaf(1).with(0x0005, Value::StrongVec(vec![leaf(2), leaf(3)])))))
        .with(
            0x0002,
            Value::Strong(Box::new(
                Obj::new([9; 16])
                    .with(0x3B04, Value::Strong(Box::new(Obj::new([8; 16]).with(0x2605, Value::StrongSet(vec![leaf(4), leaf(5)], 0x1B01))))),
            )),
        )
        .with(0x0006, Value::Weak(path::DATA_DEFS.to_vec(), 0x1B01, vec![5; 16]))
        .with(0x0007, Value::WeakVec(path::DATA_DEFS.to_vec(), 0x1B01, vec![vec![4; 16], vec![5; 16]]))
        .with(0x0008, Value::Stream((0..10_000u32).map(|i| i as u8).collect()));
    for v in [craft_cfb::Version::V3, craft_cfb::Version::V4] {
        let bytes = store::write(&root, v).unwrap();
        assert_eq!(store::read(&bytes).unwrap(), root);
    }
    // a data value longer than a property can hold is an error, not a panic
    assert!(store::write(&Obj::new(ids::ROOT).data_prop(1, vec![0; 70_000]), craft_cfb::Version::V4).is_err());
}

/// The property `p` of `o` as a list of objects, to change.
fn objs_mut(o: &mut Obj, p: u16) -> &mut Vec<Obj> {
    match o.props.iter_mut().find(|(k, _)| *k == p).map(|(_, v)| v) {
        Some(Value::StrongVec(v)) | Some(Value::StrongSet(v, _)) => v,
        _ => panic!("no object list {p:#06x}"),
    }
}

fn strong_mut(o: &mut Obj, p: u16) -> &mut Obj {
    match o.props.iter_mut().find(|(k, _)| *k == p).map(|(_, v)| v) {
        Some(Value::Strong(v)) => v,
        _ => panic!("no object {p:#06x}"),
    }
}

fn is_picture(o: &Obj) -> bool {
    o.weak_key(pid::DATA_DEFINITION) == Some(&ddef::PICTURE[..])
}

/// How Premiere Pro 26.5.2 lays a sequence with a nested sequence out in AAF (seen in a file it
/// exported): every composition is tagged top-level, the nested one too; all video tracks of a
/// composition are in ONE slot, a nested scope with a segment per track, lowest first; a clip of
/// the nested sequence is a source clip that points at its composition. Our own file is rewritten
/// into that layout and must read back as the same edit.
#[test]
fn premiere_style_layers_and_nested_compositions_are_read() {
    let r = FrameRate::FPS_24;
    // "Outer": media on V1 and V3, the sequence "S" nested on V2 with its sound on A1
    let inner = document();
    let mut sources = inner.sources.clone();
    for kind in [CKind::Picture, CKind::Sound] {
        let mut nest = source("seq:2", "S", kind, r, 16, None);
        (nest.path, nest.nested, nest.length, nest.width, nest.height) = (None, Some(0), r.tick_of(96), 1920, 1080);
        sources.push(nest);
    }
    let gap = |frames: i64| CItem::Filler(r.tick_of(frames));
    let outer = composition(
        "Outer",
        r,
        vec![
            track(CKind::Picture, "V1", 1, vec![clip(r, 0, 5, 30, "a")]),
            track(CKind::Picture, "V2", 2, vec![gap(10), clip(r, 2, 12, 40, "S")]),
            track(CKind::Picture, "V3", 3, vec![gap(60), clip(r, 0, 100, 20, "a")]),
            track(CKind::Sound, "A1", 1, vec![gap(10), clip(r, 3, 12, 40, "S")]),
        ],
    );
    let ours = written(&Document { compositions: vec![outer], sources, nested: inner.compositions }, false);

    let mut root = store::read(&ours).unwrap();
    let content = strong_mut(strong_mut(&mut root, pid::ROOT_HEADER), pid::CONTENT);
    let mut compositions = 0;
    for mob in objs_mut(content, pid::MOBS).iter_mut().filter(|m| m.class == cls::COMPOSITION_MOB) {
        compositions += 1;
        mob.set(pid::USAGE_CODE, Value::Data(def::USAGE_TOP_LEVEL.to_vec()));
        let slots = objs_mut(mob, pid::SLOTS);
        let picture: Vec<usize> = slots.iter().enumerate().filter(|(_, s)| s.strong(pid::SEGMENT).is_some_and(is_picture)).map(|(i, _)| i).collect();
        let layers: Vec<Obj> = picture.iter().map(|&i| slots[i].strong(pid::SEGMENT).unwrap().clone()).collect();
        let length = layers.iter().filter_map(|l| l.i64(pid::LENGTH)).max().unwrap_or(0);
        let scope = Obj::new(cls::NESTED_SCOPE)
            .with(pid::DATA_DEFINITION, Value::Weak(path::DATA_DEFS.to_vec(), pid::IDENTIFICATION, ddef::PICTURE.to_vec()))
            .data_prop(pid::LENGTH, length.to_le_bytes().to_vec())
            .with(pid::NESTED_SLOTS, Value::StrongVec(layers));
        // the first video slot keeps its id (clips of the nested sequence point at it) and holds
        // every layer; the other video slots go
        slots[picture[0]].set(pid::SEGMENT, Value::Strong(Box::new(scope)));
        for &i in picture[1..].iter().rev() {
            slots.remove(i);
        }
    }
    assert_eq!(compositions, 2);
    let theirs = store::write(&root, craft_cfb::Version::V4).unwrap();
    assert_ne!(theirs, ours);

    let want = read_clean(&ours);
    let got = read_clean(&theirs);
    // one edit; the nested composition is one it uses, not a second edit
    assert_eq!(got.compositions.len(), 1);
    assert_eq!(got.nested.len(), 1);
    // layers of one slot share its name and are all as long as the scope: the same edit apart
    // from picture track names and gaps after the last clip
    let edit = |mut doc: Document| {
        for t in doc.compositions.iter_mut().chain(&mut doc.nested).flat_map(|c| &mut c.tracks).filter(|t| t.kind == CKind::Picture) {
            t.name.clear();
            while matches!(t.items.last(), Some(CItem::Filler(_))) {
                t.items.pop();
            }
        }
        doc
    };
    assert_eq!(edit(got.clone()), edit(want));
    // and that edit is the one that was written: three video tracks, the nest on the second
    let q = &got.compositions[0];
    let picture: Vec<_> = q.tracks.iter().filter(|t| t.kind == CKind::Picture).collect();
    assert_eq!(picture.iter().map(|t| t.items.iter().filter(|i| matches!(i, CItem::Clip(_))).count()).collect::<Vec<_>>(), [1, 1, 1]);
    let nest_of = |kind: CKind| {
        let t = q.tracks.iter().filter(|t| t.kind == kind).nth(if kind == CKind::Picture { 1 } else { 0 }).unwrap();
        let [CItem::Filler(gap), CItem::Clip(c), ..] = &t.items[..] else { panic!("{:?}", t.items) };
        (*gap, c.len, c.start, got.sources[c.source].nested, got.sources[c.source].name.clone())
    };
    assert_eq!(nest_of(CKind::Picture), (r.tick_of(10), r.tick_of(40), r.tick_of(12), Some(0), "S".to_string()));
    assert_eq!(nest_of(CKind::Sound), nest_of(CKind::Picture));
    assert_eq!(got.nested[0].name, "S");
}

/// A slot's name is the track's name, whatever it starts with; a track without one reads back
/// without one.
#[test]
fn audio_tracks_keep_their_names() {
    for name in ["Dialogue.Dialogue-1", "Dialogue.Dialogue-1 L", "", "é", "音声", "🎤 Dialogue", "Ä1"] {
        let mut doc = document();
        doc.compositions[0].tracks[1].name = name.into();
        let back = read_clean(&written(&doc, false));
        assert_eq!(back.compositions[0].tracks[1].name, name);
    }
}
