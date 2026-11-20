//! PowerPoint 95 files built here from records, written from the format notes (never files made
//! by PowerPoint), and an optional check over real ones named by `CRAFT_PPT95_SAMPLES`.
#![allow(clippy::unwrap_used)]

use craft_pptx::text::{Align, Anchor, Bullet, ListStyle, Spacing};
use craft_pptx::{Background, ColorRef, Fill, Geom, LayoutType, Paragraph, PhType, Presentation, Rgba, SchemeSlot, Shape, ShapeKind, TextBody};

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
    /// The slide's object list.
    objects: Vec<Vec<u8>>,
    /// Its notes page (1008).
    notes: Option<Vec<u8>>,
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
            objects: vec![],
            notes: None,
        }
    }
}

fn slide(s: &SlideSpec) -> Vec<u8> {
    let mut kids = vec![rec(rt::SLIDE_ATOM, 0, 0, &le32(&[s.id, s.master])), slide_box(s.follow_objects, s.follow_scheme, s.follow_background)];
    // A notes page comes after the slide box, as PowerPoint 95 writes it.
    kids.extend(s.notes.clone());
    kids.extend([
        rec(rt::LAYOUT, 1, 0, &le32(&[s.layout, 0, 0])),
        container(rt::SCHEME, 0x78, &[scheme_atom(0, s.scheme)]),
        background_shape(s.fill, s.filled),
        container(rt::OBJECTS, 0x14, &s.objects),
    ]);
    container(rt::SLIDE, 0x10, &kids)
}

fn master(id: u32, scheme: [u32; 8], fill: [u8; 4]) -> Vec<u8> {
    master_with(id, scheme, fill, &[])
}

fn master_with(id: u32, scheme: [u32; 8], fill: [u8; 4], objects: &[Vec<u8>]) -> Vec<u8> {
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
            container(rt::OBJECTS, 0x14, objects),
        ],
    )
}

/// A slot list's terminator: what comes before it is slot `n`.
fn slot_end(n: u32) -> Vec<u8> {
    rec(2017, 0, n, &[])
}

/// A `FontEntityAtom` (a LOGFONT) naming `face`, in Windows-1252 bytes.
fn font(face: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 60];
    b[16..20].copy_from_slice(&400u32.to_le_bytes());
    b[28..28 + face.len()].copy_from_slice(face);
    container(4022, 9, &[rec(4023, 0, 0, &b)])
}

/// A ruler: each level's left margin and first-line indent in master units.
fn ruler(levels: [(i32, i32); 5]) -> Vec<u8> {
    let mut b: Vec<u8> = levels.iter().flat_map(|(l, f)| [l.to_le_bytes(), f.to_le_bytes()].concat()).collect();
    b.extend_from_slice(&le32(&[3, 576, 0]));
    container(4069, 9, &[rec(4019, 0, 0, &b), rec(4070, 0, 0, &[])])
}

/// The body ruler (slot 0) and a wide one (slot 3, after an empty slot 2).
const BODY_RULER: [(i32, i32); 5] = [(216, 0), (468, 288), (720, 576), (1008, 864), (1296, 1152)];
const WIDE_RULER: [(i32, i32); 5] = [(576, 288), (828, 648), (1044, 900), (1260, 1116), (1476, 1332)];

/// The Environment: fonts Times New Roman, Arial, Courier New and "Café" by slot, and the rulers:
/// the body's, a plain one, an empty slot and a wide one.
fn environment() -> Vec<u8> {
    environment_with(&[], &[])
}

/// The Environment, starting with `pictures` (the 2006 list) and ending with `styles` (the
/// master's text styles' container) when given.
fn environment_with(pictures: &[Vec<u8>], styles: &[Vec<u8>]) -> Vec<u8> {
    let fonts = container(
        2005,
        0x19,
        &[font(b"Times New Roman"), slot_end(0), font(b"Arial"), slot_end(1), font(b"Courier New"), slot_end(2), font(b"Caf\xE9"), slot_end(3)],
    );
    let plain = [(0, 0), (288, 288), (576, 576), (864, 864), (1152, 1152)];
    let rulers = container(
        4016,
        0x18,
        &[ruler(BODY_RULER), slot_end(0), ruler(plain), slot_end(1), rec(2018, 0, 0, &[]), slot_end(2), ruler(WIDE_RULER), slot_end(3)],
    );
    let mut kids = pictures.to_vec();
    kids.extend([rulers, fonts, container(2005, 0x1A, &[])]);
    kids.extend_from_slice(styles);
    container(1010, 0x15, &kids)
}

/// One level of a master style: font slot, size, style bits, colour; bullet, bullet character,
/// alignment, line spacing, space before and after.
#[derive(Clone, Copy)]
struct Level {
    font: u16,
    size: u16,
    style: u16,
    color: [u8; 4],
    bullet: bool,
    bullet_char: u8,
    align: u8,
    spacing: [i32; 3],
}

const LEVEL: Level = Level { font: 0, size: 24, style: 0, color: [0, 0, 0, 1], bullet: false, bullet_char: 0x95, align: 0, spacing: [100, 0, 0] };

