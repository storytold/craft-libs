//! PowerPoint 95 files built here from records, written from the format notes (never files made
//! by PowerPoint), and an optional check over real ones named by `CRAFT_PPT95_SAMPLES`.
#![allow(clippy::unwrap_used)]

use craft_pptx::{Background, ColorRef, Fill, LayoutType, Presentation, Rgba, SchemeSlot};

use super::rt;
use crate::tests::{AppDefaults, build};

const MASTER: u32 = 0x8000_0000;
const NONE: u32 = 0xFFFF;

/// A record: type, instance, length, fourth word, body.
fn rec(kind: u32, instance: u32, tag: u32, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 16);
    for w in [kind, instance, body.len() as u32, tag] {
        v.extend_from_slice(&w.to_le_bytes());
    }
    v.extend_from_slice(body);
    v
}

fn container(kind: u32, tag: u32, children: &[Vec<u8>]) -> Vec<u8> {
    rec(kind, NONE, tag, &children.concat())
}

fn le32(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

const DEFAULT: [u32; 8] = [0xFFFFFF, 0x000000, 0x969696, 0x000000, 0x00CC99, 0x3333CC, 0xCCCCFF, 0xB2B2B2];

/// A `ColorSchemeAtom` of eight colours given as 0xRRGGBB.
fn scheme_atom(tag: u32, colors: [u32; 8]) -> Vec<u8> {
    let mut b = le32(&[8]);
    for c in colors {
        b.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 0]);
    }
    rec(rt::COLOR_SCHEME, 0, tag, &b)
}

/// The page's background shape, filled with R, G, B and an index byte (0xFE: the RGB itself).
fn background_shape(fill: [u8; 4], filled: bool) -> Vec<u8> {
    let geometry = rec(3009, 0, 0, &le32(&[(-3i32) as u32, 0, 0, (-2880i32) as u32, (-2160i32) as u32, 2880, 2160, 0, (-3i32) as u32, 0]));
    let mut format = vec![0u8; 56];
    format[12] = if filled { 0 } else { 0xFF };
    format[20..24].copy_from_slice(&fill);
    container(rt::SHAPE, 0x1C, &[geometry, rec(rt::FORMAT, 1, 0, &format)])
}

/// The slide box: the slide rectangle, then whether it follows the master's objects, scheme and
/// background.
fn slide_box(objects: bool, scheme: bool, background: bool) -> Vec<u8> {
    let mut b = le32(&[(-2880i32) as u32, (-2160i32) as u32, 2880, 2160]);
    b.extend_from_slice(&[u8::from(objects), u8::from(scheme), 0, 0, u8::from(background), 0, 0, 0]);
    rec(rt::SLIDE_BOX, 1, 0, &b)
}

struct SlideSpec {
    id: u32,
    master: u32,
    layout: u32,
    follow_objects: bool,
    follow_scheme: bool,
    follow_background: bool,
    fill: [u8; 4],
    filled: bool,
    scheme: [u32; 8],
}

impl SlideSpec {
    fn new(id: u32, layout: u32) -> SlideSpec {
        SlideSpec {
            id,
            master: MASTER,
            layout,
            follow_objects: true,
            follow_scheme: true,
            follow_background: true,
            fill: [0, 0, 0, 0],
            filled: true,
            scheme: DEFAULT,
        }
    }
}

fn slide(s: &SlideSpec) -> Vec<u8> {
    container(
        rt::SLIDE,
        0x10,
        &[
            rec(rt::SLIDE_ATOM, 0, 0, &le32(&[s.id, s.master])),
            slide_box(s.follow_objects, s.follow_scheme, s.follow_background),
            rec(rt::LAYOUT, 1, 0, &le32(&[s.layout, 0, 0])),
            container(rt::SCHEME, 0x78, &[scheme_atom(0, s.scheme)]),
            background_shape(s.fill, s.filled),
            container(rt::OBJECTS, 0x14, &[]),
        ],
    )
}

fn master(id: u32, scheme: [u32; 8], fill: [u8; 4]) -> Vec<u8> {
    container(
        rt::MAIN_MASTER,
        0x10,
        &[
            rec(rt::SLIDE_ATOM, 0, 0, &le32(&[id, 0])),
            slide_box(true, true, true),
            // A scheme preset, which isn't the master's scheme.
            scheme_atom(0x37, [0x0000FF, 0xFFFFFF, 0, 0xFFFF00, 0xFF9900, 0x00FFFF, 0xFF0033, 0x969696]),
            rec(rt::LAYOUT, 1, 0, &[1, 0, 0, 0, 1, 2, 6, 8, 7, 0, 0, 0]),
            container(rt::SCHEME, 0x78, &[scheme_atom(0, scheme)]),
            background_shape(fill, true),
            container(rt::OBJECTS, 0x14, &[]),
        ],
    )
}

