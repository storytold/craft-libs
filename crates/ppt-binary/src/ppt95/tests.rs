//! PowerPoint 95 files built here from records, written from the format notes (never files made
//! by PowerPoint), and an optional check over real ones named by `CRAFT_PPT95_SAMPLES`.
#![allow(clippy::unwrap_used)]

use craft_pptx::text::{Align, Anchor, Bullet, ListStyle, Spacing};
use craft_pptx::{Background, ColorRef, Fill, LayoutType, Paragraph, PhType, Presentation, Rgba, SchemeSlot, Shape, TextBody};

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
            container(rt::OBJECTS, 0x14, &s.objects),
        ],
    )
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
    environment_with(&[])
}

/// The Environment, with `styles` (the master's text styles' container) when given.
fn environment_with(styles: &[Vec<u8>]) -> Vec<u8> {
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
    let mut kids = vec![rulers, fonts, container(2005, 0x1A, &[])];
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
    stream_with(atom, slides, masters, environment())
}

fn stream_with(atom: Vec<u8>, slides: &[Vec<u8>], masters: &[Vec<u8>], environment: Vec<u8>) -> Vec<u8> {
    let list = |tag: u32, items: &[Vec<u8>]| {
        let mut kids = vec![rec(2001, 0, 0, &le32(&[items.len() as u32]))];
        kids.extend_from_slice(items);
        container(rt::LIST, tag, &kids)
    };
    let document = container(
        rt::DOCUMENT,
        0,
        &[atom, environment, list(0xA, slides), list(0xB, masters), list(0xC, &[]), container(1026, 0x64, &[]), rec(1002, 0, 0, &[])],
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
    let empty = rec(rt::SLIDE, NONE, 0x10, &[]);
    let many = vec![empty; super::MAX_SLIDES + 5];
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
            (Some(Spacing::Pct(100.0)), Some(Spacing::Pts(4000.0)), Some(Spacing::Pts(1.0))),
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
    let stream = |env: Vec<u8>| stream_with(document_atom(5760, 4320, 0), &[], &[master_with(MASTER, scheme, [0; 4], &master_objects)], env);
    let p = import(&stream(environment_with(&[styles(&atoms)]))).unwrap();
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
    let cut = import(&stream(environment_with(&[styles(&[cut])]))).unwrap();
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
    let stream =
        stream_with(document_atom(5760, 4320, 0), &[], &[master_with(MASTER, DEFAULT, [0; 4], &[title])], environment_with(&[styles(&atoms)]));
    let t = import(&stream).unwrap().masters[0].title_style.level(0).cloned().unwrap();
    // The wide ruler (slot 3): level 1 at 576 units, its first line at 288.
    assert_eq!((t.para.margin_left, t.para.indent), (Some(72.0), Some(-36.0)));
}

/// A shape with text is a text box with no fill or line of its own; a shape without text isn't
/// read yet; a placeholder of a type PowerPoint 95 isn't known to write keeps its text as a text
/// box.
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

    // The box is the unturned one; the angle is in sixteenths of a degree.
    let turned = container(rt::SHAPE, 0x13, &[geometry(bx, 480), simple_text(b"Box", bx, BODY)]);
    let p = deck_of(vec![turned], &[]);
    let x = p.slides[0].shapes[0].xfrm.unwrap();
    assert_eq!((x.x, x.w, x.rot), (64.75, 230.5, 30.0));
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
    // Many zero-length runs: no more are read than the text has characters (plus its end).
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
    let env = environment_with(&[styles(&[style_atom([level; 5], 0x13), style_atom([level; 5], 0x21)])]);
    let stream = stream_with(document_atom(5760, 4320, 1), &[slide(&s)], &[master_with(MASTER, DEFAULT, [0; 4], &objects)], env);
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
        _ => {}
    }
}

/// A file holding fewer slides than its DocumentAtom counts names the missing ones by position;
/// a whole file names none.
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
        check_sample(&name, &p);
        seen += 1;
    }
    eprintln!("read {seen} PowerPoint 95 samples");
    assert!(seen > 0);
}