/// A master style atom (4052): five levels of character formatting from +16 (24 bytes each), then
/// five of paragraph formatting from +160 (52 bytes each), with the anchoring at +8.
fn style_atom(levels: [Level; 5], anchor: u8) -> Vec<u8> {
    let mut b = vec![0u8; 472];
    b[8] = anchor;
    b[12] = 0x1F;
    for (i, l) in levels.iter().enumerate() {
        let c = 16 + i * 24;
        b[c..c + 2].copy_from_slice(&l.font.to_le_bytes());
        b[c + 2] = 2;
        b[c + 4..c + 6].copy_from_slice(&l.size.to_le_bytes());
        b[c + 8..c + 10].copy_from_slice(&l.style.to_le_bytes());
        b[c + 10..c + 14].copy_from_slice(&l.color);
        let p = 160 + i * 52;
        b[p] = u8::from(l.bullet);
        b[p + 6] = l.bullet_char;
        b[p + 12] = 100;
        b[p + 24] = i as u8;
        b[p + 28] = l.align;
        for (k, v) in l.spacing.iter().enumerate() {
            b[p + 32 + k * 4..p + 36 + k * 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    rec(4052, 0, 0, &b)
}

/// The master's text styles (1028): a 1029 atom, then the style atoms in text-type order.
fn styles(atoms: &[Vec<u8>]) -> Vec<u8> {
    let mut kids = vec![rec(1029, 0, 0, &[1, 1, 1, 1, 1, 0, 0, 0, 0, 0x0C])];
    kids.extend_from_slice(atoms);
    container(1028, 0x2A, &kids)
}

/// A character run: length, font slot, size in points, style bits (1 bold, 2 italic,
/// 4 underline) and colour (R, G, B, then 0xFE or a scheme slot).
fn char_run(len: u32, font: u16, size: u16, style: u16, color: [u8; 4]) -> Vec<u8> {
    let mut b = vec![0u8; 44];
    b[0..4].copy_from_slice(&len.to_le_bytes());
    b[4..8].copy_from_slice(&0x0FE2u32.to_le_bytes());
    b[20..22].copy_from_slice(&font.to_le_bytes());
    b[24..26].copy_from_slice(&size.to_le_bytes());
    b[28..30].copy_from_slice(&style.to_le_bytes());
    b[30..34].copy_from_slice(&color);
    b
}

/// A plain character run: Times New Roman 24 pt in the scheme's text colour.
fn plain(len: u32) -> Vec<u8> {
    char_run(len, 0, 24, 0, [0, 0, 0, 1])
}

/// A paragraph run: length, bullet on, bullet character, alignment (0 left, 1 centre, 2 right,
/// 3 justified) and level (0-based), single spaced with no space before or after.
fn para_run(len: u32, bullet: bool, bullet_char: u8, align: u8, level: u8) -> Vec<u8> {
    spaced_run(len, bullet, bullet_char, align, level, [100, 0, 0])
}

/// A paragraph run with its line spacing, space before and space after: a percentage of a line,
/// or minus a size in points.
fn spaced_run(len: u32, bullet: bool, bullet_char: u8, align: u8, level: u8, spacing: [i32; 3]) -> Vec<u8> {
    let mut b = vec![0u8; 72];
    for (k, v) in spacing.iter().enumerate() {
        b[52 + k * 4..56 + k * 4].copy_from_slice(&v.to_le_bytes());
    }
    b[0..4].copy_from_slice(&len.to_le_bytes());
    b[4..8].copy_from_slice(&0x0FE3u32.to_le_bytes());
    b[20] = u8::from(bullet);
    b[26] = bullet_char;
    b[32] = 100;
    b[44] = level;
    b[48] = align;
    b
}

/// The text body on slides, and on the master's placeholders.
const BODY: u32 = 4002;
const MASTER_BODY: u32 = 4068;

/// A text container: header, then the body with its text atom (the text rectangle `rect`), ruler
/// slot and runs. The run tables are 2028 containers each holding one 2030 atom of entries.
fn text(chars: &[u8], char_runs: &[Vec<u8>], para_runs: &[Vec<u8>], ruler: u32, rect: [i32; 4], body: u32) -> Vec<u8> {
    text_flagged(chars, char_runs, para_runs, ruler, rect, body, 5, 3)
}

/// A text container whose text atom has the flags `flags` at +4 (bit 2 word wrap) and the
/// anchoring `anchor` at +8.
#[allow(clippy::too_many_arguments)]
fn text_flagged(
    chars: &[u8],
    char_runs: &[Vec<u8>],
    para_runs: &[Vec<u8>],
    ruler: u32,
    rect: [i32; 4],
    body: u32,
    flags: u32,
    anchor: u32,
) -> Vec<u8> {
    let mut atom = le32(&[4, flags, anchor, 0xFFFF_8000, 0]);
    atom.extend(rect.iter().flat_map(|v| v.to_le_bytes()));
    let table = |tag: u32, entries: &[u8]| container(2028, tag, &[rec(2030, 0, tag, entries)]);
    let runs = container(
        4064,
        0,
        &[rec(2030, 0, 0x35, chars), table(0x2F, &char_runs.concat()), table(0x30, &para_runs.concat()), table(0x31, &[0; 24]), rec(4013, 0, 0, &[])],
    );
    container(4001, 0x1E, &[rec(4000, 0, 0, &[0; 16]), container(body, 0, &[rec(4003, 0, 0, &atom), rec(4021, 0, 0, &le32(&[ruler])), runs])])
}

/// Text in one paragraph run and one plain character run, inset 58 × 29 units in `bx`.
fn simple_text(chars: &[u8], bx: [i32; 4], body: u32) -> Vec<u8> {
    let n = chars.len() as u32 + 1;
    text(chars, &[plain(n)], &[para_run(n, false, 0x95, 0, 0)], 1, [bx[0] + 58, bx[1] + 29, bx[2] - 58, bx[3] - 29], body)
}

/// A shape's geometry: a rectangle at `bx` (left, top, right, bottom about the slide's centre),
/// turned `rot` sixteenths of a degree.
fn geometry(bx: [i32; 4], rot: i32) -> Vec<u8> {
    let mut b = le32(&[(-3i32) as u32, 0, 0]);
    b.extend(bx.iter().chain(&[rot]).flat_map(|v| v.to_le_bytes()));
    b.extend_from_slice(&le32(&[(-3i32) as u32, 1]));
    rec(3009, 0, 0, &b)
}

/// A shape (3008), with text when `text` is given.
fn shape_95(bx: [i32; 4], text: Option<Vec<u8>>) -> Vec<u8> {
    let mut kids = vec![geometry(bx, 0)];
    kids.extend(text);
    container(rt::SHAPE, 0x13, &kids)
}

/// A placeholder (3010) of PowerPoint 95 type `kind`, size (0 full, 1 half, 2 quarter) and index.
fn placeholder_95(kind: u8, size: u8, index: u32, bx: [i32; 4], text: Option<Vec<u8>>) -> Vec<u8> {
    let mut atom = vec![kind, size, 0, 0];
    atom.extend_from_slice(&index.to_le_bytes());
    let mut kids = vec![rec(3011, 0, 0, &atom), geometry(bx, 0)];
    kids.extend(text);
    container(3010, 0x13, &kids)
}

/// One slide holding `objects` (its scheme's title colour 0x112233), on a master holding
/// `master_objects`.
fn deck_of(objects: Vec<Vec<u8>>, master_objects: &[Vec<u8>]) -> Presentation {
    let mut s = SlideSpec::new(0x100, 16);
    // A slide with its own scheme, as PowerPoint 95 marks one: not following the master's.
    s.scheme[3] = 0x112233;
    s.follow_scheme = false;
    s.objects = objects;
    import(&stream_of(document_atom(5760, 4320, 1), &[slide(&s)], &[master_with(MASTER, DEFAULT, [0; 4], master_objects)])).unwrap()
}

fn body_of(s: &Shape) -> &TextBody {
    s.text.as_ref().unwrap()
}

fn texts(p: &Presentation) -> Vec<String> {
    p.slides[0].shapes.iter().map(|s| body_of(s).text()).collect()
}

fn boxed(s: &Shape) -> (f64, f64, f64, f64) {
    let x = s.xfrm.unwrap();
    (x.x, x.y, x.w, x.h)
}

fn document_atom(w: u32, h: u32, slides: u32) -> Vec<u8> {
    let mut b = le32(&[w, h, h, w, 0x0001_0000, 0, 0, 0, slides, 5, 10]);
    b.truncate(44);
    rec(rt::DOCUMENT_ATOM, 4, 0, &b)
}

/// A `PowerPoint Document` stream: root 3 holding the Document and a type-10 atom.
fn stream_of(atom: Vec<u8>, slides: &[Vec<u8>], masters: &[Vec<u8>]) -> Vec<u8> {
    stream_with(atom, slides, masters, environment(), &[])
}

/// A stream with `environment`, whose Document ends with `extra` records (a notes master, say).
fn stream_with(atom: Vec<u8>, slides: &[Vec<u8>], masters: &[Vec<u8>], environment: Vec<u8>, extra: &[Vec<u8>]) -> Vec<u8> {
    let list = |tag: u32, items: &[Vec<u8>]| {
        let mut kids = vec![rec(2001, 0, 0, &le32(&[items.len() as u32]))];
        kids.extend_from_slice(items);
        container(rt::LIST, tag, &kids)
    };
    let mut kids = vec![atom, environment, list(0xA, slides), list(0xB, masters)];
    kids.extend_from_slice(extra);
    kids.extend([list(0xC, &[]), container(1026, 0x64, &[]), rec(1002, 0, 0, &[])]);
    let document = container(rt::DOCUMENT, 0, &kids);
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

/// A title slide: the centred title and subtitle placeholders with their text, index, size and
/// box (from master units about the centre to points from the top left), and the text rectangle
/// as insets.
#[test]
fn title_and_subtitle_placeholders_keep_their_text() {
    let title = placeholder_95(0x0F, 0, 0, [-2448, -720, 2448, 0], Some(simple_text(b"Title text", [-2448, -720, 2448, 0], BODY)));
    let sub = placeholder_95(0x10, 1, 1, [-2016, 288, 2016, 1392], Some(simple_text(b"Sub text", [-2016, 288, 2016, 1392], BODY)));
    let p = deck_of(vec![title, sub], &[]);
    let shapes = &p.slides[0].shapes;
    assert_eq!(texts(&p), ["Title text", "Sub text"]);
    let ph: Vec<_> = shapes.iter().map(|s| s.ph.clone().unwrap()).collect();
    assert_eq!((ph[0].kind, ph[0].idx, ph[0].size.as_deref()), (PhType::CtrTitle, 0, None));
    assert_eq!((ph[1].kind, ph[1].idx, ph[1].size.as_deref()), (PhType::SubTitle, 1, Some("half")));
    assert_eq!(boxed(&shapes[0]), (54.0, 180.0, 612.0, 90.0));
    assert_eq!(boxed(&shapes[1]), (108.0, 306.0, 504.0, 138.0));
    let b = &body_of(&shapes[0]).body;
    assert_eq!((b.inset_l, b.inset_t, b.inset_r, b.inset_b), (Some(7.25), Some(3.625), Some(7.25), Some(3.625)));
    assert!(!shapes[0].text_box);

    // The other types, each an empty placeholder (no text container), which draws nothing.
    let kinds = [
        (0x0C, PhType::Title),
        (0x0D, PhType::Body),
        (1, PhType::Title),
        (2, PhType::Body),
        (6, PhType::Date),
        (7, PhType::SlideNum),
        (8, PhType::Footer),
    ];
    let p = deck_of(kinds.iter().map(|(k, _)| placeholder_95(*k, 0, 0, [0, 0, 100, 100], None)).collect(), &[]);
    let got: Vec<_> = p.slides[0].shapes.iter().map(|s| s.ph_type().unwrap()).collect();
    assert_eq!(got, kinds.map(|(_, t)| t));
    assert!(p.slides[0].shapes.iter().all(|s| body_of(s).paragraphs.is_empty()));
}

/// A bulleted body: paragraphs split at `\r`, each with its level, bullet character and the
/// margins its level has in the text's ruler.
#[test]
fn a_bulleted_body_keeps_levels_bullets_and_margins() {
    let chars = b"One\rTwo\rThree\rFour";
    let runs = [para_run(4, true, 0x95, 0, 0), para_run(4, true, 0x96, 0, 1), para_run(6, true, b'*', 0, 2), para_run(5, false, 0x95, 0, 0)];
    let body = text(chars, &[plain(19)], &runs, 0, [0; 4], BODY);
    let p = deck_of(vec![placeholder_95(0x0D, 0, 1, [-2448, -912, 2448, 1680], Some(body))], &[]);
    let paras = &body_of(&p.slides[0].shapes[0]).paragraphs;
    assert_eq!(paras.iter().map(Paragraph::text).collect::<Vec<_>>(), ["One", "Two", "Three", "Four"]);
    assert_eq!(paras.iter().map(|p| p.level).collect::<Vec<_>>(), [0, 1, 2, 0]);
    let bullet = |c: &str| Some(Bullet::Char { char: c.into() });
    assert_eq!(paras.iter().map(|p| p.props.bullet.clone()).collect::<Vec<_>>(), [bullet("•"), bullet("–"), bullet("*"), Some(Bullet::None)]);
    // Ruler slot 0: level 1 at 216 units with its first line at 0, level 2 at 468 and 288.
    assert_eq!((paras[0].props.margin_left, paras[0].props.indent), (Some(27.0), Some(-27.0)));
    assert_eq!((paras[1].props.margin_left, paras[1].props.indent), (Some(58.5), Some(-22.5)));

    // Slot 3 follows an empty slot (2018): slots are counted by their ends, not by the rulers.
    let wide = text(b"One", &[plain(4)], &[para_run(4, true, 0x95, 0, 0)], 3, [0; 4], BODY);
    let p = deck_of(vec![placeholder_95(0x0D, 0, 1, [0, 0, 100, 100], Some(wide))], &[]);
    let para = &body_of(&p.slides[0].shapes[0]).paragraphs[0];
    assert_eq!((para.props.margin_left, para.props.indent), (Some(72.0), Some(-36.0)));
    // The empty slot, or one past the table, gives no margins.
    for slot in [2, 99] {
        let t = text(b"One", &[plain(4)], &[para_run(4, true, 0x95, 0, 0)], slot, [0; 4], BODY);
        let p = deck_of(vec![placeholder_95(0x0D, 0, 1, [0, 0, 100, 100], Some(t))], &[]);
        assert_eq!(body_of(&p.slides[0].shapes[0]).paragraphs[0].props.margin_left, None);
    }
}

/// Character runs: bold, italic and underline bits, the font by slot, the size, and an RGB or a
/// scheme slot of the slide's scheme as the colour.
/// A run of no characters is skipped: it neither formats the text nor uses up the runs read, so a
/// run after it still formats the text.
#[test]
fn zero_length_runs_are_skipped() {
    let runs = [plain(0), plain(0), plain(0), plain(0), char_run(4, 1, 40, 1, [0, 0, 0, 1])];
    let body = text(b"abc", &runs, &[para_run(0, false, 0, 2, 0), para_run(4, false, 0, 1, 0)], 1, [0; 4], BODY);
    let p = deck_of(vec![shape_95([-2362, -1613, -518, -1325], Some(body))], &[]);
    let para = &body_of(&p.slides[0].shapes[0]).paragraphs[0];
    let r = &para.runs[0].props;
    assert_eq!((para.runs[0].text.as_str(), r.font.as_deref(), r.size, r.bold), ("abc", Some("Arial"), Some(40.0), Some(true)));
    assert_eq!(para.props.align, Some(Align::Center));
}

/// The font table is the first font list holding a face: a list whose only face is cut short
/// gives way to a later one.
#[test]
fn the_font_table_is_the_first_list_with_a_face() {
    let cut = container(4022, 9, &[rec(4023, 0, 0, &[0; 8])]);
    let lists = [container(2005, 0x19, &[cut, slot_end(0)]), container(2005, 0x19, &[font(b"Arial"), slot_end(0)])];
    let mut s = SlideSpec::new(0x100, 16);
    s.objects = vec![shape_95([-2362, -1613, -518, -1325], Some(simple_text(b"Hi", [-2362, -1613, -518, -1325], BODY)))];
    let stream = stream_with(document_atom(5760, 4320, 1), &[slide(&s)], &[master(MASTER, DEFAULT, [0; 4])], container(1010, 0x15, &lists), &[]);
    let p = import(&stream).unwrap();
    assert_eq!(body_of(&p.slides[0].shapes[0]).paragraphs[0].runs[0].props.font.as_deref(), Some("Arial"));
}

/// A paragraph's end mark keeps the formatting of the character run at its CR, which PowerPoint 95
/// gives a run of its own when it differs (`endmark`, `endmark2`): not that of the paragraph's
/// first character.
#[test]
fn a_paragraphs_end_mark_keeps_the_formatting_at_its_cr() {
    let end_sizes = |runs: &[(u32, u16)]| {
        let runs: Vec<_> = runs.iter().map(|&(n, size)| char_run(n, 0, size, 0, [0, 0, 0, 1])).collect();
        let body = text(b"AB\rCD", &runs, &[para_run(5, false, 0, 0, 0)], 1, [0; 4], BODY);
        let p = deck_of(vec![shape_95([-2362, -1613, -518, -1325], Some(body))], &[]);
        body_of(&p.slides[0].shapes[0]).paragraphs.iter().map(|para| para.end_props.size).collect::<Vec<_>>()
    };
    // What PowerPoint 95 wrote for "AB\rCD" at 40 pt with A and C at 24, and at 24 with the first
    // CR at 40.
    assert_eq!(end_sizes(&[(1, 24), (2, 40), (1, 24), (1, 40)]), [Some(40.0), Some(40.0)]);
    assert_eq!(end_sizes(&[(2, 24), (1, 40), (2, 24)]), [Some(40.0), Some(24.0)]);
}

#[test]
fn character_runs_keep_their_formatting() {
    let runs = [
        char_run(4, 0, 24, 1, [0, 0, 0, 1]),
        char_run(4, 1, 24, 2, [0, 0, 0, 1]),
        char_run(4, 2, 40, 4, [0xFF, 0, 0, 0xFE]),
        char_run(5, 99, 0, 7, [0, 0, 0, 3]),
    ];
    let body = text(b"BoldItalUndrLast", &runs, &[para_run(17, false, 0, 0, 0)], 1, [0; 4], BODY);
    let p = deck_of(vec![shape_95([-2362, -1613, -518, -1325], Some(body))], &[]);
    let runs = &body_of(&p.slides[0].shapes[0]).paragraphs[0].runs;
    assert_eq!(runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["Bold", "Ital", "Undr", "Last"]);
    let r: Vec<_> = runs.iter().map(|r| &r.props).collect();
    assert_eq!([r[0].bold, r[1].bold, r[2].bold, r[3].bold], [Some(true), Some(false), Some(false), Some(true)]);
    assert_eq!([r[0].italic, r[1].italic, r[3].italic], [Some(false), Some(true), Some(true)]);
    assert_eq!([r[1].underline.as_deref(), r[2].underline.as_deref()], [Some("none"), Some("sng")]);
    assert_eq!(
        r.iter().map(|r| r.font.as_deref()).collect::<Vec<_>>(),
        [Some("Times New Roman"), Some("Arial"), Some("Courier New"), None],
        "an unknown font slot leaves the font to the master"
    );
    assert_eq!(r.iter().map(|r| r.size).collect::<Vec<_>>(), [Some(24.0), Some(24.0), Some(40.0), None]);
    let solid = |c: Rgba| Some(Fill::solid(ColorRef::rgb(c)));
    assert_eq!(r[0].fill, solid(Rgba::rgb(0, 0, 0)));
    assert_eq!(r[2].fill, solid(Rgba::rgb(0xFF, 0, 0)));
    assert_eq!(r[3].fill, solid(Rgba::rgb(0x11, 0x22, 0x33)), "slot 3 of the slide's own scheme");
}

#[test]
fn paragraphs_keep_their_alignment() {
    let runs = [
        para_run(2, false, 0, 0, 0),
        para_run(2, false, 0, 1, 0),
        para_run(2, false, 0, 2, 0),
        para_run(2, false, 0, 3, 0),
        para_run(2, false, 0, 9, 0),
    ];
    let body = text(b"L\rC\rR\rJ\rX", &[plain(10)], &runs, 1, [0; 4], BODY);
    let p = deck_of(vec![shape_95([0, 0, 1000, 1000], Some(body))], &[]);
    let aligns: Vec<_> = body_of(&p.slides[0].shapes[0]).paragraphs.iter().map(|p| p.props.align).collect();
    assert_eq!(aligns, [Some(Align::Left), Some(Align::Center), Some(Align::Right), Some(Align::Justify), None]);
}

/// Line spacing, space before and space after: a percentage of a line when positive, minus a
/// size in points when negative; hostile sizes are bounded.
#[test]
fn paragraphs_keep_their_spacing() {
    let runs = [
        spaced_run(2, false, 0, 0, 0, [150, 50, 0]),
        spaced_run(2, false, 0, 0, 0, [-36, -12, -6]),
        spaced_run(2, false, 0, 0, 0, [i32::MAX, i32::MIN, -1]),
    ];
    let body = text(b"A\rB\rC", &[plain(6)], &runs, 1, [0; 4], BODY);
    let p = deck_of(vec![shape_95([0, 0, 1000, 1000], Some(body))], &[]);
    let got: Vec<_> =
        body_of(&p.slides[0].shapes[0]).paragraphs.iter().map(|p| (p.props.line_spacing, p.props.space_before, p.props.space_after)).collect();
    assert_eq!(
        got,
        [
            (Some(Spacing::Pct(1.5)), Some(Spacing::Pct(0.5)), Some(Spacing::Pct(0.0))),
            (Some(Spacing::Pts(36.0)), Some(Spacing::Pts(12.0)), Some(Spacing::Pts(6.0))),
            (Some(Spacing::Pct(100.0)), Some(Spacing::Pts(1584.0)), Some(Spacing::Pts(1.0))),
        ]
    );
}

/// The text atom's +8 anchoring: bits 4–5 top, middle or bottom, bit 1 clear for centred
/// across; an unknown vertical code leaves the anchoring alone. Its +4 bit 2 is word wrap.
#[test]
fn text_keeps_its_anchoring_and_wrap() {
    let bx = [0, 0, 1000, 1000];
    let anchored = |flags: u32, anchor: u32, placeholder: bool| {
        let t = text_flagged(b"A", &[plain(2)], &[para_run(2, false, 0, 0, 0)], 1, [0; 4], BODY, flags, anchor);
        let o = if placeholder { placeholder_95(0x0D, 0, 1, bx, Some(t)) } else { shape_95(bx, Some(t)) };
        let b = body_of(&deck_of(vec![o], &[]).slides[0].shapes[0]).body.clone();
        (b.anchor, b.anchor_ctr, b.wrap)
    };
    assert_eq!(anchored(5, 0x03, false), (Some(Anchor::Top), Some(false), Some(true)));
    assert_eq!(anchored(5, 0x13, false), (Some(Anchor::Middle), Some(false), Some(true)));
    assert_eq!(anchored(5, 0x23, false), (Some(Anchor::Bottom), Some(false), Some(true)));
    assert_eq!(anchored(5, 0x01, false), (Some(Anchor::Top), Some(true), Some(true)));
    assert_eq!(anchored(5, 0x11, true), (Some(Anchor::Middle), Some(true), Some(true)));
    assert_eq!(anchored(5, 0x21, true), (Some(Anchor::Bottom), Some(true), Some(true)));
    assert_eq!(anchored(1, 0x03, false), (Some(Anchor::Top), Some(false), Some(false)), "no word wrap");
    // An unknown code: a text box keeps its top anchoring, a placeholder its master's.
    assert_eq!(anchored(5, 0xF3, false), (Some(Anchor::Top), None, Some(true)));
    assert_eq!(anchored(4, 0xF3, true), (None, None, Some(true)));
}

/// The master's text styles (the Environment's 4052 atoms, by text type) become its title, body
/// and other styles: each level's font, size, bold, colour, bullet, alignment and spacing, with
/// the margins of the ruler its placeholder names.
#[test]
fn the_masters_text_styles_are_read() {
    let title = Level { font: 1, size: 44, style: 1, color: [0, 0, 0, 3], align: 1, ..LEVEL };
    let body: [Level; 5] = std::array::from_fn(|i| Level {
        size: [32, 28, 24, 20, 20][i],
        bullet: true,
        bullet_char: if i % 2 == 1 { 0x96 } else { 0x95 },
        spacing: [100, 20, 0],
        ..LEVEL
    });
    let notes = Level { size: 12, ..LEVEL };
    let other = Level { color: [0xFF, 0, 0, 0xFE], align: 3, spacing: [150, -12, -6], ..LEVEL };
    let atoms = [style_atom([title; 5], 0x13), style_atom(body, 3), style_atom([notes; 5], 3), style_atom([notes; 5], 3), style_atom([other; 5], 3)];
    let master_objects = [
        placeholder_95(1, 0, 0, [-2448, -1776, 2448, -1056], Some(text(b"T", &[plain(2)], &[para_run(2, false, 0, 1, 0)], 1, [0; 4], MASTER_BODY))),
        placeholder_95(2, 0, 1, [-2448, -912, 2448, 1680], Some(text(b"B", &[plain(2)], &[para_run(2, true, 0x95, 0, 0)], 0, [0; 4], MASTER_BODY))),
    ];
    let mut scheme = DEFAULT;
    scheme[3] = 0x112233;
    let stream = |env: Vec<u8>| stream_with(document_atom(5760, 4320, 0), &[], &[master_with(MASTER, scheme, [0; 4], &master_objects)], env, &[]);
    let p = import(&stream(environment_with(&[], &[styles(&atoms)]))).unwrap();
    let m = &p.masters[0];

    let t = m.title_style.level(0).unwrap();
    assert_eq!((t.run.font.as_deref(), t.run.size, t.run.bold), (Some("Arial"), Some(44.0), Some(true)));
    assert_eq!(t.run.fill, Some(Fill::solid(ColorRef::rgb(Rgba::rgb(0x11, 0x22, 0x33)))), "slot 3 of the master's scheme");
    assert_eq!((t.para.align, t.para.bullet.clone()), (Some(Align::Center), Some(Bullet::None)));
    // The plain ruler (slot 1) the master title names: no margin at level 1.
    assert_eq!((t.para.margin_left, t.para.indent), (Some(0.0), Some(0.0)));

    let sizes: Vec<_> = (0..5).map(|l| m.body_style.level(l).unwrap().run.size).collect();
    assert_eq!(sizes, [Some(32.0), Some(28.0), Some(24.0), Some(20.0), Some(20.0)]);
    let b = m.body_style.level(1).unwrap();
    assert_eq!(b.para.bullet, Some(Bullet::Char { char: "–".into() }));
    assert_eq!((b.para.line_spacing, b.para.space_before), (Some(Spacing::Pct(1.0)), Some(Spacing::Pct(0.2))));
    // The body ruler (slot 0) the master body names: level 2 at 468 units, first line at 288.
    assert_eq!((b.para.margin_left, b.para.indent), (Some(58.5), Some(-22.5)));

    let o = m.other_style.level(0).unwrap();
    assert_eq!((o.run.size, o.run.fill.clone()), (Some(24.0), Some(Fill::solid(ColorRef::rgb(Rgba::rgb(0xFF, 0, 0))))));
    assert_eq!((o.para.align, o.para.line_spacing, o.para.space_after), (Some(Align::Justify), Some(Spacing::Pct(1.5)), Some(Spacing::Pts(6.0))));
    assert_eq!(o.para.margin_left, None, "other text has no master placeholder, so no ruler");

    // Without styles, or with a truncated atom, the defaults' styles stay as they are.
    let plain_master = import(&stream(environment())).unwrap();
    let defaults = |p: &Presentation| -> [ListStyle; 3] {
        let m = &p.masters[0];
        [m.title_style.clone(), m.body_style.clone(), m.other_style.clone()]
    };
    assert_ne!(defaults(&p), defaults(&plain_master));
    let mut cut = style_atom([title; 5], 0x13);
    cut.truncate(16 + 400);
    let cut = import(&stream(environment_with(&[], &[styles(&[cut])]))).unwrap();
    assert_eq!(defaults(&cut), defaults(&plain_master));
}

/// A master whose title placeholder is a centred title (PowerPoint 95 type 0x0F) gives the title
/// style its ruler's margins, as a plain title does.
#[test]
fn a_masters_centred_title_gives_the_title_style_its_ruler() {
    let title = placeholder_95(
        0x0F,
        0,
        0,
        [-2448, -1776, 2448, -1056],
        Some(text(b"T", &[plain(2)], &[para_run(2, false, 0, 1, 0)], 3, [0; 4], MASTER_BODY)),
    );
    let atoms = [style_atom([LEVEL; 5], 0x13)];
    let stream = stream_with(
        document_atom(5760, 4320, 0),
        &[],
        &[master_with(MASTER, DEFAULT, [0; 4], &[title])],
        environment_with(&[], &[styles(&atoms)]),
        &[],
    );
    let t = import(&stream).unwrap().masters[0].title_style.level(0).cloned().unwrap();
    // The wide ruler (slot 3): level 1 at 576 units, its first line at 288.
    assert_eq!((t.para.margin_left, t.para.indent), (Some(72.0), Some(-36.0)));
}

/// A shape with text and no format of its own is a text box with no fill or line; one without text
/// draws nothing and is left out; a placeholder of a type PowerPoint 95 isn't known to write keeps
/// its text as a text box.
#[test]
fn a_shape_with_text_is_a_text_box() {
    let bx = [-2362, -1613, -518, -1325];
    let p = deck_of(
        vec![
            shape_95(bx, Some(simple_text(b"Box", bx, BODY))),
            shape_95([0, 0, 100, 100], None),
            placeholder_95(0x42, 0, 3, bx, Some(simple_text(b"Odd", bx, BODY))),
        ],
        &[],
    );
    assert_eq!(texts(&p), ["Box", "Odd"]);
    let s = &p.slides[0].shapes[0];
    assert!(s.text_box && s.ph.is_none());
    assert_eq!((s.fill.clone(), s.line.clone()), (Some(Fill::None), Some(craft_pptx::Line::none())));
    assert_eq!(boxed(s), (64.75, 68.375, 230.5, 36.0));
    assert!(p.slides[0].shapes[1].text_box && p.slides[0].shapes[1].ph.is_none());

    // The box is the unturned one; the angle is in sixteenths of a degree, counter-clockwise.
    let turned = container(rt::SHAPE, 0x13, &[geometry(bx, 480), simple_text(b"Box", bx, BODY)]);
    let p = deck_of(vec![turned], &[]);
    let x = p.slides[0].shapes[0].xfrm.unwrap();
    assert_eq!((x.x, x.w, x.rot), (64.75, 230.5, 330.0));
}

/// The master's placeholders place the default master's (title and body), and the footer row
/// places every layout's too, with the master's alignment and size. The first of each family
/// counts; the master's other objects are its own shapes.
#[test]
fn master_placeholders_move_the_layout_boxes() {
    let date_text = text(b"*", &[char_run(2, 0, 14, 0, [0, 0, 0, 1])], &[para_run(2, false, 0, 1, 0)], 1, [0; 4], MASTER_BODY);
    let title_box = [-2448, -1776, 2448, -1056];
    let objects = [
        placeholder_95(1, 0, 0, title_box, Some(simple_text(b"Click to edit Master title style", title_box, MASTER_BODY))),
        placeholder_95(2, 0, 1, [-2448, -912, 2448, 1680], None),
        placeholder_95(6, 1, 2, [-2448, 1776, -1248, 2064], Some(date_text)),
        // A second title, and a placeholder of an unknown type: neither places anything.
        placeholder_95(1, 0, 5, [0, 0, 8, 8], None),
        placeholder_95(0x42, 0, 6, [0, 0, 8, 8], Some(simple_text(b"Odd", [0, 0, 8, 8], MASTER_BODY))),
        shape_95([-200, -200, 200, 200], Some(simple_text(b"Logo", [-200, -200, 200, 200], BODY))),
    ];
    let p = deck_of(vec![], &objects);
    let m = &p.masters[0];
    let on = |shapes: &[Shape], kind: PhType| shapes.iter().find(|s| s.ph_type() == Some(kind)).map(boxed);
    assert_eq!(on(&m.shapes, PhType::Title), Some((54.0, 48.0, 612.0, 90.0)));
    assert_eq!(on(&m.shapes, PhType::Body), Some((54.0, 156.0, 612.0, 324.0)));
    assert_eq!(on(&m.shapes, PhType::Date), Some((54.0, 492.0, 150.0, 36.0)));
    // The defaults' footer, which the file doesn't place, stays where it was.
    assert_eq!(on(&m.shapes, PhType::Footer), Some((0.33 * 720.0, 0.92 * 540.0, 0.34 * 720.0, 0.05 * 540.0)));
    for l in &m.layouts {
        assert_eq!(on(&l.shapes, PhType::Date), Some((54.0, 492.0, 150.0, 36.0)), "{}", l.name);
        let date = l.shapes.iter().find(|s| s.ph_type() == Some(PhType::Date)).unwrap();
        let level = body_of(date).list_style.level(0).unwrap();
        assert_eq!((level.para.align, level.run.size), (Some(Align::Center), Some(14.0)));
    }
    let own: Vec<_> = m.shapes.iter().filter(|s| s.ph.is_none()).map(|s| body_of(s).text()).collect();
    assert_eq!(own, ["Logo"]);
}

/// An empty footer on the master still gives every layout's footer its size and colour: those of
/// its end mark.
#[test]
fn an_empty_footer_keeps_its_look() {
    let empty = text(b"", &[char_run(1, 0, 10, 0, [0xFF, 0, 0, 0xFE])], &[para_run(1, false, 0, 2, 0)], 1, [0; 4], MASTER_BODY);
    let p = deck_of(vec![], &[placeholder_95(8, 1, 2, [-816, 1776, 816, 2064], Some(empty))]);
    for l in &p.masters[0].layouts {
        let footer = l.shapes.iter().find(|s| s.ph_type() == Some(PhType::Footer)).unwrap();
        let level = body_of(footer).list_style.level(0).unwrap();
        assert_eq!((level.para.align, level.run.size), (Some(Align::Right), Some(10.0)), "{}", l.name);
        assert_eq!(level.run.fill, Some(Fill::solid(ColorRef::rgb(Rgba::rgb(0xFF, 0, 0)))), "{}", l.name);
    }
}

/// The characters, a font's face and a bullet character are Windows-1252, 0x80–0x9F included.
#[test]
fn text_is_windows_1252() {
    let chars = b"caf\xE9 \x80 \x93q\x94 \x85";
    let n = chars.len() as u32 + 1;
    let body = text(chars, &[char_run(n, 3, 24, 0, [0, 0, 0, 1])], &[para_run(n, true, 0xA7, 0, 0)], 1, [0; 4], BODY);
    let p = deck_of(vec![shape_95([0, 0, 1000, 1000], Some(body))], &[]);
    let para = &body_of(&p.slides[0].shapes[0]).paragraphs[0];
    assert_eq!(para.text(), "café € \u{201C}q\u{201D} \u{2026}");
    assert_eq!(para.runs[0].props.font.as_deref(), Some("Café"));
    assert_eq!(para.props.bullet, Some(Bullet::Char { char: "§".into() }));
}

/// Damaged text never panics and stays bounded: runs past the text, huge run lengths and counts,
/// truncated entries, control characters, empty text, and a character atom past the cap.
/// Paragraphs and runs are about 4 and 2 KB each, so the document's text makes at most
/// [`super::text::MAX_TEXT_ITEMS`] of them together, across all its shapes; text past that is
/// left out.
#[test]
fn text_makes_a_bounded_number_of_paragraphs_and_runs() {
    let bx = [0, 0, 1000, 1000];
    // 40,000 one-letter paragraphs, a paragraph and a run each: 80,000 items in one shape.
    let many = [b'a', 13].repeat(40_000);
    let p = deck_of(vec![shape_95(bx, Some(simple_text(&many, bx, BODY))), shape_95(bx, Some(simple_text(b"after", bx, BODY)))], &[]);
    let items: usize = p.slides[0].shapes.iter().filter_map(|s| s.text.as_ref()).flat_map(|t| &t.paragraphs).map(|para| 1 + para.runs.len()).sum();
    assert_eq!(items, super::text::MAX_TEXT_ITEMS);
    let first = body_of(&p.slides[0].shapes[0]);
    assert_eq!((first.paragraphs.len(), first.paragraphs[0].runs[0].text.as_str()), (super::text::MAX_TEXT_ITEMS / 2, "a"));
    assert!(!texts(&p).iter().any(|t| t == "after"), "the budget is the document's, not each shape's");
}

#[test]
fn hostile_text_never_panics() {
    let bx = [0, 0, 1000, 1000];
    let one = |body: Vec<u8>| {
        let p = deck_of(vec![shape_95(bx, Some(body))], &[]);
        p.slides[0].shapes.first().and_then(|s| s.text.clone())
    };
    // Runs past the text: the text is all there, in one paragraph.
    let t = one(text(b"Hi", &[plain(1000)], &[para_run(u32::MAX, true, 0, 7, 200)], 1, [0; 4], BODY)).unwrap();
    assert_eq!(t.text(), "Hi");
    assert_eq!(t.paragraphs[0].level, 4, "the level is capped at PowerPoint 95's fifth");
    // Many zero-length runs: skipped, and the text is all there.
    let many = vec![plain(0); 100_000];
    let t = one(text(b"abc", &many, &vec![para_run(0, false, 0, 0, 0); 50_000], 1, [0; 4], BODY)).unwrap();
    assert_eq!(t.text(), "abc");
    // A truncated entry is not read; the text keeps no formatting from it.
    let mut cut = plain(4);
    cut.truncate(43);
    let t = one(text(b"abc", &[cut], &[], 1, [0; 4], BODY)).unwrap();
    assert_eq!((t.text(), t.paragraphs[0].runs[0].props.size), ("abc".into(), None));
    // Control characters are dropped; a vertical tab is a line break.
    let t = one(text(b"a\x01\x0Bb\t", &[], &[], 1, [0; 4], BODY)).unwrap();
    assert_eq!(t.text(), "a\u{b}b\t");
    // Empty text: an empty text box is not a shape, and a placeholder keeps its empty paragraphs.
    assert_eq!(one(text(b"", &[], &[], 1, [0; 4], BODY)), None);
    assert_eq!(one(text(b"\r\r", &[], &[], 1, [0; 4], BODY)), None);
    let p = deck_of(vec![placeholder_95(0x0D, 0, 1, bx, Some(text(b"\r\r", &[], &[], 1, [0; 4], BODY)))], &[]);
    assert_eq!(body_of(&p.slides[0].shapes[0]).paragraphs.len(), 3);
    // A text container with no body, and one whose text rectangle is outside the box.
    assert!(one(container(4001, 0x1E, &[rec(4000, 0, 0, &[0; 16])])).is_none());
    let t = one(text(b"x", &[], &[], u32::MAX, [i32::MIN, i32::MAX, i32::MIN, i32::MAX], BODY)).unwrap();
    assert_eq!((t.body.inset_l, t.body.inset_t), (None, None));
    // Characters past the cap are not read.
    let big = vec![b'x'; super::text::MAX_CHARS + 10];
    let t = one(text(&big, &[], &[], 1, [0; 4], BODY)).unwrap();
    assert_eq!(t.text().len(), super::text::MAX_CHARS);

    // Every word of a slide with text and placeholders set to hostile values, and cuts.
    let mut s = SlideSpec::new(0x100, 0);
    let two = text(
        b"One\rTwo",
        &[plain(4), char_run(4, 2, 40, 7, [1, 2, 3, 0xFE])],
        &[para_run(4, true, 0x95, 1, 1), para_run(4, false, 0, 2, 0)],
        0,
        [10, 10, 990, 990],
        BODY,
    );
    s.objects = vec![shape_95(bx, Some(two)), placeholder_95(0x0C, 0, 0, bx, Some(simple_text(b"T", bx, BODY)))];
    let objects = [
        placeholder_95(6, 0, 2, bx, Some(simple_text(b"*", bx, MASTER_BODY))),
        placeholder_95(2, 0, 1, bx, Some(text(b"B", &[plain(2)], &[spaced_run(2, true, 0x95, 3, 0, [150, -12, 20])], 0, [0; 4], MASTER_BODY))),
    ];
    let level = Level { spacing: [150, -36, 20], align: 3, ..LEVEL };
    let env = environment_with(&[], &[styles(&[style_atom([level; 5], 0x13), style_atom([level; 5], 0x21)])]);
    let stream = stream_with(document_atom(5760, 4320, 1), &[slide(&s)], &[master_with(MASTER, DEFAULT, [0; 4], &objects)], env, &[]);
    // Whole, it reaches the text reader: else every hostile copy below could fail early and pass.
    let p = import(&stream).unwrap();
    assert_eq!(texts(&p), ["One\nTwo", "T"]);
    let t = p.masters[0].title_style.level(0).unwrap();
    assert_eq!((t.para.line_spacing, t.para.space_before), (Some(Spacing::Pct(1.5)), Some(Spacing::Pts(36.0))));
    for at in (0..stream.len() - 4).step_by(4) {
        for v in [0u32, 1, 0x7FFF_FFFF, 0xFFFF_FFFF] {
            let mut d = stream.clone();
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
            let _ = import(&d);
        }
    }
    for cut in (0..stream.len()).step_by(3) {
        let _ = import(&stream[..cut]);
    }
}

/// A geometry record with its shape code word, adjust value, box and angle.
fn coded_geometry(code: u32, adjust: i32, bx: [i32; 4], rot: i32) -> Vec<u8> {
    let mut b = le32(&[code, adjust as u32, 0]);
    b.extend(bx.iter().chain(&[rot]).flat_map(|v| v.to_le_bytes()));
    b.extend_from_slice(&le32(&[code, 0]));
    rec(3009, 0, 0, &b)
}

/// How a test shape is drawn: line and fill colours (R, G, B, then 0xFE or a scheme slot; `None`
/// for none), the shadow's colour when it has one, its offset right and down and whether it is
/// semi-transparent, the flips the user set (bit 1 horizontal, bit 2 vertical), the line's style
/// byte (its style less one) and dash byte, and the angle the user set.
#[derive(Clone, Copy)]
struct Format {
    line: Option<[u8; 4]>,
    fill: Option<[u8; 4]>,
    shadow: Option<[u8; 4]>,
    offset: [i32; 2],
    semi: bool,
    flips: u8,
    style: u8,
    dash: u8,
    turn: i32,
}

/// PowerPoint 95's defaults: lines in slot 1, fills in slot 4, no shadow (its colour in slot 2,
/// 6 pt right and down).
const PLAIN: Format =
    Format { line: Some([0, 0, 0, 1]), fill: Some([0, 0, 0, 4]), shadow: None, offset: [48, 48], semi: false, flips: 0, style: 0, dash: 0, turn: 0 };

fn format(f: Format) -> Vec<u8> {
    let mut b = vec![0u8; 56];
    b[0] = if f.line.is_some() { 0 } else { 0xFF };
    b[1] = f.dash;
    b[3] = f.style;
    b[4..8].copy_from_slice(&f.line.unwrap_or([0, 0, 0, 1]));
    b[12] = if f.fill.is_some() { 0 } else { 0xFF };
    b[13] = 1;
    b[20..24].copy_from_slice(&f.fill.unwrap_or([0, 0, 0, 4]));
    // +24 stays slot 1 whatever the shadow is, as PowerPoint 95 writes it.
    b[24..28].copy_from_slice(&[0, 0, 0, 1]);
    b[28] = if f.shadow.is_some() { 0 } else { 0xFF };
    b[29] = if f.semi { 2 } else { 1 };
    b[30..34].copy_from_slice(&f.shadow.unwrap_or([0, 0, 0, 2]));
    b[36..40].copy_from_slice(&f.offset[0].to_le_bytes());
    b[40..44].copy_from_slice(&f.offset[1].to_le_bytes());
    b[48..52].copy_from_slice(&f.turn.to_le_bytes());
    b[52] = f.flips;
    rec(rt::FORMAT, 1, 0, &b)
}

/// A shape of `code` with its adjust value, box, angle and format, its angle less than a quarter
/// turn, so the geometry and the format agree.
fn autoshape(code: i32, adjust: i32, bx: [i32; 4], rot: i32, f: Format) -> Vec<u8> {
    drawn_shape(code, adjust, bx, rot, Format { turn: rot, ..f })
}

/// A shape whose geometry holds `drawn`, the box and angle as drawn, and whose format holds the
/// angle the user set.
fn drawn_shape(code: i32, adjust: i32, drawn: [i32; 4], rot: i32, f: Format) -> Vec<u8> {
    container(rt::SHAPE, 0x13, &[coded_geometry(code as u32, adjust, drawn, rot), format(f)])
}

/// A line from (x1, y1) to (x2, y2), with its format when given.
fn line_95(ends: [i32; 4], f: Option<Format>) -> Vec<u8> {
    arrow_line(ends, [0, 0], f)
}

/// A line with its arrowhead atom (start, end).
fn arrow_line(ends: [i32; 4], heads: [u8; 2], f: Option<Format>) -> Vec<u8> {
    let mut kids = vec![rec(rt::LINE_ATOM, 0, 0, &ends.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>()), rec(rt::ARROWHEADS, 0, 0, &heads)];
    kids.extend(f.map(format));
    container(rt::LINE, 0x13, &kids)
}

/// PowerPoint 95's trapezoid outline with bottom corners inset by `a`.
fn trapezoid_95(a: i32) -> Geom {
    let d = format!("M 0 0 L {a} 21600 L {} 21600 L 21600 0 Z", 21600 - a);
    Geom::Custom { paths: vec![craft_pptx::CustomPath { w: 21600.0, h: 21600.0, d, ..Default::default() }] }
}

fn preset_of(s: &Shape) -> (&str, &[f64]) {
    match &s.geom {
        Geom::Preset { name, adj } => (name, adj),
        Geom::Custom { .. } => ("custom", &[]),
    }
}

fn rgb(r: u8, g: u8, b: u8) -> ColorRef {
    ColorRef::rgb(Rgba::rgb(r, g, b))
}

/// Each shape code PowerPoint 95 wrote is its preset, with its adjust value in DrawingML terms (the
/// probes' adjust values are DrawingML's defaults on a square), except the trapezoid, which has its
/// own outline ([`trapezoids_are_wide_at_the_top`]); the bits above its code's low byte don't change
/// the code.
#[test]
fn shape_codes_are_presets_with_their_adjust_values() {
    let sq = [0, 0, 1152, 1152];
    let cases: [(i32, i32, &str, &[f64]); 12] = [
        (-3, 0, "rect", &[]),
        (-2, 2700, "roundRect", &[12_500.0]),
        (-1, 0, "ellipse", &[]),
        (0, 0, "diamond", &[]),
        (1, 10800, "triangle", &[50_000.0]),
        (2, 0, "rtTriangle", &[]),
        (3, 5400, "parallelogram", &[25_000.0]),
        (0x6004, 5400, "custom", &[]),
        (5, 5400, "hexagon", &[25_000.0]),
        (6, 6326, "octagon", &[6326.0 / 21600.0 * 100_000.0]),
        (7, 5400, "plus", &[25_000.0]),
        (8, 0, "star5", &[]),
    ];
    let p = deck_of(cases.iter().map(|(code, adj, ..)| autoshape(*code, *adj, sq, 0, PLAIN)).collect(), &[]);
    let shapes = &p.slides[0].shapes;
    assert_eq!(shapes.len(), 12);
    for (s, (code, _, name, adj)) in shapes.iter().zip(cases) {
        assert_eq!(preset_of(s), (name, adj), "code {code}");
        assert_eq!(boxed(s), (360.0, 270.0, 144.0, 144.0));
        assert!(!s.xfrm.unwrap().flip_v, "{name}");
        assert!(!s.text_box && s.ph.is_none());
        assert!(name == "custom" || craft_drawingml::preset::build(name, 144.0, 144.0, adj).is_some());
    }
    // An inset across a box twice as wide as it is tall is twice the share of its shorter side.
    let p = deck_of(vec![autoshape(3, 5400, [0, 0, 1152, 576], 0, PLAIN), autoshape(-2, 99_999, sq, 0, PLAIN)], &[]);
    assert_eq!(preset_of(&p.slides[0].shapes[0]), ("parallelogram", &[50_000.0][..]));
    assert_eq!(preset_of(&p.slides[0].shapes[1]), ("roundRect", &[50_000.0][..]), "a corner past half the side is half");
}

/// A shape's fill (an RGB or a slot of the slide's scheme, or none), its line colour (or none) at
/// PowerPoint 95's 1 pt, and its shadow when on.
#[test]
fn shapes_keep_their_fill_line_and_shadow() {
    let bx = [0, 0, 1152, 576];
    let red = Format { fill: Some([0xFF, 0, 0, 0xFE]), ..PLAIN };
    let blue_line = Format { line: Some([0, 0, 0xFF, 0xFE]), ..PLAIN };
    let no_fill = Format { fill: None, ..PLAIN };
    let no_line = Format { line: None, ..PLAIN };
    let shadow = Format { shadow: Some([0x80, 0x80, 0x80, 0xFE]), ..PLAIN };
    let p = deck_of([PLAIN, red, blue_line, no_fill, no_line, shadow].into_iter().map(|f| autoshape(-3, 0, bx, 0, f)).collect(), &[]);
    let s = &p.slides[0].shapes;
    assert_eq!(s.len(), 6);
    // The default: slot 4 (fills) and slot 1 (text and lines) of the slide's scheme.
    assert_eq!(s[0].fill, Some(Fill::solid(rgb(0, 0xCC, 0x99))));
    assert_eq!(s[0].line, Some(craft_pptx::Line::solid(rgb(0, 0, 0), 1.0)));
    assert_eq!(s[0].effects, None);
    assert_eq!(s[1].fill, Some(Fill::solid(rgb(0xFF, 0, 0))));
    assert_eq!(s[2].line, Some(craft_pptx::Line::solid(rgb(0, 0, 0xFF), 1.0)));
    assert_eq!(s[3].fill, Some(Fill::None));
    assert_eq!(s[3].line, s[0].line);
    assert_eq!(s[4].line, Some(craft_pptx::Line::none()));
    let sh = s[5].effects.as_ref().and_then(|e| e.outer_shadow.clone()).unwrap();
    assert_eq!((sh.color, sh.dir, sh.blur), (rgb(0x80, 0x80, 0x80), 45.0, 0.0), "the shadow's colour is at +30, not +24");
    assert!((sh.dist - 6.0 * 2f64.sqrt()).abs() < 1e-9);

    // A slot of the slide's own scheme.
    let mut spec = SlideSpec::new(0x100, 16);
    spec.scheme[4] = 0x123456;
    spec.follow_scheme = false;
    spec.objects = vec![autoshape(-1, 0, bx, 0, PLAIN)];
    let p = deck(&[spec]);
    assert_eq!(p.slides[0].shapes[0].fill, Some(Fill::solid(rgb(0x12, 0x34, 0x56))));

    // A shape with no fill, line or shadow draws nothing and is left out; with text it is a text
    // box. A filled one with text keeps the text.
    let none = Format { fill: None, line: None, ..PLAIN };
    let with_text = |f: Format| container(rt::SHAPE, 0x13, &[coded_geometry(-1i32 as u32, 0, bx, 0), format(f), simple_text(b"Hi", bx, BODY)]);
    let p = deck_of(vec![autoshape(-1, 0, bx, 0, none), with_text(none), with_text(red)], &[]);
    let s = &p.slides[0].shapes;
    assert_eq!(s.len(), 2);
    assert!(s[0].text_box && s[0].fill == Some(Fill::None));
    assert!(!s[1].text_box);
    assert_eq!((preset_of(&s[1]).0, body_of(&s[1]).text(), s[1].fill.clone()), ("ellipse", "Hi".into(), Some(Fill::solid(rgb(0xFF, 0, 0)))));
}

/// A turned shape keeps its unturned box and its angle, which PowerPoint 95 measures
/// counter-clockwise and DrawingML clockwise: 30° there is 330° here. The flips the user set don't
/// flip a shape: what is drawn is in its geometry ([`orientation_bits_turn_and_mirror`]).
#[test]
fn rotation_runs_counter_clockwise() {
    let bx = [-2304, -1584, -1152, -1008];
    let p = deck_of(
        vec![
            autoshape(-3, 0, bx, 480, PLAIN),
            autoshape(2, 0, bx, 0, Format { flips: 1, ..PLAIN }),
            autoshape(9, 10800, bx, 480, PLAIN),
            autoshape(-1, 0, bx, 0, Format { flips: 3, ..PLAIN }),
        ],
        &[],
    );
    let x: Vec<_> = p.slides[0].shapes.iter().map(|s| s.xfrm.unwrap()).collect();
    assert_eq!((x[0].x, x[0].y, x[0].w, x[0].h, x[0].rot, x[0].flip_h), (72.0, 72.0, 144.0, 72.0, 330.0, false));
    assert_eq!((x[1].rot, x[1].flip_h, x[1].flip_v), (0.0, false, false));
    assert_eq!((x[2].rot, x[2].flip_h, x[2].flip_v), (330.0, false, false), "the arrowrot30 probe: the tip moves up");
    assert_eq!((x[3].rot, x[3].flip_h, x[3].flip_v), (0.0, false, false));
}

/// A line spans the box of its ends and is flipped to run from its start to its end, in its
/// format's colour (the scheme's text and lines colour without one), never filled.
#[test]
fn lines_keep_their_ends_and_direction() {
    let blue = Format { line: Some([0, 0, 0xFF, 0xFE]), ..PLAIN };
    let p = deck_of(
        vec![
            line_95([-2304, -1584, -576, -432], Some(PLAIN)),
            line_95([-576, -1584, -2304, -432], Some(blue)),
            line_95([-576, -432, -2304, -1584], None),
            line_95([0, 0, 576, 0], Some(Format { line: None, ..PLAIN })),
        ],
        &[],
    );
    let s = &p.slides[0].shapes;
    assert_eq!(s.len(), 3, "a line with no line draws nothing");
    for l in s {
        assert_eq!(preset_of(l).0, "line");
        assert_eq!(boxed(l), (72.0, 72.0, 216.0, 144.0));
        assert_eq!(l.fill, Some(Fill::None));
    }
    let flips: Vec<_> = s.iter().map(|l| (l.xfrm.unwrap().flip_h, l.xfrm.unwrap().flip_v)).collect();
    assert_eq!(flips, [(false, false), (true, false), (true, true)]);
    assert_eq!(s[0].line, Some(craft_pptx::Line::solid(rgb(0, 0, 0), 1.0)));
    assert_eq!(s[1].line, Some(craft_pptx::Line::solid(rgb(0, 0, 0xFF), 1.0)));
    assert_eq!(s[2].line, s[0].line);
}

/// A shape code no probe showed is a rectangle with its fill, line and text, whatever the bits
/// above it.
#[test]
fn unknown_shape_codes_are_rectangles() {
    let bx = [0, 0, 576, 576];
    let red = Format { fill: Some([0xFF, 0, 0, 0xFE]), ..PLAIN };
    let mut objects: Vec<_> = [-4, 15, 0x7F, -128, 0x6015, -100].iter().map(|&c| autoshape(c, 0, bx, 0, red)).collect();
    objects.push(container(rt::SHAPE, 0x13, &[coded_geometry(42, 0, bx, 0), format(PLAIN), simple_text(b"Kept", bx, BODY)]));
    let p = deck_of(objects, &[]);
    let s = &p.slides[0].shapes;
    assert_eq!(s.len(), 7);
    for s in &s[..6] {
        assert_eq!((preset_of(s), s.fill.clone(), s.text_box), (("rect", &[][..]), Some(Fill::solid(rgb(0xFF, 0, 0))), false));
        assert_eq!(s.line, Some(craft_pptx::Line::solid(rgb(0, 0, 0), 1.0)));
        assert_eq!(boxed(s), (360.0, 270.0, 72.0, 72.0));
    }
    assert_eq!((preset_of(&s[6]).0, s[6].text_box, body_of(&s[6]).text()), ("rect", false, "Kept".into()));
}

/// Damaged shapes and lines never panic: truncated geometry, format and line records, huge
/// values, and every word of a page of them set to hostile values or cut.
#[test]
fn hostile_shapes_never_panic() {
    let bx = [0, 0, 576, 576];
    let cut = |r: Vec<u8>, keep: usize| {
        let mut r = r;
        r.truncate(16 + keep);
        r[8..12].copy_from_slice(&(keep as u32).to_le_bytes());
        r
    };
    let mut objects = vec![];
    for keep in [0, 3, 8, 13, 21, 25, 29, 53] {
        objects.push(container(rt::SHAPE, 0x13, &[coded_geometry(1, 10800, bx, 0), cut(format(PLAIN), keep)]));
        objects.push(container(rt::SHAPE, 0x13, &[cut(coded_geometry(1, 10800, bx, 0), keep), format(PLAIN)]));
        objects.push(container(rt::LINE, 0x13, &[cut(rec(rt::LINE_ATOM, 0, 0, &le32(&[0, 0, 8, 8])), keep.min(16))]));
    }
    let huge = Format {
        line: Some([0, 0, 0, 0xFD]),
        fill: Some([9, 9, 9, 200]),
        shadow: Some([1, 1, 1, 8]),
        offset: [i32::MIN, i32::MAX],
        semi: true,
        flips: 0xFF,
        style: 0xFF,
        dash: 0xFF,
        turn: i32::MIN,
    };
    objects.push(autoshape(5, i32::MAX, [i32::MIN, i32::MIN, i32::MAX, i32::MAX], i32::MIN, huge));
    objects.push(autoshape(3, i32::MIN, [0, 0, 0, 0], i32::MAX, PLAIN));
    objects.push(line_95([i32::MIN, i32::MAX, i32::MAX, i32::MIN], Some(huge)));
    let p = deck_of(objects.clone(), &[]);
    for s in &p.slides[0].shapes {
        let x = s.xfrm.unwrap();
        assert!(x.w.is_finite() && x.h.is_finite() && x.rot.is_finite());
        if let Geom::Preset { adj, .. } = &s.geom {
            assert!(adj.iter().all(|a| a.is_finite()));
        }
    }

    let mut s = SlideSpec::new(0x100, 0);
    s.objects = vec![
        autoshape(4, 5400, bx, 480, Format { shadow: Some([0, 0, 0, 2]), flips: 3, ..PLAIN }),
        arrow_line([0, 0, 576, 288], [4, 1], Some(PLAIN)),
    ];
    let stream = stream_of(document_atom(5760, 4320, 1), &[slide(&s)], &[master_with(MASTER, DEFAULT, [0; 4], &[autoshape(-1, 0, bx, 0, PLAIN)])]);
    for at in (0..stream.len() - 4).step_by(4) {
        for v in [0u32, 1, 0xFE, 0x7FFF_FFFF, 0xFFFF_FFFF] {
            let mut d = stream.clone();
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
            let _ = import(&d);
        }
    }
    for cut in 0..stream.len() {
        let _ = import(&stream[..cut]);
    }
}

/// A rectangle's angle is the one the format holds. The geometry holds the outline as drawn, whole
/// quarter-turns folded into its box: after an odd number of them its box is the turned one, so it
/// is turned back about its centre. Less than a quarter turn, or a half turn, leaves it as it is.
/// Angles run counter-clockwise, so each is negated.
#[test]
fn quarter_turns_are_turned_back() {
    // A 2 in × 1 in rectangle at 1 in, 1 in; turned a quarter, it is drawn 1 in × 2 in about the
    // same centre (as the rot90wide probe).
    let (wide, tall) = ([-2304, -1584, -1152, -1008], [-2016, -1872, -1440, -720]);
    let turned = |drawn: [i32; 4], drawn_rot: i32, turn: i32| drawn_shape(-3, 0, drawn, drawn_rot, Format { turn, ..PLAIN });
    let p = deck_of(
        vec![
            turned(wide, 480, 480),
            turned(tall, 0, 1440),
            turned(wide, 0, 2880),
            turned(tall, 0, 4320),
            turned(tall, 0, -1440),
            turned(wide, 480, 3360),
            turned(tall, 480, 1920),
            // A right triangle at 120° keeps its unturned box and the angle (the rot120wide probe).
            drawn_shape(2, 0, wide, 1920, Format { turn: 1920, ..PLAIN }),
            turned(wide, 0, 480),
        ],
        &[],
    );
    let got: Vec<_> = p.slides[0].shapes.iter().map(|s| (boxed(s), s.xfrm.unwrap().rot)).collect();
    let flat = (72.0, 72.0, 144.0, 72.0);
    assert_eq!(
        got,
        [(flat, 330.0), (flat, 270.0), (flat, 180.0), (flat, 90.0), (flat, 90.0), (flat, 150.0), (flat, 240.0), (flat, 240.0), (flat, 330.0)]
    );
}

/// For shape codes of 0 and up the byte above the code orients the outline: bits 4 and 5 turn it
/// counter-clockwise by quarters, then bit 6 mirrors it across. In DrawingML (flip first, then turn
/// clockwise) a turn by q is a clockwise turn by −q, and a mirror after it a horizontal flip turned
/// clockwise by q. After an odd number of quarters the box drawn is the turned one, and is turned
/// back. The format's angle and flips don't count for these shapes: the geometry is what is drawn.
#[test]
fn orientation_bits_turn_and_mirror() {
    // 1 in × 2 in as drawn: unturned, 2 in × 1 in.
    let tall = [-2016, -1872, -1440, -720];
    let flat = (72.0, 72.0, 144.0, 72.0);
    let upright = (108.0, 36.0, 72.0, 144.0);
    // The user's flips and angle are what PowerPoint 95 wrote for these: rtrot, rthflip, rtvflip,
    // rtboth, rthfliprot, and the arrows' up, left and down (shape14 to shape16).
    type Box = (f64, f64, f64, f64);
    let cases: [(u32, i32, u8, f64, bool, Box); 8] = [
        (0x00, 0, 0, 0.0, false, upright),
        (0x10, 1440, 1, 270.0, false, flat),
        (0x20, 0, 3, 180.0, false, upright),
        (0x30, 0, 2, 90.0, false, flat),
        (0x40, 0, 0, 0.0, true, upright),
        (0x50, 0, 0, 90.0, true, flat),
        (0x60, 1440, 1, 180.0, true, upright),
        (0x70, 0, 0, 270.0, true, flat),
    ];
    for code in [2, 9] {
        let shapes =
            cases.iter().map(|&(bits, turn, flips, ..)| drawn_shape(code | (bits << 8) as i32, 10800, tall, 0, Format { turn, flips, ..PLAIN }));
        let p = deck_of(shapes.collect(), &[]);
        assert_eq!(p.slides[0].shapes.len(), cases.len(), "code {code}");
        for (s, (bits, _, _, rot, flip_h, bx)) in p.slides[0].shapes.iter().zip(cases) {
            let x = s.xfrm.unwrap();
            assert_eq!((x.rot, x.flip_h, x.flip_v, boxed(s)), (rot, flip_h, false, bx), "code {code} bits {bits:#x}");
        }
    }
    // What is left of the angle turns after the mirror: bits 0x50 and 30° counter-clockwise is a
    // mirror turned 90° − 30° clockwise; without the mirror, 120° counter-clockwise.
    let p = deck_of(vec![drawn_shape(0x5002, 0, tall, 480, PLAIN), drawn_shape(0x1002, 0, tall, 480, PLAIN)], &[]);
    let x: Vec<_> = p.slides[0].shapes.iter().map(|s| s.xfrm.unwrap()).map(|x| (x.rot, x.flip_h, x.w, x.h)).collect();
    assert_eq!(x, [(60.0, true, 144.0, 72.0), (240.0, false, 144.0, 72.0)]);
}

/// The trapezoid's own outline, wide at the bottom, is DrawingML's `trapezoid`; PowerPoint 95's
/// new trapezoid is that mirrored and turned a half-turn (0x60), a vertical flip. Whenever the
/// transform holds a mirror it is the outline wide at the top, unflipped and turned a half-turn
/// further, so its text stays upright; its bottom corners are inset by the adjust value (up to half
/// the side). The user's flips don't count: shape8, traphflip and trapvflip.
#[test]
fn trapezoids_are_wide_at_the_top() {
    let sq = [0, 0, 1152, 1152];
    let p = deck_of(
        vec![
            autoshape(0x6004, 5400, sq, 0, PLAIN),
            autoshape(0x6004, 2700, sq, 0, Format { flips: 1, ..PLAIN }),
            autoshape(0x6004, 20_000, sq, 0, PLAIN),
            autoshape(0x6004, -5, sq, 0, PLAIN),
            autoshape(4, 5400, sq, 0, Format { flips: 2, ..PLAIN }),
            autoshape(0x2004, 5400, sq, 0, PLAIN),
            autoshape(0x4004, 5400, sq, 0, PLAIN),
            autoshape(0x6004, 5400, sq, 480, PLAIN),
            autoshape(4, 2700, [0, 0, 1152, 576], 0, PLAIN),
        ],
        &[],
    );
    let s = &p.slides[0].shapes;
    let got: Vec<_> = s.iter().map(|s| (s.geom.clone(), s.xfrm.unwrap().rot, s.xfrm.unwrap().flip_h, s.xfrm.unwrap().flip_v)).collect();
    let preset = |a: f64| Geom::Preset { name: "trapezoid".into(), adj: vec![a] };
    let want = [
        (trapezoid_95(5400), 0.0),
        (trapezoid_95(2700), 0.0),
        (trapezoid_95(10800), 0.0),
        (trapezoid_95(0), 0.0),
        (preset(25_000.0), 0.0),
        (preset(25_000.0), 180.0),
        (trapezoid_95(5400), 180.0),
        (trapezoid_95(5400), 330.0),
        // An inset across a box twice as wide as it is tall is twice the share of its shorter side.
        (preset(25_000.0), 0.0),
    ]
    .map(|(g, rot)| (g, rot, false, false));
    assert_eq!(got, want);
    assert_eq!(boxed(&s[0]), (360.0, 270.0, 144.0, 144.0));
}

/// The object model's shapes 13 to 24 are codes 9 to 14: the arrow and the thick arrow (DrawingML's
/// right arrow, its shaft half and three quarters of the height, the adjust value where its head
/// starts), the home plate (where its point starts), the cube (its depth), the balloon and the seal.
/// The adjust values and proportions are those PowerPoint 95 drew (shape13, shape17 and shape21 to
/// shape24).
#[test]
fn the_other_autoshapes_are_presets() {
    let sq = [0, 0, 1152, 1152];
    let cases: [(i32, i32, &str, &[f64]); 6] = [
        (9, 10800, "rightArrow", &[50_000.0, 50_000.0]),
        (10, 10800, "rightArrow", &[75_000.0, 50_000.0]),
        (11, 14400, "homePlate", &[100_000.0 / 3.0]),
        (12, 5400, "cube", &[25_000.0]),
        (13, 1800, "wedgeRoundRectCallout", &[-100_000.0 * 5.0 / 12.0, 70_000.0, 20_000.0]),
        (14, 0, "star16", &[]),
    ];
    let p = deck_of(cases.iter().map(|(code, adj, ..)| autoshape(*code, *adj, sq, 0, PLAIN)).collect(), &[]);
    let shapes = &p.slides[0].shapes;
    assert_eq!(shapes.len(), 6);
    for (s, (code, _, name, adj)) in shapes.iter().zip(cases) {
        let (got, got_adj) = preset_of(s);
        assert_eq!(got, name, "code {code}");
        assert_eq!(got_adj.len(), adj.len(), "code {code}");
        assert!(got_adj.iter().zip(adj).all(|(a, b)| (a - b).abs() < 1e-6), "code {code}: {got_adj:?}");
        assert!(craft_drawingml::preset::build(name, 144.0, 144.0, adj).is_some());
        // The balloon's box is its body, the top five sixths.
        let want = if code == 13 { (360.0, 270.0, 144.0, 120.0) } else { (360.0, 270.0, 144.0, 144.0) };
        assert_eq!(boxed(s), want, "code {code}");
    }
    // A head across a box twice as wide as it is tall is twice the share of its shorter side; an
    // adjust value past the side is clamped.
    let wide = [0, 0, 1152, 576];
    let p = deck_of(vec![autoshape(9, 10800, wide, 0, PLAIN), autoshape(11, -7, wide, 0, PLAIN), autoshape(12, 99_999, sq, 0, PLAIN)], &[]);
    let s = &p.slides[0].shapes;
    assert_eq!(preset_of(&s[0]), ("rightArrow", &[50_000.0, 100_000.0][..]));
    assert_eq!(preset_of(&s[1]), ("homePlate", &[200_000.0][..]));
    assert_eq!(preset_of(&s[2]), ("cube", &[100_000.0][..]));
    // A balloon turned a quarter counter-clockwise: its body is left of the square's centre.
    let p = deck_of(vec![autoshape(0x100D, 1800, sq, 0, PLAIN)], &[]);
    let x = p.slides[0].shapes[0].xfrm.unwrap();
    assert_eq!((x.rot, x.w, x.h), (270.0, 144.0, 120.0));
    assert!((x.x + x.w / 2.0 - 420.0).abs() < 1e-9 && (x.y + x.h / 2.0 - 342.0).abs() < 1e-9, "{x:?}");
}

/// A shadow falls as far as the format's +36 and +40 say (master units, right and down; 6 pt each
/// by default), as a distance and a direction clockwise from the right, and is half transparent
/// when +29 is 2 (shadow, shadowoff1, shadowoff2). An absurd offset is clamped to 10 in.
#[test]
fn shadows_keep_their_offset_and_transparency() {
    let bx = [0, 0, 576, 576];
    let on = |offset: [i32; 2], semi: bool| autoshape(-3, 0, bx, 0, Format { shadow: Some([0, 0, 0, 0xFE]), offset, semi, ..PLAIN });
    let p = deck_of(vec![on([48, 48], false), on([24, 80], false), on([-40, -16], true), on([i32::MIN, i32::MAX], false), on([0, 0], false)], &[]);
    let got: Vec<_> = p.slides[0].shapes.iter().map(|s| s.effects.as_ref().and_then(|e| e.outer_shadow.clone()).unwrap()).collect();
    let want = [(6.0, 6.0), (3.0, 10.0), (-5.0, -2.0), (-720.0, 720.0), (0.0, 0.0)];
    assert_eq!(got.len(), want.len());
    for (sh, (dx, dy)) in got.iter().zip(want) {
        let (x, y) = (sh.dist * sh.dir.to_radians().cos(), sh.dist * sh.dir.to_radians().sin());
        assert!((x - dx).abs() < 1e-9 && (y - dy).abs() < 1e-9, "{dx}, {dy}: {} at {}", sh.dist, sh.dir);
        assert!((0.0..360.0).contains(&sh.dir));
    }
    assert_eq!(got[0].color, rgb(0, 0, 0));
    assert_eq!(got[2].color, rgb(0, 0, 0).with(craft_pptx::ColorTransform::Alpha(50_000)));
}

/// The line's style sets its width, 1 to 10 pt; its dash style dots or dashes it. Values no probe
/// showed are a 1 pt solid line.
#[test]
fn lines_keep_their_width_and_dashes() {
    let bx = [0, 0, 576, 576];
    let styles = [0u8, 1, 2, 3, 4, 5, 6, 0xFF];
    let p = deck_of(styles.iter().map(|&style| autoshape(-3, 0, bx, 0, Format { style, ..PLAIN })).collect(), &[]);
    let widths: Vec<_> = p.slides[0].shapes.iter().map(|s| s.line.as_ref().and_then(|l| l.width)).collect();
    assert_eq!(widths, [1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 1.0, 1.0].map(Some));

    let dashes = [0u8, 1, 2, 3, 4, 5, 0xFF];
    let mut objects: Vec<_> = dashes.iter().map(|&dash| autoshape(-3, 0, bx, 0, Format { dash, ..PLAIN })).collect();
    objects.push(line_95([0, 0, 576, 0], Some(Format { dash: 2, style: 3, ..PLAIN })));
    let p = deck_of(objects, &[]);
    let got: Vec<_> = p.slides[0].shapes.iter().map(|s| s.line.as_ref().and_then(|l| l.dash.clone())).collect();
    let preset = |n: &str| Some(craft_pptx::style::Dash::Preset { name: n.into() });
    assert_eq!(got, [None, preset("sysDot"), preset("dash"), preset("lgDash"), preset("dashDot"), None, None, preset("dash")]);
    let line = p.slides[0].shapes[7].line.clone().unwrap();
    assert_eq!((line.width, line.fill), (Some(6.0), Some(Fill::solid(rgb(0, 0, 0)))));
}

/// A shadow is in its own colour at +30: an RGB, or a slot of the scheme (slot 2, the shadow
/// colour, by default).
#[test]
fn shadows_keep_their_colour() {
    let bx = [0, 0, 576, 576];
    let mut spec = SlideSpec::new(0x100, 16);
    spec.scheme[2] = 0x445566;
    spec.follow_scheme = false;
    spec.objects = vec![
        autoshape(-3, 0, bx, 0, Format { shadow: Some([0xFF, 0, 0, 0xFE]), ..PLAIN }),
        autoshape(-3, 0, bx, 0, Format { shadow: Some([0, 0, 0, 2]), ..PLAIN }),
        autoshape(-3, 0, bx, 0, Format { shadow: Some([0, 0, 0, 200]), fill: None, line: None, ..PLAIN }),
    ];
    let p = deck(&[spec]);
    let s = &p.slides[0].shapes;
    let colours: Vec<_> = s.iter().map(|s| s.effects.as_ref().and_then(|e| e.outer_shadow.as_ref()).map(|sh| sh.color.clone())).collect();
    assert_eq!(colours, [Some(rgb(0xFF, 0, 0)), Some(rgb(0x44, 0x55, 0x66))], "a shadow of no slot draws nothing");
}

/// A line's 3007 atom gives its start (DrawingML's head) and end (tail) arrowheads: 4 a small
/// swept-back head, 1 a larger one; other values none. A cut atom keeps what is there.
#[test]
fn lines_keep_their_arrowheads() {
    let ends = [0, 0, 1152, 0];
    let cases: [[u8; 2]; 7] = [[0, 0], [0, 4], [4, 0], [4, 4], [0, 1], [1, 9], [7, 0xFF]];
    let mut objects: Vec<_> = cases.iter().map(|&h| arrow_line(ends, h, Some(PLAIN))).collect();
    objects.push(arrow_line(ends, [4, 4], None));
    objects.push(container(rt::LINE, 0x13, &[rec(rt::LINE_ATOM, 0, 0, &le32(&[0, 0, 8, 8])), rec(rt::ARROWHEADS, 0, 0, &[4]), format(PLAIN)]));
    let p = deck_of(objects, &[]);
    let end = |kind: &str, size: &str| Some(craft_pptx::style::LineEnd { kind: kind.into(), w: size.into(), len: size.into() });
    let (none, stealth, big) = (None, end("stealth", "med"), end("triangle", "lg"));
    let got: Vec<_> = p.slides[0].shapes.iter().map(|s| s.line.as_ref().map(|l| (l.head.clone(), l.tail.clone())).unwrap()).collect();
    assert_eq!(
        got,
        [
            (none.clone(), none.clone()),
            (none.clone(), stealth.clone()),
            (stealth.clone(), none.clone()),
            (stealth.clone(), stealth.clone()),
            (none.clone(), big.clone()),
            (big, none.clone()),
            (none.clone(), none.clone()),
            (stealth.clone(), stealth.clone()),
            (stealth, none),
        ]
    );
}

/// Hostile angles, flips and line bytes never panic, and every angle is finite and in 0..360.
#[test]
fn hostile_angles_never_panic() {
    let bx = [i32::MIN, i32::MIN, i32::MAX, i32::MAX];
    let mut objects = vec![];
    for (drawn, turn) in [(i32::MIN, i32::MAX), (i32::MAX, i32::MIN), (0, i32::MIN), (i32::MAX, i32::MAX), (-1, 1)] {
        objects.push(drawn_shape(8, i32::MAX, bx, drawn, Format { turn, flips: 0xFF, style: 0x80, dash: 0x80, ..PLAIN }));
    }
    objects.push(arrow_line([i32::MIN, 0, i32::MAX, 0], [0xFF, 0x80], Some(Format { style: 0xFF, dash: 0xFF, ..PLAIN })));
    let p = deck_of(objects, &[]);
    assert_eq!(p.slides[0].shapes.len(), 6);
    for s in &p.slides[0].shapes {
        let x = s.xfrm.unwrap();
        assert!([x.x, x.y, x.w, x.h].iter().all(|v| v.is_finite()));
        assert!((0.0..360.0).contains(&x.rot), "{}", x.rot);
    }
}

/// The text of a drawn shape and of a freeform keeps its anchoring, wrap, spacing and justified
/// alignment, as a text box's does.
/// PowerPoint 95 lays a shape's text out in the box the geometry holds, turned by the text's own
/// angle (4003 +16): a shape turned a quarter before its text was typed holds its box turned and
/// draws its text upright (`wrap-before`, `rot90arrowtext`), one turned on to 120° after that draws
/// it at the 30° its geometry holds (`wrap-mix`), and one turned after its text was typed turns
/// its text with it (`wrap-after`). The insets stay on the held box's edges, as PowerPoint lays
/// out `upright` text. A balloon's box is its body, so the inset at its bottom is measured from
/// the body (`balloontext`).
#[test]
fn text_in_a_turned_box_keeps_its_own_angle() {
    // A text rectangle `insets` (left, top, right, bottom, in master units) inside `bx`, drawn at
    // `angle` (sixteenths of a degree, counter-clockwise).
    let text_in = |[l, t, r, b]: [i32; 4], [il, it, ir, ib]: [i32; 4], angle: i32| {
        let mut t = text(b"Hi", &[plain(3)], &[para_run(3, false, 0, 0, 0)], 1, [l + il, t + it, r - ir, b - ib], BODY);
        let at = t.windows(4).position(|w| w == 4003u32.to_le_bytes()).unwrap() + 16 + 16;
        t[at..at + 4].copy_from_slice(&angle.to_le_bytes());
        t
    };
    let shape = |code: i32, bx: [i32; 4], drawn: i32, turn: i32, insets: [i32; 4]| {
        container(rt::SHAPE, 0x13, &[coded_geometry(code as u32, 1800, bx, drawn), format(Format { turn, ..PLAIN }), text_in(bx, insets, drawn)])
    };
    let (tall, wide) = ([-288, -576, 288, 576], [-576, -288, 576, 288]);
    let p = deck_of(
        vec![
            // A rectangle turned a quarter counter-clockwise before its text: its box held turned.
            shape(-3, tall, 0, 1440, [66, 37, 66, 37]),
            // The same turned on to 120°: the geometry holds the 30° left over.
            shape(-3, tall, 480, 1920, [66, 37, 66, 37]),
            // An up arrow: a right arrow turned a quarter, its head at the held box's top.
            shape(9 | 0x1000, tall, 0, 0, [209, 329, 208, 37]),
            // Turned after its text: the geometry holds the box unturned and the whole angle.
            shape(-3, wide, 1440, 1440, [66, 37, 66, 37]),
            // A balloon in a 144 pt square: its body is the top 120 pt.
            shape(13, [-576, -576, 576, 576], 0, 0, [121, 91, 121, 307]),
        ],
        &[],
    );
    let got: Vec<_> = p.slides[0]
        .shapes
        .iter()
        .map(|s| {
            let b = &body_of(s).body;
            (s.xfrm.unwrap().rot, b.upright, b.rot, (b.inset_l.unwrap(), b.inset_t.unwrap(), b.inset_r.unwrap(), b.inset_b.unwrap()))
        })
        .collect();
    assert_eq!(
        got,
        [
            (270.0, Some(true), None, (8.25, 4.625, 8.25, 4.625)),
            (240.0, Some(true), Some(330.0), (8.25, 4.625, 8.25, 4.625)),
            (270.0, Some(true), None, (26.125, 41.125, 26.0, 4.625)),
            (270.0, None, None, (8.25, 4.625, 8.25, 4.625)),
            (0.0, None, None, (15.125, 11.375, 15.125, 14.375)),
        ]
    );
    // A mirrored trapezoid (orientation bits 0x40) takes a half-turn as its own outline; its text,
    // at its own angle of 0, stays upright rather than turning with it.
    let p = deck_of(vec![shape(4 | 0x4000, wide, 0, 0, [66, 37, 66, 37])], &[]);
    let (x, b) = (p.slides[0].shapes[0].xfrm.unwrap(), &body_of(&p.slides[0].shapes[0]).body);
    assert_eq!((x.rot, x.flip_h, b.upright, b.rot), (180.0, false, Some(true), None));
    // A balloon turned a quarter counter-clockwise: its body is cut from the shape's bottom, the
    // held box's right, where its tail is.
    let p = deck_of(vec![shape(13 | 0x1000, [-576, -576, 576, 576], 0, 0, [91, 121, 307, 121])], &[]);
    let b = &body_of(&p.slides[0].shapes[0]).body;
    assert_eq!((b.upright, b.inset_l, b.inset_t, b.inset_r, b.inset_b), (Some(true), Some(11.375), Some(15.125), Some(14.375), Some(15.125)));
}

#[test]
fn drawn_shapes_text_keeps_its_anchoring_and_spacing() {
    let bx = [-576, -576, 576, 576];
    let t = || text_flagged(b"A", &[plain(2)], &[spaced_run(2, false, 0, 3, 0, [150, -12, 50])], 1, bx, BODY, 1, 0x13);
    let ellipse = container(rt::SHAPE, 0x13, &[coded_geometry(2, 0, bx, 0), format(PLAIN), t()]);
    let square = [(-576, -576), (576, -576), (576, 576), (-576, 576)];
    let freeform =
        container(rt::FREEFORM, 0x13, &[freeform_atom(bx, 4, true), rec(rt::ARROWHEADS, 0, 0, &[0, 0]), format(PLAIN), points_atom(4, &square), t()]);
    let p = deck_of(vec![ellipse, freeform], &[]);
    let shapes = &p.slides[0].shapes;
    assert_eq!(shapes.len(), 2);
    for s in shapes {
        assert!(!s.text_box, "{}", s.name);
        let b = body_of(s);
        assert_eq!((b.body.anchor, b.body.anchor_ctr, b.body.wrap), (Some(Anchor::Middle), Some(false), Some(false)), "{}", s.name);
        let para = &b.paragraphs[0].props;
        assert_eq!(para.align, Some(Align::Justify), "{}", s.name);
        assert_eq!(
            (para.line_spacing, para.space_before, para.space_after),
            (Some(Spacing::Pct(1.5)), Some(Spacing::Pts(12.0)), Some(Spacing::Pct(0.5))),
            "{}",
            s.name
        );
    }
}

/// A freeform's 3017 atom: its box twice, the point count, and the closed flag in the low byte
/// of the last word, with something else above it.
fn freeform_atom(bx: [i32; 4], count: u32, closed: bool) -> Vec<u8> {
    let mut b: Vec<u8> = bx.iter().chain(&bx).flat_map(|v| v.to_le_bytes()).collect();
    b.extend_from_slice(&count.to_le_bytes());
    b.extend_from_slice(&[u8::from(closed), 0xAD, 0xB3, 0x02]);
    rec(rt::FREEFORM_ATOM, 0, 0, &b)
}

/// A 3035 atom: `count`, then the points.
fn points_atom(count: u16, points: &[(i32, i32)]) -> Vec<u8> {
    let mut b = count.to_le_bytes().to_vec();
    b.extend(points.iter().flat_map(|(x, y)| x.to_le_bytes().into_iter().chain(y.to_le_bytes())));
    rec(rt::POINTS, NONE, 0x1D, &b)
}

/// A freeform (3016) in `bx` through `points`, closed or open, with its arrowheads and format.
fn freeform_95(bx: [i32; 4], points: &[(i32, i32)], closed: bool, heads: [u8; 2], f: Option<Format>) -> Vec<u8> {
    let mut kids = vec![freeform_atom(bx, points.len() as u32, closed), rec(rt::ARROWHEADS, 0, 0, &heads)];
    kids.extend(f.map(format));
    kids.push(points_atom(points.len() as u16, points));
    container(rt::FREEFORM, 0x13, &kids)
}

/// A group (3001) of `members`.
fn group_95(members: &[Vec<u8>]) -> Vec<u8> {
    let mut list = vec![rec(2001, 0, 0, &(members.len() as u32).to_le_bytes())];
    list.extend_from_slice(members);
    container(rt::GROUP, 0x13, &[rec(3002, 0, 0, &[0xD9, 0x8A, 0x06, 0x00]), container(rt::LIST, 0xF, &list)])
}

fn path_of(s: &Shape) -> (f64, f64, &str) {
    let Geom::Custom { paths } = &s.geom else { panic!("{} is not a path", s.name) };
    (paths[0].w, paths[0].h, &paths[0].d)
}

fn group_members(s: &Shape) -> &[Shape] {
    let craft_pptx::ShapeKind::Group { children, child } = &s.kind else { panic!("{} is not a group", s.name) };
    assert_eq!(Some(*child), s.xfrm, "a group's child space is its own box");
    children
}

/// The probes' open triangle and closed square, in master units from the slide's centre.
const TRIANGLE_BOX: [i32; 4] = [-2304, -1584, -1152, -432];
const TRIANGLE: [(i32, i32); 3] = [(-2304, -1584), (-1152, -1584), (-1728, -432)];
const SQUARE: [(i32, i32); 4] = [(-2304, -1584), (-1152, -1584), (-1152, -432), (-2304, -432)];

/// A freeform is a path through its points in its box: open, unfilled and with its arrowheads;
/// closed, filled and without them.
#[test]
fn freeforms_are_paths() {
    let open = Format { fill: None, ..PLAIN };
    let p = deck_of(
        vec![
            freeform_95(TRIANGLE_BOX, &TRIANGLE, false, [0, 4], Some(open)),
            freeform_95(TRIANGLE_BOX, &SQUARE, true, [0, 4], Some(PLAIN)),
            freeform_95(TRIANGLE_BOX, &TRIANGLE, false, [1, 0], None),
        ],
        &[],
    );
    let s = &p.slides[0].shapes;
    assert_eq!(s.len(), 3);
    for f in s {
        assert!(f.name.starts_with("Freeform"), "{}", f.name);
        assert_eq!(boxed(f), (72.0, 72.0, 144.0, 144.0));
    }
    assert_eq!(path_of(&s[0]), (1152.0, 1152.0, "M 0 0 L 1152 0 L 576 1152"));
    assert_eq!(s[0].fill, Some(Fill::None));
    let line = s[0].line.clone().unwrap();
    assert_eq!((line.fill, line.head, line.tail.map(|t| t.kind)), (Some(Fill::solid(rgb(0, 0, 0))), None, Some("stealth".into())));

    assert_eq!(path_of(&s[1]), (1152.0, 1152.0, "M 0 0 L 1152 0 L 1152 1152 L 0 1152 Z"));
    assert_eq!(s[1].fill, Some(Fill::solid(rgb(0x00, 0xCC, 0x99))));
    let line = s[1].line.clone().unwrap();
    assert_eq!((line.head, line.tail), (None, None), "a closed freeform has no ends");

    // Without a format: 1 pt in the scheme's text and lines colour, unfilled.
    assert_eq!(s[2].fill, Some(Fill::None));
    let line = s[2].line.clone().unwrap();
    assert_eq!((line.width, line.head.map(|h| h.kind)), (Some(1.0), Some("triangle".into())));
}

/// A freeform reads no more points than [`super::MAX_POINTS`], nor than its record holds whatever
/// its count says; one of fewer than two points draws nothing, a closed flag on two points draws
/// it open, and a flat box still has a path space.
#[test]
fn freeform_points_are_bounded() {
    let many: Vec<(i32, i32)> = (0..10_000).map(|i| (i % 100, i / 100)).collect();
    let bx = [0, 0, 100, 100];
    let with_count = |count: u16, points: &[(i32, i32)]| {
        container(rt::FREEFORM, 0x13, &[freeform_atom(bx, u32::from(count), true), format(PLAIN), points_atom(count, points)])
    };
    let p = deck_of(
        vec![
            freeform_95(bx, &many, false, [0, 0], Some(PLAIN)),
            with_count(9, &TRIANGLE),
            with_count(2, &SQUARE),
            with_count(0, &SQUARE),
            freeform_95(bx, &[(5, 5)], true, [0, 0], Some(PLAIN)),
            freeform_95(bx, &[], true, [0, 0], Some(PLAIN)),
            container(rt::FREEFORM, 0x13, &[freeform_atom(bx, 3, true), format(PLAIN)]),
            freeform_95([0, 0, 0, 576], &[(0, 0), (0, 576)], false, [0, 0], Some(PLAIN)),
        ],
        &[],
    );
    let s = &p.slides[0].shapes;
    assert_eq!(s.len(), 4, "freeforms of fewer than two points are left out");
    let d = path_of(&s[0]).2;
    assert_eq!(d.matches('M').count() + d.matches('L').count(), super::MAX_POINTS);
    assert_eq!(path_of(&s[1]).2, "M -2304 -1584 L -1152 -1584 L -1728 -432 Z", "the count is cut to the points held");
    assert_eq!(path_of(&s[2]).2, "M -2304 -1584 L -1152 -1584", "two points are drawn open");
    assert_eq!(path_of(&s[3]), (1.0, 576.0, "M 0 0 L 0 576"));
    assert_eq!(boxed(&s[3]), (360.0, 270.0, 0.0, 72.0));
}

/// A group is one shape spanning its members, whose child space is its own box, so its members,
/// shapes, lines and freeforms alike, stay where they would be ungrouped.
#[test]
fn groups_span_their_members() {
    let red = Format { fill: Some([0xFF, 0, 0, 0xFE]), ..PLAIN };
    let members = [
        autoshape(-3, 0, [-2304, -1584, -1728, -1008], 0, red),
        line_95([-1440, -1296, -864, -1008], Some(PLAIN)),
        freeform_95(TRIANGLE_BOX, &TRIANGLE, false, [0, 0], Some(PLAIN)),
    ];
    let ungrouped = deck_of(members.to_vec(), &[]);
    let p = deck_of(vec![group_95(&members)], &[]);
    let s = &p.slides[0].shapes;
    assert_eq!(s.len(), 1);
    assert!(s[0].name.starts_with("Group"));
    let m = group_members(&s[0]);
    assert_eq!(m.iter().map(boxed).collect::<Vec<_>>(), ungrouped.slides[0].shapes.iter().map(boxed).collect::<Vec<_>>());
    assert_eq!(boxed(&m[0]), (72.0, 72.0, 72.0, 72.0));
    assert_eq!(boxed(&m[1]), (180.0, 108.0, 72.0, 36.0));
    assert_eq!(boxed(&s[0]), (72.0, 72.0, 180.0, 144.0));
    assert_eq!((preset_of(&m[0]).0, preset_of(&m[1]).0, preset_of(&m[2]).0), ("rect", "line", "custom"));
    assert_eq!(m[0].fill, Some(Fill::solid(rgb(0xFF, 0, 0))));

    // A group of nothing drawn is left out, and one in a master is the master's shape.
    let p = deck_of(vec![group_95(&[]), group_95(&[autoshape(-3, 0, [0, 0, 8, 8], 0, Format { line: None, fill: None, ..PLAIN })])], &[]);
    assert!(p.slides[0].shapes.is_empty());
    let p = deck_of(vec![], &[group_95(&members)]);
    assert_eq!(p.masters[0].shapes.iter().filter(|s| matches!(s.kind, craft_pptx::ShapeKind::Group { .. })).count(), 1);
}

/// A group in a group: the probe's rectangle and ellipse, grouped with a triangle.
#[test]
fn groups_nest() {
    let inner = group_95(&[autoshape(-3, 0, [-2304, -1584, -1728, -1008], 0, PLAIN), autoshape(-1, 0, [-1440, -1296, -864, -1008], 0, PLAIN)]);
    let p = deck_of(vec![group_95(&[inner, autoshape(1, 10800, [-576, -1584, 0, -1008], 0, PLAIN)])], &[]);
    let s = &p.slides[0].shapes;
    assert_eq!(s.len(), 1);
    let outer = group_members(&s[0]);
    assert_eq!(boxed(&s[0]), (72.0, 72.0, 288.0, 72.0));
    assert_eq!(outer.len(), 2);
    let inner = group_members(&outer[0]);
    assert_eq!(boxed(&outer[0]), (72.0, 72.0, 180.0, 72.0));
    assert_eq!(inner.iter().map(|s| preset_of(s).0).collect::<Vec<_>>(), ["rect", "ellipse"]);
    assert_eq!((preset_of(&outer[1]).0, boxed(&outer[1])), ("triangle", (288.0, 72.0, 72.0, 72.0)));
}

/// Groups nested 1000 deep read [`super::MAX_DEPTH`] of them and leave the rest out, without
/// recursing further, on a stack of 1 MiB (a Windows main thread's); members count towards the
/// page's object cap.
#[test]
fn groups_are_bounded() {
    let rect = autoshape(-3, 0, [0, 0, 576, 576], 0, PLAIN);
    let mut g = group_95(std::slice::from_ref(&rect));
    for _ in 1..1000 {
        g = group_95(&[rect.clone(), g]);
    }
    let p = std::thread::Builder::new().stack_size(1 << 20).spawn(move || deck_of(vec![g], &[])).unwrap().join().unwrap();
    let (mut depth, mut s) = (0, &p.slides[0].shapes[0]);
    loop {
        depth += 1;
        let m = group_members(s);
        match m.iter().find(|c| matches!(c.kind, craft_pptx::ShapeKind::Group { .. })) {
            Some(inner) => s = inner,
            None => break,
        }
    }
    assert_eq!(depth, super::MAX_DEPTH);

    let line = line_95([0, 0, 8, 8], None);
    let many = vec![line; super::MAX_OBJECTS + 10];
    let p = deck_of(vec![group_95(&many), line_95([0, 0, 8, 8], None)], &[]);
    let s = &p.slides[0].shapes;
    assert_eq!((s.len(), group_members(&s[0]).len()), (1, super::MAX_OBJECTS - 1));
}

/// Damaged freeforms and groups never panic: every word of a page of them set to hostile values,
/// or the page cut anywhere.
#[test]
fn hostile_freeforms_and_groups_never_panic() {
    let huge = [i32::MIN, i32::MAX, i32::MAX, i32::MIN];
    let p = deck_of(vec![freeform_95(huge, &[(i32::MIN, i32::MAX), (i32::MAX, i32::MIN), (0, 0)], true, [0xFF, 0xFF], Some(PLAIN))], &[]);
    let (w, h, d) = path_of(&p.slides[0].shapes[0]);
    assert!(w.is_finite() && h.is_finite() && !d.contains("inf") && !d.contains("NaN"));

    let mut s = SlideSpec::new(0x100, 0);
    s.objects = vec![group_95(&[
        freeform_95(TRIANGLE_BOX, &TRIANGLE, false, [4, 1], Some(PLAIN)),
        group_95(&[line_95([0, 0, 576, 288], Some(PLAIN)), autoshape(-1, 0, [0, 0, 576, 576], 0, PLAIN)]),
    ])];
    let stream = stream_of(
        document_atom(5760, 4320, 1),
        &[slide(&s)],
        &[master_with(MASTER, DEFAULT, [0; 4], &[group_95(&[freeform_95(TRIANGLE_BOX, &SQUARE, true, [0, 0], None)])])],
    );
    for at in (0..stream.len() - 4).step_by(4) {
        for v in [0u32, 1, 2, 0xBB8, 0xBC8, 0x7D0, 0x7FFF_FFFF, 0xFFFF_FFFF] {
            let mut d = stream.clone();
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
            let _ = import(&d);
        }
    }
    for cut in 0..stream.len() {
        let _ = import(&stream[..cut]);
    }
}

/// A Windows metafile without a placeable header: the standard header (type 1, 9 words, version
/// 0x300, its size in words), `records`, then the end record.
fn wmf(records: &[u8]) -> Vec<u8> {
    let words = (18 + records.len() + 6) / 2;
    let mut b = vec![1, 0, 9, 0, 0, 3];
    b.extend_from_slice(&(words as u32).to_le_bytes());
    b.extend_from_slice(&[0; 8]);
    b.extend_from_slice(records);
    b.extend_from_slice(&[3, 0, 0, 0, 0, 0]);
    b
}

/// A metafile that is told apart from others by `n`: a SETWINDOWEXT of n × n.
fn wmf_n(n: u8) -> Vec<u8> {
    wmf(&[5, 0, 0, 0, 0x0C, 0x02, n, 0, n, 0])
}

/// The document's picture list (2006): each slot's metafile record body, or an empty slot.
fn picture_list(slots: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut kids = vec![];
    for (i, s) in slots.iter().enumerate() {
        match s {
            Some(body) => {
                let header = rec(4038, 0, 0, &le32(&[0xF00, 0, 0, 12, 12]));
                kids.push(container(4028, 9, &[container(4037, 0, &[header, rec(4033, 0, 0, body)])]));
                kids.push(rec(4043, 0, 0, &[]));
            }
            None => kids.push(rec(2018, 0, 0, &[])),
        }
        kids.push(slot_end(i as u32));
    }
    container(2006, 0x17, &kids)
}

/// A picture on a slide: a rectangle at `bx` drawn with `f`, naming `slot`, cropped by `crop`
/// (left, top, right, bottom in ten-thousandths).
fn picture_95(bx: [i32; 4], slot: u32, crop: [i32; 4], f: Format) -> Vec<u8> {
    let link = container(4014, 0x1F, &[rec(4015, 0, 0, &le32(&crop.map(|v| v as u32))), container(4053, 0, &[rec(4036, 0, 0, &le32(&[slot]))])]);
    container(rt::SHAPE, 0x13, &[coded_geometry((-3i32) as u32, 0, bx, 0), format(f), link])
}

/// 1 in square at 1 in, 1 in: AddPicture(file, 1440, 1440, 1440, 1440).
const PICTURE_BOX: [i32; 4] = [-2304, -1584, -1728, -1008];

/// A deck of `slides` slide object lists over the picture list `slots`, its pictures within
/// `budget` bytes.
fn picture_deck(slides: Vec<Vec<Vec<u8>>>, slots: &[Option<Vec<u8>>], budget: usize) -> Presentation {
    let s: Vec<Vec<u8>> =
        slides.into_iter().enumerate().map(|(i, objects)| slide(&SlideSpec { objects, ..SlideSpec::new(0x100 + i as u32, 16) })).collect();
    let stream = stream_with(
        document_atom(5760, 4320, s.len() as u32),
        &s,
        &[master(MASTER, DEFAULT, [0; 4])],
        environment_with(&[picture_list(slots)], &[]),
        &[],
    );
    let document = super::document(&stream).unwrap();
    super::convert_within(&document, &AppDefaults, budget)
}

/// A picture shape as its box, media, crop and the media's bytes.
type Pictured = ((f64, f64, f64, f64), craft_pptx::MediaId, [f64; 4], Vec<u8>);

/// Each picture on slide `i`: its box, media, crop and the media's bytes.
fn pictures_on(p: &Presentation, i: usize) -> Vec<Pictured> {
    p.slides[i]
        .shapes
        .iter()
        .filter_map(|s| match &s.kind {
            ShapeKind::Picture { fill } => {
                let media = p.media.iter().find(|m| m.id == fill.media).unwrap();
                assert_eq!(media.content_type, "image/x-wmf");
                Some((boxed(s), fill.media, fill.crop, media.data.to_vec()))
            }
            _ => None,
        })
        .collect()
}

/// A picture is a picture shape in its box, showing its slot's metafile as it is, cropped as it
/// says, drawn with its format's line and fill.
#[test]
fn pictures_show_their_metafile() {
    let p = picture_deck(
        vec![vec![picture_95(PICTURE_BOX, 0, [0; 4], PLAIN), picture_95([-2160, -1512, -1728, -1008], 0, [2500, 1250, 0, -500], PLAIN)]],
        &[Some(wmf_n(12))],
        usize::MAX,
    );
    let pics = pictures_on(&p, 0);
    assert_eq!(pics.len(), 2);
    assert_eq!((pics[0].0, pics[0].2, &pics[0].3), ((72.0, 72.0, 72.0, 72.0), [0.0; 4], &wmf_n(12)));
    assert_eq!((pics[1].0, pics[1].2), ((90.0, 81.0, 54.0, 63.0), [0.25, 0.125, 0.0, -0.05]));
    let s = &p.slides[0].shapes[0];
    assert_eq!((s.fill.clone(), s.line.clone()), (Some(Fill::solid(rgb(0, 0xCC, 0x99))), Some(craft_pptx::Line::solid(rgb(0, 0, 0), 1.0))));
    assert_eq!(preset_of(s).0, "rect");
    // Through the whole import too, with the full budget.
    let stream = stream_with(
        document_atom(5760, 4320, 1),
        &[slide(&SlideSpec { objects: vec![picture_95(PICTURE_BOX, 0, [0; 4], PLAIN)], ..SlideSpec::new(0x100, 16) })],
        &[master(MASTER, DEFAULT, [0; 4])],
        environment_with(&[picture_list(&[Some(wmf_n(2))])], &[]),
        &[],
    );
    let p = import(&stream).unwrap();
    assert_eq!(pictures_on(&p, 0).iter().map(|x| x.3.clone()).collect::<Vec<_>>(), [wmf_n(2)]);
}

/// Pictures naming one slot share one media item, on one slide or many; each slot is its own.
#[test]
fn pictures_sharing_a_slot_share_media() {
    let pic = |slot| picture_95(PICTURE_BOX, slot, [0; 4], PLAIN);
    let p = picture_deck(vec![vec![pic(0), pic(1), pic(0)], vec![pic(0)]], &[Some(wmf_n(2)), Some(wmf_n(3))], usize::MAX);
    let (first, second) = (pictures_on(&p, 0), pictures_on(&p, 1));
    assert_eq!((first.len(), second.len(), p.media.len()), (3, 1, 2));
    assert_eq!((first[0].1, second[0].1), (first[2].1, first[2].1));
    assert_ne!(first[0].1, first[1].1);
    assert_eq!((first[0].3.clone(), first[1].3.clone()), (wmf_n(2), wmf_n(3)));
}

/// A picture naming a slot that is missing, empty or holds no metafile is left out; the shapes
/// beside it are kept.
#[test]
fn a_dangling_picture_is_left_out() {
    let pic = |slot| picture_95(PICTURE_BOX, slot, [0; 4], PLAIN);
    let slots = [Some(wmf_n(2)), None, Some(b"GIF89a, not a metafile".to_vec()), Some(vec![1, 0, 9, 0, 0, 3])];
    let p = picture_deck(
        vec![vec![pic(1), pic(2), pic(3), pic(7), pic(u32::MAX), autoshape(-1, 0, [0, 0, 576, 576], 0, PLAIN), pic(0)]],
        &slots,
        usize::MAX,
    );
    let shapes = &p.slides[0].shapes;
    assert_eq!(shapes.iter().map(|s| preset_of(s).0).collect::<Vec<_>>(), ["ellipse", "rect"]);
    assert_eq!(pictures_on(&p, 0).len(), 1);
}

/// A metafile record holding more than its header's size is cut to it; one holding less is kept
/// as it is; one whose header gives a size smaller than the header isn't a metafile.
#[test]
fn a_metafile_is_cut_to_its_size() {
    let mut long = wmf_n(2);
    long.extend_from_slice(b"trailing bytes");
    let mut short = wmf_n(3);
    short.truncate(short.len() - 6);
    let pic = |slot| picture_95(PICTURE_BOX, slot, [0; 4], PLAIN);
    // A header giving a size smaller than itself: not a metafile, so its picture is left out.
    let mut tiny = wmf_n(4);
    tiny[6..10].copy_from_slice(&4u32.to_le_bytes());
    let p = picture_deck(vec![vec![pic(0), pic(1), pic(2)]], &[Some(long), Some(short.clone()), Some(tiny)], usize::MAX);
    assert_eq!(pictures_on(&p, 0).into_iter().map(|x| x.3).collect::<Vec<_>>(), [wmf_n(2), short]);
}

/// All the pictures together keep to the budget: a picture that would pass it is left out, and a
/// slot read already costs nothing more when it is shown again.
#[test]
fn pictures_keep_to_the_budget() {
    let pic = |slot| picture_95(PICTURE_BOX, slot, [0; 4], PLAIN);
    let (a, b) = (wmf_n(2), wmf_n(3));
    let p = picture_deck(vec![vec![pic(0), pic(1), pic(0)]], &[Some(a.clone()), Some(b)], a.len() + 4);
    assert_eq!(pictures_on(&p, 0).into_iter().map(|x| x.3).collect::<Vec<_>>(), [a.clone(), a.clone()]);
    // The full import's budget is the PowerPoint 97 path's.
    assert_eq!(crate::blip::MAX_TOTAL, 2 << 30);
    let p = picture_deck(vec![vec![pic(0)]], &[Some(a.clone())], a.len() - 1);
    assert!(p.slides[0].shapes.is_empty() && p.media.is_empty());
}

/// A notes page (1008) inside a slide, its slide id in its 1009 atom, holding a slide image
/// placeholder (type 0x13, linked to the slide) and the notes placeholder (0x0E) with `text`.
fn notes_page(slide_id: u32, text: &[u8]) -> Vec<u8> {
    let bx = [-1440, -2448, 1440, -288];
    let image = container(4014, 0x1F, &[rec(4015, 0, 0, &[0; 16]), container(4054, 0, &[rec(4032, 0, 0, &le32(&[slide_id]))])]);
    let image = container(3010, 0x13, &[rec(3011, 0, 0, &[0x13, 0, 0, 0, 0, 0, 0, 0]), geometry(bx, 0), format(PLAIN), image]);
    let body_box = [-1584, -144, 1584, 2448];
    let body = placeholder_95(0x0E, 0, 1, body_box, Some(simple_text(text, body_box, BODY)));
    notes_with(slide_id, vec![image, body])
}

/// A notes page holding `objects`.
fn notes_with(slide_id: u32, objects: Vec<Vec<u8>>) -> Vec<u8> {
    container(rt::NOTES, 0x25, &[rec(1009, 0, 0, &le32(&[slide_id])), slide_box(true, true, true), container(rt::OBJECTS, 0x14, &objects)])
}

/// A text container's text type (its 4003 atom's +0) set to `kind`.
fn text_typed(mut text: Vec<u8>, kind: u32) -> Vec<u8> {
    let at = text.windows(4).position(|w| w == 4003u32.to_le_bytes()).unwrap();
    text[at + 16..at + 20].copy_from_slice(&kind.to_le_bytes());
    text
}

/// Each slide's notes are its own notes page's notes placeholder's text (or, without one, the text
/// of the notes text type); the notes master's text and the slide image aren't notes, and nothing
/// on a notes page becomes a slide shape.
#[test]
fn notes_land_on_their_slide() {
    let mut slides = [SlideSpec::new(0x100, 16), SlideSpec::new(0x101, 16), SlideSpec::new(0x102, 16), SlideSpec::new(0x103, 16)];
    slides[0].notes = Some(notes_page(0x100, b"First note\rsecond line"));
    slides[2].notes = Some(notes_page(0x102, b"Third"));
    // A notes page whose text is a plain shape of the notes text type.
    let bx = [-1584, -144, 1584, 2448];
    slides[3].notes = Some(notes_with(
        0x103,
        vec![shape_95(bx, Some(text_typed(simple_text(b"Typed", bx, BODY), 4))), shape_95(bx, Some(text_typed(simple_text(b"Fourth", bx, BODY), 2)))],
    ));
    slides[0].objects = vec![shape_95([0, 0, 576, 576], Some(simple_text(b"On the slide", [0, 0, 576, 576], BODY)))];
    let s: Vec<Vec<u8>> = slides.iter().map(slide).collect();
    let notes_master = notes_with(MASTER, vec![placeholder_95(0x05, 0, 1, bx, Some(simple_text(b"Master notes", bx, MASTER_BODY)))]);
    let p = import(&stream_with(document_atom(5760, 4320, 4), &s, &[master(MASTER, DEFAULT, [0; 4])], environment(), &[notes_master])).unwrap();
    assert_eq!(p.slides.iter().map(|s| s.notes.text()).collect::<Vec<_>>(), ["First note\nsecond line", "", "Third", "Fourth"]);
    assert_eq!(p.slides[0].notes.paragraphs[0].runs[0].props.size, Some(24.0));
    assert_eq!(texts(&p), ["On the slide"]);
    assert!(p.slides[1..].iter().all(|s| s.shapes.is_empty()));

    // Text of the notes type before the notes placeholder: the placeholder's text is the notes.
    let mut spec = SlideSpec::new(0x100, 16);
    spec.notes = Some(notes_with(
        0x100,
        vec![
            shape_95(bx, Some(text_typed(simple_text(b"Typed first", bx, BODY), 2))),
            placeholder_95(0x0E, 0, 1, bx, Some(simple_text(b"From the placeholder", bx, BODY))),
        ],
    ));
    let p = import(&stream_of(document_atom(5760, 4320, 1), &[slide(&spec)], &[master(MASTER, DEFAULT, [0; 4])])).unwrap();
    assert_eq!(p.slides[0].notes.text(), "From the placeholder");
}

/// Notes paragraphs keep their spacing and justified alignment, as a slide's do.
#[test]
fn notes_keep_their_spacing_and_alignment() {
    let bx = [-1584, -144, 1584, 2448];
    let t = text_flagged(b"Note", &[plain(5)], &[spaced_run(5, false, 0, 3, 0, [150, -12, 50])], 1, bx, BODY, 5, 0x13);
    let mut s = SlideSpec::new(0x100, 16);
    s.notes = Some(notes_with(0x100, vec![placeholder_95(0x0E, 0, 1, bx, Some(t))]));
    let p = import(&stream_of(document_atom(5760, 4320, 1), &[slide(&s)], &[master(MASTER, DEFAULT, [0; 4])])).unwrap();
    let para = &p.slides[0].notes.paragraphs[0].props;
    assert_eq!(
        (para.align, para.line_spacing, para.space_before, para.space_after),
        (Some(Align::Justify), Some(Spacing::Pct(1.5)), Some(Spacing::Pts(12.0)), Some(Spacing::Pct(0.5)))
    );
}

/// Damaged pictures and notes never panic: every word of a deck of them set to hostile values (huge
/// sizes, bad slot numbers, odd types), or the deck cut anywhere.
#[test]
fn hostile_pictures_and_notes_never_panic() {
    let mut s = SlideSpec::new(0x100, 16);
    s.objects = vec![picture_95(PICTURE_BOX, 0, [2500, 0, -20000, i32::MAX], PLAIN), group_95(&[picture_95(PICTURE_BOX, 1, [0; 4], PLAIN)])];
    s.notes = Some(notes_page(0x100, b"Note"));
    let stream = stream_with(
        document_atom(5760, 4320, 1),
        &[slide(&s)],
        &[master(MASTER, DEFAULT, [0; 4])],
        environment_with(&[picture_list(&[Some(wmf_n(2)), None, Some(wmf_n(3))])], &[]),
        &[],
    );
    for at in (0..stream.len() - 4).step_by(2) {
        for v in [0u32, 1, 2, 9, 0xFAB, 0xFB5, 0xFC4, 0x3F0, 0x7FFF_FFFF, 0xFFFF_FFFF] {
            let mut d = stream.clone();
            d[at..at + 4].copy_from_slice(&v.to_le_bytes());
            if let Ok(p) = import(&d) {
                for s in &p.slides {
                    for x in s.shapes.iter().filter_map(|x| x.xfrm) {
                        assert!(x.w.is_finite() && x.h.is_finite());
                    }
                }
            }
        }
    }
    for cut in 0..stream.len() {
        let _ = import(&stream[..cut]);
    }
}

/// The presets of the object model's shape types 1 to 12, as PowerPoint 95 rendered them ("custom" for
/// the trapezoid, which has its own outline).
const SHAPE_PRESETS: [&str; 12] =
    ["rect", "roundRect", "ellipse", "diamond", "triangle", "rtTriangle", "parallelogram", "custom", "hexagon", "octagon", "plus", "star5"];

/// What one PowerPoint 95 sample shows, by its case name ([`powerpoint_95_samples`]).
fn check_sample(name: &str, p: &Presentation) {
    // Every master places the defaults' title and body, and the footer row on every layout.
    let m = &p.masters[0];
    let on = |shapes: &[Shape], kind: PhType| shapes.iter().find(|s| s.ph_type() == Some(kind)).map(boxed);
    assert_eq!(on(&m.shapes, PhType::Title), Some((54.0, 48.0, 612.0, 90.0)), "{name}");
    assert_eq!(on(&m.layouts[0].shapes, PhType::SlideNum), on(&m.shapes, PhType::SlideNum), "{name}");
    // The text cases' slide text: the first slide's text box, or the second slide's placeholders.
    let shapes = |i: usize| &p.slides[i].shapes;
    let paras = |i: usize, j: usize| &body_of(&shapes(i)[j]).paragraphs;
    let first_run = |i: usize, j: usize| &paras(i, j)[0].runs[0];
    match name {
        // "AB\rCD" at 40 pt with A and C at 24; at 24 with the first CR at 40 (Font.Points).
        "endmark" | "endmark2" => {
            let ends: Vec<_> = body_of(&p.slides[0].shapes[0]).paragraphs.iter().map(|q| q.end_props.size).collect();
            let want = if name == "endmark" { [Some(40.0), Some(40.0)] } else { [Some(40.0), Some(24.0)] };
            assert_eq!(ends, want, "{name}");
        }
        "twoslides" => assert_eq!(p.slides.len(), 2),
        "title" => {
            assert_eq!(layout_kind(p, 1), LayoutType::Title);
            assert_eq!(
                shapes(1).iter().map(|s| (s.ph_type(), body_of(s).text())).collect::<Vec<_>>(),
                [(Some(PhType::CtrTitle), "Title text".into()), (Some(PhType::SubTitle), "Sub text".into())]
            );
        }
        "layout3" => assert_eq!(layout_kind(p, 1), LayoutType::TwoContent),
        "background" => assert_eq!(p.slides[0].background, solid(0xFF, 0, 0)),
        // FollowMasterObjects turned off on the slide.
        "nomasterobj" => assert!(!p.slides[0].show_master_shapes),
        // The Slide Color Scheme dialog's Apply (this slide only): the slide keeps its own scheme.
        "schemeapply" => assert!(p.slides[0].show_master_shapes),
        "bullets" | "indent3" => {
            assert_eq!(body_of(&shapes(1)[0]).text(), "Head");
            assert_eq!(paras(1, 1).iter().map(|p| p.level).collect::<Vec<_>>(), [0, 1, 0], "{name}");
            assert_eq!(paras(1, 1)[1].props.bullet, Some(Bullet::Char { char: "–".into() }), "{name}");
        }
        "flat3" => assert!(paras(1, 1).iter().all(|p| (p.level, &p.props.bullet) == (0, &Some(Bullet::Char { char: "•".into() })))),
        "bulletchar" => assert!(paras(1, 1).iter().all(|p| p.props.bullet == Some(Bullet::Char { char: "*".into() }))),
        "nobullet" => assert!(paras(1, 1).iter().all(|p| p.props.bullet == Some(Bullet::None))),
        "margins" => {
            let p = &paras(1, 1)[0].props;
            assert_eq!((p.margin_left, p.indent), (Some(72.0), Some(-36.0)));
        }
        "bold" | "italic" | "underline" => {
            let runs = &paras(0, 0)[0].runs;
            assert_eq!(runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), [&body_of(&shapes(0)[0]).text()[..4], " plain"]);
            let flags = |r: &craft_pptx::Run| (r.props.bold, r.props.italic, r.props.underline.as_deref() == Some("sng"));
            let on = match name {
                "bold" => (Some(true), Some(false), false),
                "italic" => (Some(false), Some(true), false),
                _ => (Some(false), Some(false), true),
            };
            assert_eq!((flags(&runs[0]), flags(&runs[1])), (on, (Some(false), Some(false), false)), "{name}");
        }
        "points40" => assert_eq!(first_run(0, 0).props.size, Some(40.0)),
        "fontname" => assert_eq!(first_run(0, 0).props.font.as_deref(), Some("Courier New")),
        "fontcolor" => assert_eq!(first_run(0, 0).props.fill, Some(Fill::solid(ColorRef::rgb(Rgba::rgb(0xFF, 0, 0))))),
        "center" | "left" | "right" => {
            let align =
                [("left", Align::Left), ("center", Align::Center), ("right", Align::Right)].into_iter().find(|(n, _)| *n == name).map(|(_, a)| a);
            assert_eq!(paras(0, 0)[0].props.align, align, "{name}");
        }
        "align" => assert_eq!(paras(0, 0).iter().map(|p| p.props.align).collect::<Vec<_>>(), [Some(Align::Left), Some(Align::Right)]),
        "twopara" => assert_eq!(body_of(&shapes(0)[0]).text(), "One\nTwo"),
        _ => check_text_extras(name, p),
    }
}

/// The round 8 samples: spacing, justified text, anchoring, word wrap and the master's styles.
fn check_text_extras(name: &str, p: &Presentation) {
    let m = &p.masters[0];
    let shapes = |i: usize| &p.slides[i].shapes;
    let paras = |i: usize, j: usize| &body_of(&shapes(i)[j]).paragraphs;
    match name {
        "ls15" | "ls2" | "lspts" | "sb1" | "sbpts" | "sa05" | "sapts" => {
            let p = &paras(0, 0)[0].props;
            let want = match name {
                "ls15" => (Spacing::Pct(1.5), Spacing::Pct(0.5), Spacing::Pct(0.0)),
                "ls2" => (Spacing::Pct(2.0), Spacing::Pct(0.5), Spacing::Pct(0.0)),
                "lspts" => (Spacing::Pts(36.0), Spacing::Pct(0.5), Spacing::Pct(0.0)),
                "sb1" => (Spacing::Pct(1.0), Spacing::Pct(1.0), Spacing::Pct(0.0)),
                "sbpts" => (Spacing::Pct(1.0), Spacing::Pts(12.0), Spacing::Pct(0.0)),
                "sa05" => (Spacing::Pct(1.0), Spacing::Pct(0.5), Spacing::Pct(0.5)),
                _ => (Spacing::Pct(1.0), Spacing::Pct(0.5), Spacing::Pts(6.0)),
            };
            assert_eq!((p.line_spacing, p.space_before, p.space_after), (Some(want.0), Some(want.1), Some(want.2)), "{name}");
        }
        "justify" => assert!(paras(0, 0).iter().all(|p| p.props.align == Some(Align::Justify))),
        "atop" | "amid" | "abot" | "atopc" | "amidc" | "abotc" | "btop" | "bmid" | "bbot" | "bmidc" => {
            let s = if name.starts_with('a') { &shapes(0)[0] } else { &shapes(1)[1] };
            let b = &body_of(s).body;
            let v = if name.contains("mid") {
                Anchor::Middle
            } else if name.contains("bot") {
                Anchor::Bottom
            } else {
                Anchor::Top
            };
            assert_eq!((b.anchor, b.anchor_ctr), (Some(v), Some(name.ends_with('c'))), "{name}");
        }
        "anowrap" => assert_eq!(body_of(&shapes(0)[0]).body.wrap, Some(false)),
        "gmbase" | "gmtitle" | "gmtleft" | "gmbsize" | "gmbjust" | "gmbspace" => {
            let (t, b) = (m.title_style.level(0).unwrap(), m.body_style.level(1).unwrap());
            let title = if name == "gmtitle" { (Some(54.0), Some(true)) } else { (Some(44.0), Some(false)) };
            assert_eq!((t.run.size, t.run.bold), title, "{name}");
            assert_eq!(t.para.align, Some(if name == "gmtleft" { Align::Left } else { Align::Center }), "{name}");
            assert_eq!(b.run.size, Some(if name == "gmbsize" { 36.0 } else { 28.0 }), "{name}");
            assert_eq!(b.para.bullet, Some(Bullet::Char { char: "–".into() }), "{name}");
            assert_eq!(b.para.align, Some(if name == "gmbjust" { Align::Justify } else { Align::Left }), "{name}");
            let spacing = if name == "gmbspace" { (Spacing::Pct(1.5), Spacing::Pts(6.0)) } else { (Spacing::Pct(1.0), Spacing::Pct(0.0)) };
            assert_eq!((b.para.line_spacing, b.para.space_after), (Some(spacing.0), Some(spacing.1)), "{name}");
            assert_eq!(b.para.space_before, Some(Spacing::Pct(0.2)), "{name}");
            // The slide's body carries the same formatting as the master's level.
            let level2 = &paras(1, 1)[1];
            assert_eq!((level2.level, level2.runs[0].props.size), (1, b.run.size), "{name}");
            assert_eq!(level2.props.line_spacing, b.para.line_spacing, "{name}");
        }
        "textbox" => {
            let s = &shapes(0)[0];
            assert_eq!((body_of(s).text(), s.text_box, s.ph.is_none()), ("Box".into(), true, true));
            // The automation reported 1295, 1367.5, 4610 × 720 twips.
            assert_eq!(boxed(s), (64.75, 68.375, 230.5, 36.0));
        }
        "picture" | "jpeg" | "png" | "picwmf" | "piccrop" | "piccrop2" | "twopics" | "picshare" | "notes" | "notes2" => check_media_sample(name, p),
        _ => check_shape_sample(name, p),
    }
}

/// What one PowerPoint 95 picture or notes sample shows, by its case name.
fn check_media_sample(name: &str, p: &Presentation) {
    let notes: Vec<String> = p.slides.iter().map(|s| s.notes.text()).collect();
    // Each picture on a slide: its box, media, crop, and whether it has the default black line.
    let pictures = |i: usize| -> Vec<_> {
        p.slides[i]
            .shapes
            .iter()
            .map(|s| {
                let ShapeKind::Picture { fill } = &s.kind else { panic!("{name}: {:?} is not a picture", s.name) };
                let media = p.media.iter().find(|m| m.id == fill.media).unwrap();
                assert_eq!((media.content_type.as_str(), &media.data[..4]), ("image/x-wmf", &[1, 0, 9, 0][..]), "{name}");
                assert_eq!(s.line, Some(craft_pptx::Line::solid(rgb(0, 0, 0), 1.0)), "{name}");
                (boxed(s), fill.media, fill.crop, media.data.len())
            })
            .collect()
    };
    // AddPicture(file, 1440, 1440, 1440, 1440) in twips: 1 in square at 1 in, 1 in.
    let inch = (72.0, 72.0, 72.0, 72.0);
    match name {
        "picture" | "picwmf" | "jpeg" | "png" => {
            let pics = pictures(0);
            // The BMP's metafile is 128 bytes, the JPEG's and PNG's 220.
            let len = if name == "jpeg" || name == "png" { 220 } else { 128 };
            assert_eq!((pics.len(), pics[0].0, pics[0].2, pics[0].3, p.media.len()), (1, inch, [0.0; 4], len, 1), "{name}");
        }
        "piccrop" => {
            // CropLeft 0.25 and CropTop 0.125 of the inch square: what is left of it, where it was.
            let pics = pictures(0);
            assert_eq!((pics.len(), pics[0].0, pics[0].2), (1, (90.0, 81.0, 54.0, 63.0), [0.25, 0.125, 0.0, 0.0]));
        }
        "piccrop2" => {
            // CropRight 0.3 and CropBottom 0.05: the crop's last two values, in that order.
            let pics = pictures(0);
            assert_eq!((pics.len(), pics[0].0, pics[0].2), (1, (72.0, 72.0, 50.375, 68.375), [0.0, 0.0, 0.3, 0.05]));
        }
        "twopics" => {
            let pics = pictures(0);
            assert_eq!(pics.iter().map(|x| (x.0, x.3)).collect::<Vec<_>>(), [(inch, 128), ((216.0, 72.0, 144.0, 72.0), 220)]);
            assert_ne!(pics[0].1, pics[1].1);
        }
        "picshare" => {
            // One file inserted three times, on two slides: one picture.
            let (first, second) = (pictures(0), pictures(1));
            assert_eq!((first.len(), second.len(), p.media.len()), (2, 1, 1));
            assert_eq!((first[0].0, first[1].0, second[0].0), (inch, (216.0, 72.0, 36.0, 36.0), inch));
            assert!(first[1].1 == first[0].1 && second[0].1 == first[0].1);
        }
        "notes" => assert_eq!(notes, ["Speaker note"]),
        "notes2" => assert_eq!(notes, ["Note one", "Note two", ""]),
        _ => {}
    }
    if name.starts_with("notes") {
        assert!(p.slides.iter().all(|s| s.shapes.is_empty()), "{name}: the notes page's placeholders aren't slide shapes");
    }
}

/// The text of the PowerPoint 95 samples with text in a turned shape or a balloon: its insets and
/// whether it is drawn upright; false for any other sample.
fn check_text_frame_sample(name: &str, shapes: &[Shape]) -> bool {
    // AddShape(n, 1440, 1440, 2880, 1440) in twips.
    let wide = (72.0, 72.0, 144.0, 72.0);
    let insets = |s: &Shape| {
        let b = &body_of(s).body;
        (b.inset_l.unwrap(), b.inset_t.unwrap(), b.inset_r.unwrap(), b.inset_b.unwrap())
    };
    let upright = |s: &Shape| (body_of(s).body.upright, body_of(s).body.rot);
    match name {
        // A rectangle with text, then the same turned 90° after its text was typed: PowerPoint 95
        // keeps the box unturned and the text turns with it (4003 +16), insets 66 × 37 units.
        "recttext" | "rot90text" => {
            assert_eq!((boxed(&shapes[0]), insets(&shapes[0])), (wide, (8.25, 4.625, 8.25, 4.625)), "{name}");
        }
        // Turned 90° before its text was typed: the box and the text rectangle are held turned and
        // the text is drawn upright, its insets on the held box's edges.
        "rot90thentext" => {
            let x = shapes[0].xfrm.unwrap();
            assert_eq!((x.rot, x.w, x.h), (270.0, 144.0, 72.0));
            assert_eq!((upright(&shapes[0]), insets(&shapes[0])), ((Some(true), None), (8.25, 4.625, 8.25, 4.625)));
        }
        // An up arrow with text: a right arrow turned a quarter, its head at the held box's top
        // (329 units in), its text upright.
        "rot90arrowtext" | "wrap-arrow" => {
            assert_eq!((upright(&shapes[0]), insets(&shapes[0])), ((Some(true), None), (26.125, 41.125, 26.0, 4.625)), "{name}");
        }
        // Wrapping text (the Text Anchor dialog's word wrap): drawn upright in the held box when
        // turned before it was typed, turned with the shape when turned after, and at the 30° the
        // geometry holds when turned a quarter, typed, then turned on to 120° (`rotmix2` likewise).
        // Turned 30°, typed, then on to 90° (`rotmix`), the geometry holds the box unturned and the
        // whole angle, and the text turns with the shape.
        "wrap-none" | "wrap-after" | "rotmix" | "rot30text" => assert_eq!(upright(&shapes[0]), (None, None), "{name}"),
        "wrap-before" => assert_eq!(upright(&shapes[0]), (Some(true), None)),
        // Trapezoids with text, turned or flipped before or after it was typed: the shape ends at the
        // text's own angle, upright or upside down as PowerPoint 95 drew it, so nothing more is set.
        "traptext" | "trap180before" | "traptexthflip" | "traptextvflip" | "traptexthflipafter" | "trap180after" | "traptextvflipafter" => {
            let rot = if matches!(name, "trap180after" | "traptextvflipafter") { 180.0 } else { 0.0 };
            assert_eq!((shapes[0].xfrm.unwrap().rot, upright(&shapes[0])), (rot, (None, None)), "{name}");
        }
        "wrap-mix" | "rotmix2" => assert_eq!(upright(&shapes[0]), (Some(true), Some(330.0)), "{name}"),
        // A balloon's box is its body (the square's top 120 of 144 pt); its text's bottom edge,
        // 307 units above the square's, is 14.375 pt above the body's.
        "balloontext" => {
            assert_eq!(boxed(&shapes[0]), (288.0, 198.0, 144.0, 120.0));
            assert_eq!(insets(&shapes[0]), (15.125, 11.375, 15.125, 14.375));
        }
        _ => return false,
    }
    true
}

/// What one PowerPoint 95 shape sample shows, by its case name.
fn check_shape_sample(name: &str, p: &Presentation) {
    // Every sample comes here; one with no slide has no shapes to check.
    let Some(shapes) = p.slides.first().map(|s| &s.shapes) else { return };
    let flips = |s: &Shape| (s.xfrm.unwrap().flip_h, s.xfrm.unwrap().flip_v);
    // AddShape(n, 5760, 3960, 2880, 2880) in twips: 2 in square at 4 in, 2.75 in.
    let square = (288.0, 198.0, 144.0, 144.0);
    // AddShape(n, 1440, 1440, 2880, 1440) in twips.
    let wide = (72.0, 72.0, 144.0, 72.0);
    if check_text_frame_sample(name, shapes) {
        return;
    }
    match name {
        "base" => assert!(shapes.is_empty()),
        "shapes" => {
            let names: Vec<_> = shapes.iter().map(|s| preset_of(s).0).collect();
            assert_eq!(names, SHAPE_PRESETS);
        }
        _ if name.strip_prefix("shape").and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| (1..=12).contains(&n)) => {
            let n: usize = name[5..].parse().unwrap();
            let s = &shapes[0];
            assert_eq!((shapes.len(), preset_of(s).0), (1, SHAPE_PRESETS[n - 1]), "{name}");
            if n == 8 {
                // Wide at the top, as PowerPoint 95 drew it, with no flip of its own.
                assert_eq!(s.geom, trapezoid_95(5400));
            }
            assert_eq!(flips(s), (false, false), "{name}");
            assert_eq!(boxed(s), square, "{name}");
        }
        _ if name.strip_prefix("shape").and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| (13..=24).contains(&n)) => {
            let n: usize = name[5..].parse().unwrap();
            let s = &shapes[0];
            let kind = ["homePlate", "cube", "wedgeRoundRectCallout", "star16"]
                .get(n.saturating_sub(21))
                .filter(|_| n > 20)
                .copied()
                .unwrap_or("rightArrow");
            assert_eq!((shapes.len(), preset_of(s).0), (1, kind), "{name}");
            // Right, up, left and down: the up arrow turned a quarter counter-clockwise, the left one
            // mirrored, the down one turned three quarters and mirrored.
            let (rot, flip_h) = match n {
                14 | 18 => (270.0, false),
                15 | 19 => (0.0, true),
                16 | 20 => (270.0, true),
                _ => (0.0, false),
            };
            assert_eq!((s.xfrm.unwrap().rot, flips(s)), (rot, (flip_h, false)), "{name}");
            let want = if n == 23 { (288.0, 198.0, 144.0, 120.0) } else { square };
            assert_eq!(boxed(s), want, "{name}");
        }
        "vflip" => {
            // AddShape(5) is the isosceles triangle; flipped, it is drawn mirrored and turned a half.
            let s = &shapes[0];
            assert_eq!((shapes.len(), preset_of(s).0, boxed(s), flips(s), s.xfrm.unwrap().rot), (1, "triangle", wide, (true, false), 180.0));
        }
        "rectvflip" | "rthflip" | "rtvflip" | "rtboth" | "rtrot" | "rtrotb" | "rthfliprot" | "trapvflip" | "traphflip" => {
            let s = &shapes[0];
            let kind = match name {
                "rectvflip" => "rect",
                "traphflip" => "custom",
                "trapvflip" => "trapezoid",
                _ => "rtTriangle",
            };
            assert_eq!((shapes.len(), preset_of(s).0, boxed(s), flips(s)), (1, kind, square, (false, false)), "{name}");
            if kind == "custom" {
                assert_eq!(s.geom, trapezoid_95(5400));
            }
            // The right triangle's flips and turns are all turns of it.
            let rot = match name {
                "rthflip" | "rtrot" | "rtrotb" => 270.0,
                "rtboth" | "rthfliprot" => 180.0,
                "rtvflip" => 90.0,
                _ => 0.0,
            };
            assert_eq!(s.xfrm.unwrap().rot, rot, "{name}");
        }
        "rot120wide" => {
            let s = &shapes[0];
            assert_eq!((shapes.len(), preset_of(s).0, boxed(s), s.xfrm.unwrap().rot), (1, "rtTriangle", wide, 240.0));
        }
        "arrowrot30" => {
            let s = &shapes[0];
            assert_eq!((shapes.len(), preset_of(s).0, boxed(s), s.xfrm.unwrap().rot), (1, "rightArrow", (288.0, 198.0, 144.0, 72.0), 330.0));
        }
        "shadowoff1" | "shadowoff2" => {
            let s = &shapes[0];
            let sh = s.effects.as_ref().and_then(|e| e.outer_shadow.clone()).unwrap();
            let (dx, dy) = (sh.dist * sh.dir.to_radians().cos(), sh.dist * sh.dir.to_radians().sin());
            // 3 pt right and 10 pt down; 5 pt left, 2 pt up and semi-transparent.
            let (want, semi) = if name == "shadowoff1" { ((3.0, 10.0), false) } else { ((-5.0, -2.0), true) };
            assert!((dx - want.0).abs() < 1e-9 && (dy - want.1).abs() < 1e-9, "{name}: {dx}, {dy}");
            assert_eq!(sh.color.mods.contains(&craft_pptx::ColorTransform::Alpha(50_000)), semi, "{name}");
            assert_eq!((preset_of(s).0, boxed(s)), ("rect", (144.0, 144.0, 216.0, 144.0)), "{name}");
        }
        _ => check_drawn_sample(name, shapes),
    }
}

/// What one PowerPoint 95 sample of a rectangle's or line's look shows, by its case name.
fn check_drawn_sample(name: &str, shapes: &[Shape]) {
    let flips = |s: &Shape| (s.xfrm.unwrap().flip_h, s.xfrm.unwrap().flip_v);
    let black = rgb(0, 0, 0);
    // AddShape(n, 1440, 1440, 2880, 1440) in twips.
    let wide = (72.0, 72.0, 144.0, 72.0);
    match name {
        "rect" | "fill" | "fill2" | "linecolor" | "nofill" | "shadow" | "rotate" | "flip" | "lineoff" | "shadowcolor" | "rot90wide" => {
            let s = &shapes[0];
            assert_eq!((shapes.len(), preset_of(s).0), (1, "rect"), "{name}");
            assert_eq!(boxed(s), wide, "{name}");
            let fill = match name {
                "fill" => Fill::solid(rgb(0xFF, 0, 0)),
                "fill2" => Fill::solid(rgb(0, 0xFF, 0)),
                "nofill" => Fill::None,
                _ => Fill::solid(rgb(0, 0xCC, 0x99)),
            };
            assert_eq!(s.fill, Some(fill), "{name}");
            let line = match name {
                "lineoff" => craft_pptx::Line::none(),
                "linecolor" => craft_pptx::Line::solid(rgb(0, 0, 0xFF), 1.0),
                _ => craft_pptx::Line::solid(black.clone(), 1.0),
            };
            assert_eq!(s.line, Some(line), "{name}");
            let shadow = s.effects.as_ref().and_then(|e| e.outer_shadow.as_ref()).map(|sh| sh.color.clone());
            let want = match name {
                // The scheme's shadow colour, and the red the object model set (BGR 255).
                "shadow" => Some(rgb(0x96, 0x96, 0x96)),
                "shadowcolor" => Some(rgb(0xFF, 0, 0)),
                _ => None,
            };
            assert_eq!(shadow, want, "{name}");
            // Turned counter-clockwise; a rectangle's flip doesn't change it.
            let rot = match name {
                "rotate" => 330.0,
                "rot90wide" => 270.0,
                _ => 0.0,
            };
            assert_eq!((s.xfrm.unwrap().rot, flips(s)), (rot, (false, false)), "{name}");
        }
        _ if name.starts_with("lstyle") || name.starts_with("dash") => {
            let s = &shapes[0];
            assert_eq!((shapes.len(), preset_of(s).0, boxed(s)), (1, "rect", wide), "{name}");
            let line = s.line.clone().unwrap();
            let (width, dash) = match name {
                "lstyle2" => (2.0, None),
                "lstyle3" => (4.0, None),
                "lstyle4" => (6.0, None),
                "lstyle5" => (8.0, None),
                "lstyle6" => (10.0, None),
                "dash1" => (1.0, Some("sysDot")),
                "dash2" => (1.0, Some("dash")),
                "dash3" => (1.0, Some("lgDash")),
                "dash4" => (1.0, Some("dashDot")),
                _ => return,
            };
            let dash = dash.map(|n| craft_pptx::style::Dash::Preset { name: n.into() });
            assert_eq!((line.width, line.dash, line.fill), (Some(width), dash, Some(Fill::solid(black))), "{name}");
        }
        "line" => {
            let s = &shapes[0];
            assert_eq!((shapes.len(), preset_of(s).0), (1, "line"));
            // AddLine(1440, 1440, 4320, 2880): from 1 in, 1 in to 4 in, 3 in.
            assert_eq!(boxed(s), (72.0, 72.0, 216.0, 144.0));
            assert_eq!(flips(s), (false, false));
            assert_eq!(s.line, Some(craft_pptx::Line::solid(black, 1.0)));
        }
        "linebase" | "arrow1" | "arrow2" | "arrow3" | "arrow4" => {
            let s = &shapes[0];
            // AddLine(1440, 1440, 4320, 0): 3 in across from 1 in, 1 in.
            assert_eq!((shapes.len(), preset_of(s).0, boxed(s), flips(s)), (1, "line", (72.0, 72.0, 216.0, 0.0), (false, false)), "{name}");
            let end = |kind: &str, size: &str| Some(craft_pptx::style::LineEnd { kind: kind.into(), w: size.into(), len: size.into() });
            let stealth = end("stealth", "med");
            let want = match name {
                "arrow1" => (None, stealth),
                "arrow2" => (stealth, None),
                "arrow3" => (stealth.clone(), stealth),
                "arrow4" => (None, end("triangle", "lg")),
                _ => (None, None),
            };
            let line = s.line.clone().unwrap();
            assert_eq!((line.head, line.tail), want, "{name}");
            assert_eq!((line.width, line.fill), (Some(1.0), Some(Fill::solid(black))), "{name}");
        }
        _ => check_object_sample(name, shapes),
    }
}

/// What one PowerPoint 95 sample of a freeform or group shows, by its case name.
fn check_object_sample(name: &str, shapes: &[Shape]) {
    // AddShape(1, 1440, 1440, 1440, 1440) and AddShape(3, 3600, 2160, 1440, 720) in twips, and the
    // third, AddShape(5, 5760, 1440, 1440, 1440): where they were before they were grouped.
    let (rect, ellipse, triangle) = ((72.0, 72.0, 72.0, 72.0), (180.0, 108.0, 72.0, 36.0), (288.0, 72.0, 72.0, 72.0));
    match name {
        "freeform" | "freeform-closed" | "freeform-filled" => {
            let s = &shapes[0];
            assert_eq!((shapes.len(), boxed(s)), (1, (72.0, 72.0, 144.0, 144.0)), "{name}");
            let (fill, d) = if name == "freeform" {
                (Fill::None, "M 0 0 L 1152 0 L 576 1152")
            } else {
                (Fill::solid(rgb(0x00, 0xCC, 0x99)), "M 0 0 L 1152 0 L 1152 1152 L 0 1152 Z")
            };
            assert_eq!((path_of(s), s.fill.clone()), ((1152.0, 1152.0, d), Some(fill)), "{name}");
            assert_eq!(s.line.as_ref().and_then(|l| l.fill.clone()), Some(Fill::solid(rgb(0, 0, 0))), "{name}");
        }
        "group" | "group-rot" => {
            assert_eq!(shapes.len(), 1, "{name}");
            let m = group_members(&shapes[0]);
            assert_eq!(m.iter().map(|s| (preset_of(s).0, boxed(s))).collect::<Vec<_>>(), [("rect", rect), ("ellipse", ellipse)], "{name}");
            assert_eq!(m[0].fill, Some(Fill::solid(rgb(0xFF, 0, 0))), "{name}");
            assert_eq!((boxed(&shapes[0]), shapes[0].xfrm.unwrap().rot), ((72.0, 72.0, 180.0, 72.0), 0.0), "{name}");
        }
        "group-nested" => {
            assert_eq!(shapes.len(), 1);
            let outer = group_members(&shapes[0]);
            assert_eq!((outer.len(), boxed(&shapes[0])), (2, (72.0, 72.0, 288.0, 72.0)));
            let inner = group_members(&outer[0]);
            assert_eq!(inner.iter().map(boxed).collect::<Vec<_>>(), [rect, ellipse]);
            assert_eq!((preset_of(&outer[1]).0, boxed(&outer[1])), ("triangle", triangle));
        }
        _ => {}
    }
}

/// The polygons of a placeable WMF (its META_POLYGON records, and its META_RECTANGLE records as
/// four corners) in points on a slide of size `slide`: the WMF PowerPoint 95 saved a slide as is in
/// master units about the slide's centre, y down.
fn wmf_polygons(b: &[u8], slide: craft_pptx::Size) -> Vec<Vec<(f64, f64)>> {
    let mut polygons = vec![];
    // The placeable header (22 bytes), then the standard one (18).
    let mut at = 40;
    // Each record is read only within its size (in 16-bit words), so a short one can't take its
    // values from the next.
    while let Some(size) = b.get(at..at + 4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) as usize) {
        let Some(rec) = b.get(at..at + size * 2).filter(|_| size >= 3) else { break };
        let word = |i: usize| rec.get(i..i + 2).map(|w| i16::from_le_bytes([w[0], w[1]]));
        let pt = |x: i16, y: i16| (f64::from(x) / 8.0 + slide.width / 2.0, f64::from(y) / 8.0 + slide.height / 2.0);
        match word(4).map(|f| f as u16) {
            // META_POLYGON: a count, then that many points; one whose points don't fit is skipped.
            Some(0x0324) => {
                let n = word(6).unwrap_or(0).max(0) as usize;
                if 8 + 4 * n <= rec.len() {
                    polygons.push((0..n).filter_map(|i| Some(pt(word(8 + 4 * i)?, word(10 + 4 * i)?))).collect());
                }
            }
            // META_RECTANGLE: bottom, right, top and left, in that order.
            Some(0x041B) => {
                if let (Some(bottom), Some(right), Some(top), Some(left)) = (word(6), word(8), word(10), word(12)) {
                    polygons.push(vec![pt(left, top), pt(right, top), pt(right, bottom), pt(left, bottom)]);
                }
            }
            _ => {}
        }
        at += size * 2;
    }
    polygons
}