fn document_atom(w: u32, h: u32, slides: u32) -> Vec<u8> {
    let mut b = le32(&[w, h, h, w, 0x0001_0000, 0, 0, 0, slides, 5, 10]);
    b.truncate(44);
    rec(rt::DOCUMENT_ATOM, 4, 0, &b)
}

/// A `PowerPoint Document` stream: root 3 holding the Document and a type-10 atom.
fn stream_of(atom: Vec<u8>, slides: &[Vec<u8>], masters: &[Vec<u8>]) -> Vec<u8> {
    let list = |tag: u32, items: &[Vec<u8>]| {
        let mut kids = vec![rec(2001, 0, 0, &le32(&[items.len() as u32]))];
        kids.extend_from_slice(items);
        container(rt::LIST, tag, &kids)
    };
    let document = container(
        rt::DOCUMENT,
        0,
        &[atom, container(1010, 0x15, &[]), list(0xA, slides), list(0xB, masters), list(0xC, &[]), container(1026, 0x64, &[]), rec(1002, 0, 0, &[])],
    );
    container(rt::ROOT, 0, &[document, rec(10, 0, 0, &le32(&[1, 0x100]))])
}

fn header() -> Vec<u8> {
    let mut h = b"Microsoft (R) PowerPoint (R) Windows  \0".to_vec();
    h.extend_from_slice(&le32(&[7, 0x3F0, 0xE391_C05F]));
    h.extend_from_slice(&[1, 0]);
    h.resize(57, 0);
    h
}

fn file_of(stream: &[u8]) -> Vec<u8> {
    build(&[("PowerPoint Document", stream), ("Header", &header()), ("Current User", &[0; 5])])
}

fn import(stream: &[u8]) -> Result<Presentation, crate::PptError> {
    crate::import(&file_of(stream), &AppDefaults)
}

fn deck(slides: &[SlideSpec]) -> Presentation {
    let s: Vec<Vec<u8>> = slides.iter().map(slide).collect();
    import(&stream_of(document_atom(5760, 4320, s.len() as u32), &s, &[master(MASTER, DEFAULT, [0, 0, 0, 0])])).unwrap()
}

fn layout_kind(p: &Presentation, i: usize) -> LayoutType {
    p.layout(p.slides[i].layout).unwrap().1.kind
}

fn solid(r: u8, g: u8, b: u8) -> Option<Background> {
    Some(Background::Fill { fill: Fill::solid(ColorRef::rgb(Rgba::rgb(r, g, b))) })
}

#[test]
fn a_powerpoint_95_tree_is_detected_and_read() {
    let bytes = file_of(&stream_of(document_atom(5760, 4320, 1), &[slide(&SlideSpec::new(0x100, 16))], &[master(MASTER, DEFAULT, [0; 4])]));
    assert!(crate::sniff(&bytes));
    let (p, lost) = crate::import_checked(&bytes, &AppDefaults).unwrap();
    assert_eq!(p.slides.len(), 1);
    assert_eq!(p.masters.len(), 1);
    assert!(lost.is_empty());
    assert!(p.slides[0].shapes.is_empty());
}

/// A stream that is neither framing still says the file is too old, and a PowerPoint 97 file reads
/// as before.
#[test]
fn other_framings_keep_the_powerpoint_97_path() {
    let mut not95 = stream_of(document_atom(5760, 4320, 0), &[], &[]);
    not95[0] = 4;
    let unknown_token = crate::record::build::atom(crate::doc::rt::CURRENT_USER, 0, &le32(&[0x14, 0xDEAD_BEEF, 0, 0, 0]));
    let f = build(&[("PowerPoint Document", &not95), ("Current User", &unknown_token)]);
    let e = crate::import(&f, &AppDefaults).unwrap_err();
    assert!(matches!(e, crate::PptError::TooOld));
    assert_eq!(e.to_string(), "the presentation is in a PowerPoint format older than PowerPoint 95");
    // The Document must be the root's first child, with no instance.
    let mut wrong_instance = stream_of(document_atom(5760, 4320, 0), &[], &[]);
    wrong_instance[20..24].copy_from_slice(&0u32.to_le_bytes());
    assert!(super::document(&wrong_instance).is_none());
    // ...and hold a DocumentAtom with its slide count: an empty Document is not this path's, and
    // without the PowerPoint 97 records it is damaged.
    let empty = container(rt::ROOT, 0, &[container(rt::DOCUMENT, 0, &[])]);
    assert!(super::document(&empty).is_none());
    assert!(matches!(import(&empty), Err(crate::PptError::Damaged(_))));
    // An atom too short to hold the count.
    assert!(super::document(&stream_of(rec(rt::DOCUMENT_ATOM, 4, 0, &[0; 32]), &[], &[])).is_none());
    // A document with no slides is still one.
    assert_eq!(import(&stream_of(document_atom(5760, 4320, 0), &[], &[])).unwrap().slides.len(), 0);

    let p = crate::import(&crate::tests::file(), &AppDefaults).unwrap();
    assert_eq!(p.slides.len(), 1);
    assert_eq!(p.slides[0].shapes[0].text.as_ref().unwrap().text(), "Hello\nWorld");
}

#[test]
fn slides_keep_their_order_and_the_size() {
    let p = deck(&[SlideSpec::new(0x100, 16), SlideSpec::new(0x101, 0), SlideSpec::new(0x102, 8)]);
    assert_eq!(p.slides.len(), 3);
    assert_eq!((p.slide_size.width, p.slide_size.height), (720.0, 540.0));
    assert_eq!((p.notes_size.width, p.notes_size.height), (540.0, 720.0));
    assert_eq!([layout_kind(&p, 0), layout_kind(&p, 1), layout_kind(&p, 2)], [LayoutType::Blank, LayoutType::Title, LayoutType::TwoContent]);

    let p = import(&stream_of(document_atom(4608, 3456, 1), &[slide(&SlideSpec::new(0x100, 16))], &[])).unwrap();
    assert_eq!((p.slide_size.width, p.slide_size.height), (576.0, 432.0));
    assert_eq!(p.masters.len(), 1, "the defaults' master when the file has none");
    assert_eq!(p.slides.len(), 1);
}

#[test]
fn layout_types_map_to_layouts() {
    let p =
        deck(&[SlideSpec::new(0x100, 0), SlideSpec::new(0x101, 7), SlideSpec::new(0x102, 1), SlideSpec::new(0x103, 16), SlideSpec::new(0x104, 99)]);
    let kinds: Vec<_> = (0..5).map(|i| layout_kind(&p, i)).collect();
    assert_eq!(kinds, [LayoutType::Title, LayoutType::TitleOnly, LayoutType::TitleAndContent, LayoutType::Blank, LayoutType::TitleAndContent]);
}

/// Each slide takes the master it names; an unknown id takes the first master.
#[test]
fn slides_link_to_their_master() {
    let green = [0xFFFFFF, 0, 0x808080, 0, 0x00FF00, 0x3333CC, 0xCCCCFF, 0xB2B2B2];
    let masters = [master(MASTER, DEFAULT, [0; 4]), master(MASTER + 1, green, [0; 4])];
    let mut on_second = SlideSpec::new(0x100, 16);
    on_second.master = MASTER + 1;
    let mut unknown = SlideSpec::new(0x101, 16);
    unknown.master = 0x1234;
    let p = import(&stream_of(document_atom(5760, 4320, 2), &[slide(&on_second), slide(&unknown)], &masters)).unwrap();
    assert_eq!(p.masters.len(), 2);
    let master_of = |i: usize| p.layout(p.slides[i].layout).unwrap().0.id;
    assert_eq!(master_of(0), p.masters[1].id);
    assert_eq!(master_of(1), p.masters[0].id);
}

/// The master's theme comes from its scheme (under 1012), not the presets beside it.
#[test]
fn the_masters_scheme_is_its_theme() {
    let scheme = [0x102030, 0x405060, 0x708090, 0xA0B0C0, 0x112233, 0x445566, 0x778899, 0xAABBCC];
    let p = import(&stream_of(document_atom(5760, 4320, 0), &[], &[master(MASTER, scheme, [0; 4])])).unwrap();
    let c = &p.masters[0].theme.colors;
    assert_eq!(c.get(SchemeSlot::Lt1), Rgba::rgb(0x10, 0x20, 0x30));
    assert_eq!(c.get(SchemeSlot::Dk1), Rgba::rgb(0x40, 0x50, 0x60));
    assert_eq!(c.get(SchemeSlot::Accent1), Rgba::rgb(0x11, 0x22, 0x33));
    assert_eq!(c.get(SchemeSlot::FolHlink), Rgba::rgb(0xAA, 0xBB, 0xCC));
    assert_eq!(p.masters[0].background, None, "the scheme's background colour is the master's anyway");

    let red = import(&stream_of(document_atom(5760, 4320, 0), &[], &[master(MASTER, scheme, [0xFF, 0, 0, 0xFE])])).unwrap();
    assert_eq!(red.masters[0].background, solid(0xFF, 0, 0));
}