/// The corners of a shape's outline on the slide, for the presets whose outline is straight lines
/// (and the trapezoid's own outline): each flipped about the box's centre, then turned clockwise
/// about it, as DrawingML draws it. `None` for an outline with curves.
fn outline_corners(s: &Shape) -> Option<Vec<(f64, f64)>> {
    const STRAIGHT: [&str; 6] = ["rect", "rtTriangle", "triangle", "rightArrow", "trapezoid", "homePlate"];
    let x = s.xfrm?;
    let local: Vec<(f64, f64)> = match &s.geom {
        Geom::Preset { name, adj } if STRAIGHT.contains(&name.as_str()) => {
            let g = craft_drawingml::preset::build(name, x.w, x.h, adj)?;
            g.outline().elements().iter().filter_map(|e| e.end_point()).map(|p| (p.x, p.y)).collect()
        }
        Geom::Custom { paths } => {
            let p = paths.first()?;
            let v: Vec<f64> = p.d.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            v.as_chunks::<2>().0.iter().map(|[u, v]| (u * x.w / p.w, v * x.h / p.h)).collect()
        }
        _ => return None,
    };
    let (sin, cos) = x.rot.to_radians().sin_cos();
    let (cx, cy) = (x.x + x.w / 2.0, x.y + x.h / 2.0);
    Some(
        local
            .into_iter()
            .map(|(u, v)| {
                let (mut dx, mut dy) = (u - x.w / 2.0, v - x.h / 2.0);
                if x.flip_h {
                    dx = -dx;
                }
                if x.flip_v {
                    dy = -dy;
                }
                (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
            })
            .collect(),
    )
}

/// Whether the model's outline and the polygon PowerPoint 95 drew have the same vertices in the same
/// order, from any vertex and in either direction. The pen is drawn inside the frame, so the
/// polygon sits half a line width inside the outline, and its vertices are rounded to whole units.
fn outline_matches(corners: &[(f64, f64)], polygon: &[(f64, f64)], width: f64) -> bool {
    let tol = width / 2.0 + 1.0;
    let same = |a: &(f64, f64), b: &(f64, f64)| (a.0 - b.0).abs() <= tol && (a.1 - b.1).abs() <= tol;
    // Each vertex once: no repeats in a row, and no closing repeat of the first. Only a vertex
    // repeated is dropped; the pen's tolerance is for comparing the two outlines, not for merging
    // the close corners of a narrow one.
    let repeat = |a: &(f64, f64), b: &(f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
    let ring = |v: &[(f64, f64)]| {
        let mut out: Vec<(f64, f64)> = vec![];
        for p in v {
            if out.last().is_none_or(|q| !repeat(p, q)) {
                out.push(*p);
            }
        }
        if out.len() > 1 && repeat(&out[0], &out[out.len() - 1]) {
            out.pop();
        }
        out
    };
    let (a, b) = (ring(corners), ring(polygon));
    let n = a.len();
    n >= 3 && b.len() == n && (0..n).any(|k| (0..n).all(|i| same(&a[i], &b[(k + i) % n])) || (0..n).all(|i| same(&a[i], &b[(k + n - i) % n])))
}

/// The outline check compares vertices in order: from any vertex, either way round, with or
/// without a closing repeat, but a crossed contour through the same vertices is not a match.
#[test]
fn outlines_match_in_order() {
    let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
    assert!(outline_matches(&square, &[(10.0, 10.0), (10.0, 0.0), (0.0, 0.0), (0.0, 10.0), (10.0, 10.0)], 1.0));
    assert!(!outline_matches(&square, &[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)], 1.0));
    assert!(!outline_matches(&square, &square[..3], 1.0));
    // A 2 pt-wide rectangle drawn with a 1 pt pen: the drawn corners sit half a point inside, and
    // its short edges' corners are within the tolerance of each other but stay four vertices.
    let narrow = [(0.0, 0.0), (2.0, 0.0), (2.0, 10.0), (0.0, 10.0)];
    assert!(outline_matches(&narrow, &[(0.5, 0.5), (1.5, 0.5), (1.5, 9.5), (0.5, 9.5)], 1.0));
}

/// The WMF reader takes each record's values from within its own size: a polygon whose points
/// don't fit, and a rectangle cut short, are skipped rather than read from the next record.
#[test]
fn the_wmf_reader_stays_in_each_record() {
    let record = |func: u16, words: &[i16]| {
        let mut r = ((3 + words.len()) as u32).to_le_bytes().to_vec();
        r.extend_from_slice(&func.to_le_bytes());
        r.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        r
    };
    let wmf = |records: &[Vec<u8>]| {
        let mut b = vec![0u8; 40];
        b.extend(records.concat());
        b.extend(record(0, &[]));
        wmf_polygons(&b, craft_pptx::Size::new(720.0, 540.0))
    };
    // A polygon of 2 points that says it has 3, and a rectangle without its left.
    assert!(wmf(&[record(0x0324, &[3, 0, 0, 8, 8]), record(0x041B, &[8, 8, 0])]).is_empty());
    // Whole, both read.
    let got = wmf(&[record(0x0324, &[3, 0, 0, 8, 8, 16, 0]), record(0x041B, &[8, 8, 0, 0])]);
    assert_eq!(got, [vec![(360.0, 270.0), (361.0, 271.0), (362.0, 270.0)], vec![(360.0, 270.0), (361.0, 270.0), (361.0, 271.0), (360.0, 271.0)]]);
}

/// For a sample with a rendering beside it (`<case>.wmf`), each shape on its first slide with a
/// straight outline matches a polygon PowerPoint 95 drew, each a polygon of its own
/// ([`match_outlines`]): two shapes moved onto one place don't both match it. The number of shapes
/// checked.
fn check_outlines(name: &str, p: &Presentation, wmf: &[u8]) -> usize {
    let polygons = wmf_polygons(wmf, p.slide_size);
    let Some(slide) = p.slides.first() else { return 0 };
    let mut outlines = vec![];
    for s in &slide.shapes {
        let Some(corners) = outline_corners(s) else { continue };
        // A dashed line is drawn dash by dash.
        if s.line.as_ref().is_some_and(|l| l.dash.is_some()) {
            continue;
        }
        let width = s.line.as_ref().and_then(|l| l.width).unwrap_or(1.0);
        let near = polygons.iter().min_by_key(|poly| poly.len().abs_diff(corners.len()));
        assert!(polygons.iter().any(|poly| outline_matches(&corners, poly, width)), "{name}: {corners:?} is not drawn; nearest {near:?}");
        outlines.push((corners, width));
    }
    if let Err(i) = match_outlines(&outlines, &polygons) {
        panic!("{name}: {:?} is drawn only where another shape is", outlines[i].0);
    }
    outlines.len()
}

/// Gives each outline (its corners and line width) a polygon of its own that it matches, trying
/// every assignment rather than the first that fits, or else the first outline left without one.
fn match_outlines(outlines: &[(Vec<(f64, f64)>, f64)], polygons: &[Vec<(f64, f64)>]) -> Result<(), usize> {
    let fits: Vec<Vec<usize>> =
        outlines.iter().map(|(corners, width)| (0..polygons.len()).filter(|&q| outline_matches(corners, &polygons[q], *width)).collect()).collect();
    // Kuhn's algorithm: a polygon already taken is freed when its owner can move to another.
    fn assign(fits: &[Vec<usize>], s: usize, seen: &mut [bool], owner: &mut [Option<usize>]) -> bool {
        for &q in &fits[s] {
            if std::mem::replace(&mut seen[q], true) {
                continue;
            }
            if owner[q].is_none_or(|t| assign(fits, t, seen, owner)) {
                owner[q] = Some(s);
                return true;
            }
        }
        false
    }
    let mut owner = vec![None; polygons.len()];
    for s in 0..outlines.len() {
        if !assign(&fits, s, &mut vec![false; polygons.len()], &mut owner) {
            return Err(s);
        }
    }
    Ok(())
}

/// Two shapes can't both match one drawn polygon, and a polygon that fits two shapes goes to the
/// one that needs it, whatever order the polygons were drawn in.
#[test]
fn each_outline_matches_a_polygon_of_its_own() {
    let rect = |x: f64| vec![(x, 0.0), (x + 10.0, 0.0), (x + 10.0, 10.0), (x, 10.0)];
    // Shapes at 100 and 102 drawn at 101 and 99: the first fits both, the second only 101.
    assert_eq!(match_outlines(&[(rect(100.0), 1.0), (rect(102.0), 1.0)], &[rect(101.0), rect(99.0)]), Ok(()));
    // Two shapes moved onto one place, drawn at two.
    assert_eq!(match_outlines(&[(rect(0.0), 1.0), (rect(0.0), 1.0)], &[rect(0.0), rect(50.0)]), Err(1));
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
/// the repository). Each imports with the slide count and size its DocumentAtom gives and nothing
/// lost, and its shapes' outlines match the rendering PowerPoint 95 saved beside it, if any.
#[test]
fn powerpoint_95_samples() {
    let Some(dir) = std::env::var_os("CRAFT_PPT95_SAMPLES") else { return };
    let (mut seen, mut outlines) = (0, 0);
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
        check_sample(&name, &p);
        // The rendering is optional; one that is there but can't be read is a failure.
        match std::fs::read(path.with_extension("wmf")) {
            Ok(wmf) => outlines += check_outlines(&name, &p, &wmf),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("{name}.wmf: {e}"),
        }
        seen += 1;
    }
    eprintln!("read {seen} PowerPoint 95 samples, {outlines} outlines checked against their renderings");
    assert!(seen > 0);
}