#[test]
fn a_slide_that_doesnt_follow_the_master_keeps_its_background() {
    let mut rgb = SlideSpec::new(0x100, 16);
    rgb.follow_background = false;
    rgb.fill = [0xFF, 0, 0, 0xFE];
    // A scheme slot (4, fills) of the slide's own scheme.
    let mut slot = SlideSpec::new(0x101, 16);
    slot.follow_background = false;
    slot.follow_scheme = false;
    slot.fill = [0, 0, 0, 4];
    slot.scheme[4] = 0x123456;
    // Following the master, its own background shape is not used.
    let mut follows = SlideSpec::new(0x102, 16);
    follows.fill = [0, 0xFF, 0, 0xFE];
    let mut unfilled = SlideSpec::new(0x103, 16);
    unfilled.follow_background = false;
    unfilled.filled = false;
    let p = deck(&[rgb, slot, follows, unfilled]);
    assert_eq!(p.slides[0].background, solid(0xFF, 0, 0));
    assert_eq!(p.slides[1].background, solid(0x12, 0x34, 0x56));
    assert_eq!(p.slides[2].background, None);
    assert_eq!(p.slides[3].background, Some(Background::Fill { fill: Fill::None }));
}

/// The slide box's flags at +16 and +17 (seen in PowerPoint 95's files: turning the master's
/// objects off on a slide clears +16; giving one slide its own colour scheme clears +17): a slide
/// that follows the master's scheme takes the master's colours, not the copy it holds.
#[test]
fn slides_follow_the_masters_objects_and_scheme() {
    let mut own = SlideSpec::new(0x100, 16);
    (own.follow_objects, own.follow_scheme, own.follow_background) = (false, false, false);
    own.fill = [0, 0, 0, 0];
    own.scheme[0] = 0x112233;
    let mut follows = SlideSpec::new(0x101, 16);
    follows.follow_background = false;
    follows.fill = [0, 0, 0, 0];
    follows.scheme[0] = 0x445566;
    let p = deck(&[own, follows]);
    assert!(!p.slides[0].show_master_shapes);
    assert!(p.slides[1].show_master_shapes);
    assert_eq!(p.slides[0].background, solid(0x11, 0x22, 0x33), "its own scheme");
    assert_eq!(p.slides[1].background, solid(0xFF, 0xFF, 0xFF), "the master's scheme, not its copy");
}

/// Damaged input never panics or grows without bound: every word set to hostile values,
/// truncations, deep nesting and huge counts.
#[test]
fn hostile_powerpoint_95_streams_never_panic() {
    let mut s = SlideSpec::new(0x100, 0);
    s.follow_background = false;
    s.fill = [0, 0, 0, 7];
    let stream = stream_of(document_atom(5760, 4320, 1), &[slide(&s)], &[master(MASTER, DEFAULT, [0; 4])]);
    for at in (0..stream.len() - 4).step_by(4) {
        for v in [0u32, 1, 3, 1000, 0xFFFF, 0x7FFF_FFFF, 0xFFFF_FFFF] {
            let mut d = stream.clone();
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
            let _ = import(&d);
        }
    }
    for cut in 0..stream.len() {
        let _ = import(&stream[..cut]);
    }
    // A length past the end of the stream: the body is clipped to what is there.
    let mut long = stream.clone();
    long[8..12].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    assert_eq!(import(&long).unwrap().slides.len(), 1);
    // Deep nesting: lists inside lists, which the reader doesn't descend into.
    let mut deep = slide(&SlideSpec::new(0x100, 0));
    for _ in 0..2_000 {
        deep = container(rt::LIST, 0xA, &[deep]);
    }
    assert_eq!(import(&stream_of(document_atom(5760, 4320, 1), &[deep], &[])).unwrap().slides.len(), 0);
    // Huge counts: the document's slide count, the list count and a scheme's count are not sizes.
    let mut huge = SlideSpec::new(0x100, 0);
    huge.scheme = DEFAULT;
    let mut d = stream_of(document_atom(u32::MAX, u32::MAX, u32::MAX), &[slide(&huge)], &[]);
    let count = rec(2001, 0, 0, &le32(&[1]));
    let at = d.windows(count.len()).position(|w| w == count).unwrap();
    d[at + 16..at + 20].copy_from_slice(&u32::MAX.to_le_bytes());
    let p = import(&d).unwrap();
    assert_eq!(p.slides.len(), 1);
    assert_eq!((p.slide_size.width, p.slide_size.height), (720.0, 540.0), "an out-of-range size falls back");
    // Many slides stop at the limit.
    let many = vec![slide(&SlideSpec::new(0x100, 0)); super::MAX_SLIDES + 5];
    let p = import(&stream_of(document_atom(5760, 4320, 0), &many, &[])).unwrap();
    assert_eq!(p.slides.len(), super::MAX_SLIDES);
    // Many masters stop at their own, lower limit: each is a copy of the default master.
    let masters = vec![master(MASTER, DEFAULT, [0, 0, 0, 0]); super::MAX_MASTERS + 5];
    let p = import(&stream_of(document_atom(5760, 4320, 0), &[], &masters)).unwrap();
    assert_eq!(p.masters.len(), super::MAX_MASTERS);
}

/// A file holding fewer slides than its DocumentAtom counts names the missing ones by position,
/// and so does one cut just after a slide's header, which leaves that slide out; a whole file
/// names none.
#[test]
fn slides_the_count_says_are_missing_are_named() {
    let one = slide(&SlideSpec::new(0x100, 16));
    let checked = |count: u32| {
        crate::import_checked(&file_of(&stream_of(document_atom(5760, 4320, count), std::slice::from_ref(&one), &[])), &AppDefaults).unwrap()
    };
    assert_eq!(checked(3).1, ["slide 2", "slide 3"]);
    assert!(checked(1).1.is_empty());
    // A huge count names no more slides than the reader could have read.
    assert_eq!(checked(u32::MAX).1.len(), super::MAX_SLIDES);
    // Two slides, the stream cut just after the second one's header (its stated length kept).
    let two = stream_of(document_atom(5760, 4320, 2), &[one.clone(), one.clone()], &[]);
    let second = two.windows(one.len()).rposition(|w| w == one).unwrap();
    let (p, lost) = crate::import_checked(&file_of(&two[..second + 16]), &AppDefaults).unwrap();
    assert_eq!((p.slides.len(), lost), (1, vec!["slide 2".to_string()]));
    // Cut just after its SlideAtom's header, or partway into the atom: still lost.
    for cut in [32, 36] {
        let (p, lost) = crate::import_checked(&file_of(&two[..second + cut]), &AppDefaults).unwrap();
        assert_eq!((p.slides.len(), lost), (1, vec!["slide 2".to_string()]), "cut at {cut}");
    }
}

/// Real PowerPoint 95 files, when `CRAFT_PPT95_SAMPLES` names a directory of them (they are not in
/// the repository). Each imports with the slide count and size its DocumentAtom gives, and nothing
/// lost.
#[test]
fn powerpoint_95_samples() {
    let Some(dir) = std::env::var_os("CRAFT_PPT95_SAMPLES") else { return };
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("ppt")) {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let bytes = std::fs::read(&path).unwrap();
        let (p, lost) = crate::import_checked(&bytes, &AppDefaults).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(lost.is_empty(), "{name}: {lost:?}");
        let file = craft_cfb::CompoundFile::open(&bytes).unwrap();
        let stream = file.read_path_capped("PowerPoint Document", bytes.len() as u64).unwrap();
        let atom = super::document(&stream).and_then(|d| d.child(rt::DOCUMENT_ATOM)).unwrap();
        assert_eq!(p.slides.len(), atom.u32(32).unwrap() as usize, "{name}");
        // The size in master units, 8 to the point.
        let side = |at: usize| f64::from(atom.u32(at).unwrap()) / 8.0;
        assert_eq!((p.slide_size.width, p.slide_size.height), (side(0), side(4)), "{name}");
        match name.as_str() {
            "twoslides" => assert_eq!(p.slides.len(), 2),
            "title" => assert_eq!(layout_kind(&p, 1), LayoutType::Title),
            "layout3" => assert_eq!(layout_kind(&p, 1), LayoutType::TwoContent),
            "background" => assert_eq!(p.slides[0].background, solid(0xFF, 0, 0)),
            // FollowMasterObjects turned off on the slide.
            "nomasterobj" => assert!(!p.slides[0].show_master_shapes),
            // The Slide Color Scheme dialog's Apply (this slide only): the slide keeps its own scheme.
            "schemeapply" => assert!(p.slides[0].show_master_shapes),
            "base" => assert!(p.slides[0].show_master_shapes),
            _ => {}
        }
        seen += 1;
    }
    eprintln!("read {seen} PowerPoint 95 samples");
    assert!(seen > 0);
}
