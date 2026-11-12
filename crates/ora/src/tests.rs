use std::io::{Cursor, Write};

use super::*;

fn zip_of(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (i, (name, data)) in files.iter().enumerate() {
        z.start_file(*name, if deflate && i > 0 { deflated } else { stored }).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// An ORA archive from a hand-written stack.xml and named entries.
fn archive(stack: &str, images: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut all: Vec<(&str, &[u8])> = vec![("mimetype", MIMETYPE), ("stack.xml", stack.as_bytes())];
    all.extend(images.iter().map(|(n, d)| (*n, d.as_slice())));
    zip_of(&all, false)
}

fn image(stack: &str) -> Result<Image> {
    Archive::open(&archive(stack, &[])).map(|a| a.image().clone())
}

fn layer(node: &Node) -> &Layer {
    match node {
        Node::Layer(l) => l,
        other => panic!("not a layer: {other:?}"),
    }
}

fn group(node: &Node) -> &Stack {
    match node {
        Node::Stack(s) => s,
        other => panic!("not a stack: {other:?}"),
    }
}

#[test]
fn reads_other_applications_stacks() {
    // The shapes Krita, MyPaint and GIMP write: a root stack with attributes, `auto`
    // isolation, svg:plus, krita: ids, unknown elements, fractional positions.
    let xml = r#"<?xml version='1.0' encoding='UTF-8'?>
<image version="0.0.3" w="8" h="6" xres="150" yres="151">
 <stack opacity="1" name="root" isolation="isolate" composite-op="svg:src-over">
  <layer name="Plus" src=" data/a.png " x="2.4" y="-1.6" composite-op=" svg:plus " opacity="0.5" selected="true"/>
  <stack name="Folder" isolation=" auto " composite-op="krita:darker color" opacity="1.5">
   <layer name="Inside" src="data/b.png" composite-op="krita:hard mix" visibility="hidden" alpha-preserve="true"/>
  </stack>
  <stack name="  Through  "><layer src="data/b.png"/></stack>
  <text name="Caption">hello</text>
  <layer name="Gray16" src="data/g.png" composite-op="svg:soft-light" edit-locked="1"/>
  <layer name="Lost" src="data/missing.png"/>
  <layer/>
 </stack>
</image>"#;
    let bytes = archive(xml, &[("data/a.png", b"png a".to_vec()), ("data/b.png", b"png b".to_vec())]);
    assert!(is_ora(&bytes));
    let mut a = Archive::open(&bytes).unwrap();
    let image = a.image().clone();
    assert_eq!((image.version.as_deref(), image.width, image.height, image.xres, image.yres), (Some("0.0.3"), 8, 6, 150.0, 151.0));
    assert!(!image.explicit_group_isolation());
    assert_eq!(image.stack.len(), 7, "topmost first, unknown elements kept in place");
    let plus = layer(&image.stack[0]);
    assert_eq!((plus.props.name.as_deref(), plus.x, plus.y, plus.src.as_deref()), (Some("Plus"), 2, -2, Some("data/a.png")));
    assert_eq!((plus.props.composite_op.as_deref(), plus.props.opacity, plus.props.visible), (Some("svg:plus"), 0.5, true));
    assert_eq!(a.entry("data/a.png", MAX_ENTRY).unwrap().as_deref(), Some(&b"png a"[..]));
    let folder = group(&image.stack[1]);
    assert_eq!(
        (folder.isolation.as_deref(), folder.props.composite_op.as_deref(), folder.props.opacity),
        (Some("auto"), Some("krita:darker color"), 1.5)
    );
    let inside = layer(&folder.children[0]);
    assert!(!inside.props.visible && inside.props.alpha_preserve && !inside.props.edit_locked);
    let through = group(&image.stack[2]);
    assert_eq!(
        (through.props.name.as_deref(), through.isolation.as_deref(), through.props.composite_op.as_deref()),
        (Some("  Through  "), None, None)
    );
    assert_eq!(layer(&through.children[0]).props, Props::default());
    assert_eq!(image.stack[3], Node::Other { tag: "text".into(), name: Some("Caption".into()) });
    assert!(layer(&image.stack[4]).props.edit_locked);
    let lost = layer(&image.stack[5]);
    assert_eq!(a.entry(lost.src.as_deref().unwrap(), MAX_ENTRY).unwrap(), None, "a layer may name an entry the archive lacks");
    assert_eq!(layer(&image.stack[6]), &Layer::default());
    assert!(a.entry("data/b.png", 4).is_err(), "an entry past the caller's limit is refused");
}

#[test]
fn resolution_defaults_to_72_and_yres_to_xres() {
    let i = image("<image w='2' h='2'><stack/></image>").unwrap();
    assert_eq!((i.xres, i.yres, i.version), (72.0, 72.0, None));
    let i = image("<image w='2' h='2' xres='300'><stack/></image>").unwrap();
    assert_eq!((i.xres, i.yres), (300.0, 300.0));
}

#[test]
fn group_isolation_is_explicit_since_version_0_0_6() {
    let explicit = |v: Option<&str>| {
        Image { version: v.map(str::to_string), width: 1, height: 1, xres: 72.0, yres: 72.0, stack: vec![] }.explicit_group_isolation()
    };
    for v in ["0.0.6", " 0.0.6 ", "0.0.7", "0.1.0", "1.0.0", "0.0.6-beta", "0.0.10+build"] {
        assert!(explicit(Some(v)), "{v}");
    }
    for v in ["0.0.5", "0.0.3", "0.0", "0.0.6.1", "six", "", "0.0.x"] {
        assert!(!explicit(Some(v)), "{v}");
    }
    assert!(!explicit(None));
}

#[test]
fn a_written_archive_reads_back_the_same() {
    let p = |name: &str, op: Option<&str>| Props { name: Some(name.into()), composite_op: op.map(str::to_string), ..Props::default() };
    let image = Image {
        version: None,
        width: 70,
        height: 50,
        xres: 300.0,
        yres: 300.0,
        stack: vec![
            Node::Layer(Layer {
                props: Props { opacity: 0.25, visible: false, ..p("  Top\t&\n<\"'> \u{1} ", Some("svg:multiply")) },
                x: -3,
                y: 7,
                src: Some("data/layer0.png".into()),
            }),
            Node::Stack(Stack {
                props: Props { edit_locked: true, alpha_preserve: true, ..p("Pass <&> \"through\"", Some("krita:linear light")) },
                isolation: Some("auto".into()),
                children: vec![
                    Node::Layer(Layer { props: p("Inner", None), x: 0, y: 0, src: Some("data/layer1.png".into()) }),
                    Node::Stack(Stack::default()),
                ],
            }),
            Node::Other { tag: "text".into(), name: None },
        ],
    };
    let mut w = Writer::new();
    w.add("data/layer0.png", b"zero").unwrap();
    w.add("data/layer1.png", b"one").unwrap();
    w.add_stack(&image).unwrap();
    w.add("mergedimage.png", b"merged").unwrap();
    w.add("Thumbnails/thumbnail.png", b"thumb").unwrap();
    let bytes = w.finish().unwrap();
    // A stored `mimetype` first, readable at a fixed offset.
    assert_eq!(bytes.get(30..38), Some(&b"mimetype"[..]));
    assert_eq!(bytes.get(38..54), Some(MIMETYPE));
    assert!(is_ora(&bytes));
    let mut a = Archive::open(&bytes).unwrap();
    for (entry, data) in
        [("data/layer0.png", "zero"), ("data/layer1.png", "one"), ("mergedimage.png", "merged"), ("Thumbnails/thumbnail.png", "thumb")]
    {
        assert_eq!(a.entry(entry, 16).unwrap().as_deref(), Some(data.as_bytes()), "{entry}");
    }
    let xml = String::from_utf8(a.entry("stack.xml", 1 << 20).unwrap().unwrap()).unwrap();
    assert!(
        xml.starts_with(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<image version=\"0.0.6\" w=\"70\" h=\"50\" xres=\"300\" yres=\"300\"><stack><layer name="
        ),
        "{xml}"
    );
    assert!(xml.contains("name=\"Pass &lt;&amp;&gt; &quot;through&quot;\""), "{xml}");
    assert!(
        xml.contains(" visibility=\"hidden\" opacity=\"0.25\" composite-op=\"svg:multiply\" src=\"data/layer0.png\" x=\"-3\" y=\"7\"/>"),
        "{xml}"
    );
    assert!(xml.contains("composite-op=\"krita:linear light\" edit-locked=\"true\" alpha-preserve=\"true\" isolation=\"auto\">"), "{xml}");
    assert!(
        xml.ends_with(
            "<stack visibility=\"visible\" opacity=\"1\" composite-op=\"svg:src-over\" isolation=\"isolate\"></stack></stack></stack></image>\n"
        ),
        "{xml}"
    );
    // What comes back: the version and the defaults written out, the unknown element gone, and
    // the control character XML cannot carry dropped from the name. Tab and newline survive.
    let back = a.image();
    assert!(back.explicit_group_isolation());
    assert_eq!((back.version.as_deref(), back.width, back.height, back.xres, back.yres, back.stack.len()), (Some(VERSION), 70, 50, 300.0, 300.0, 2));
    let top = layer(&back.stack[0]);
    assert_eq!(top.props.name.as_deref(), Some("  Top\t&\n<\"'>  "));
    assert_eq!((top.props.opacity, top.props.visible, top.x, top.y), (0.25, false, -3, 7));
    let pass = group(&back.stack[1]);
    assert_eq!((pass.props.name.as_deref(), pass.isolation.as_deref()), (Some("Pass <&> \"through\""), Some("auto")));
    assert_eq!(layer(&pass.children[0]).props.composite_op.as_deref(), Some("svg:src-over"));
    assert_eq!(group(&pass.children[1]).isolation.as_deref(), Some("isolate"));
    assert_eq!(back.to_xml(), xml, "writing what was read is stable");
}

#[test]
fn a_stack_built_ten_thousand_deep_is_written_without_recursion() {
    let mut node = Stack::default();
    for _ in 0..10_000 {
        node = Stack { children: vec![Node::Stack(node)], ..Stack::default() };
    }
    let image = Image { version: None, width: 1, height: 1, xres: 72.0, yres: 72.0, stack: vec![Node::Stack(node)] };
    let xml = std::thread::Builder::new().stack_size(512 * 1024).spawn(move || {
        let xml = image.to_xml();
        std::mem::forget(image); // dropping a hand-built tree recurses; that is the builder's concern
        xml
    });
    assert_eq!(xml.unwrap().join().unwrap().matches("</stack>").count(), 10_002);
}

/// A stack.xml nested 20 000 deep is an error. The small stack makes a regression abort instead
/// of passing.
#[test]
fn deeply_nested_stack_xml_is_an_error_not_a_stack_overflow() {
    let stack = format!("<image w=\"4\" h=\"4\">{}{}</image>", "<stack>".repeat(20_000), "</stack>".repeat(20_000));
    let bytes = archive(&stack, &[]);
    let e = std::thread::Builder::new().stack_size(512 * 1024).spawn(move || Archive::open(&bytes).err()).unwrap().join().unwrap();
    assert!(matches!(&e, Some(Error::Invalid(m)) if m.contains("deeper")), "{e:?}");
    let nested = |n: usize| format!("<image w='2' h='2'>{}{}</image>", "<stack>".repeat(n), "</stack>".repeat(n));
    let bytes = archive(&nested(MAX_XML_DEPTH - 1), &[]);
    let depth = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let a = Archive::open(&bytes).unwrap();
            std::iter::successors(a.image().stack.first(), |n| group(n).children.first()).count()
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(depth, MAX_XML_DEPTH - 2);
    assert!(image(&nested(MAX_XML_DEPTH)).is_err());
}

#[test]
fn malformed_archives_fail_cleanly() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("not a zip", b"definitely not a zip archive at all".to_vec()),
        ("no stack", zip_of(&[("mimetype", MIMETYPE)], false)),
        ("no mimetype", zip_of(&[("stack.xml", b"<image w='1' h='1'><stack/></image>")], false)),
        ("wrong mimetype", zip_of(&[("mimetype", b"image/png"), ("stack.xml", b"<image w='1' h='1'><stack/></image>")], false)),
        ("bad xml", archive("<image w='2' h='2'><stack>", &[])),
        ("doctype", archive("<!DOCTYPE image SYSTEM 'x.dtd'><image w='2' h='2'><stack/></image>", &[])),
        ("wrong root", archive("<picture w='2' h='2'><stack/></picture>", &[])),
        ("no root stack", archive("<image w='2' h='2'/>", &[])),
        ("zero width", archive("<image w='0' h='2'><stack/></image>", &[])),
        ("fractional width", archive("<image w='2.5' h='2'><stack/></image>", &[])),
        ("too tall", archive("<image w='2' h='300001'><stack/></image>", &[])),
        ("huge canvas", archive("<image w='300000' h='300000'><stack/></image>", &[])),
        ("bad resolution", archive("<image w='2' h='2' yres='dense'><stack/></image>", &[])),
        ("bad opacity", archive("<image w='2' h='2'><stack><layer src='a.png' opacity='lots'/></stack></image>", &[])),
        ("nan opacity", archive("<image w='2' h='2'><stack><stack opacity='NaN'/></stack></image>", &[])),
        ("far offset", archive("<image w='2' h='2'><stack><layer src='a.png' x='99999999999'/></stack></image>", &[])),
        ("just too far", archive("<image w='2' h='2'><stack><layer y='-1200001'/></stack></image>", &[])),
        ("not utf-8", archive("<image w='2' h='2' n='\u{0}'><stack/></image>", &[]).iter().map(|b| if *b == 0 { 0xff } else { *b }).collect()),
        ("too many", archive(&format!("<image w='2' h='2'><stack>{}</stack></image>", "<stack/>".repeat(MAX_NODES + 1)), &[])),
    ];
    for (what, bytes) in cases {
        assert!(matches!(Archive::open(&bytes), Err(Error::Invalid(_))), "{what}");
    }
    assert!(image(&format!("<image w='2' h='2'><stack>{}</stack></image>", "<text/>".repeat(MAX_NODES))).is_ok());
    assert!(image("<image w='300000' h='447'><stack><layer x='1200000'/></stack></image>").is_ok());
}

#[test]
fn deflated_entries_are_read_under_the_limit() {
    let big = vec![b' '; MAX_STACK_XML + 1];
    let bytes = zip_of(&[("mimetype", MIMETYPE), ("stack.xml", &big)], true);
    assert!(bytes.len() < 1 << 20);
    assert_eq!(Archive::open(&bytes).err(), Some(Error::Invalid(format!("stack.xml: larger than {MAX_STACK_XML} bytes"))));
    let bytes = zip_of(&[("mimetype", MIMETYPE), ("stack.xml", b"<image w='1' h='1'><stack/></image>"), ("a.png", &[9; 70_000])], true);
    let mut a = Archive::open(&bytes).unwrap();
    assert_eq!(a.entry("a.png", 70_000).unwrap(), Some(vec![9; 70_000]));
    assert!(a.entry("a.png", 69_999).is_err());
}

#[test]
fn truncated_and_corrupted_files_never_panic() {
    let xml = "<image version='0.0.6' w='8' h='6'><stack><stack name='g' isolation='auto'><layer src='a.png' x='1'/></stack><layer src='b.png'/></stack></image>";
    let bytes = zip_of(&[("mimetype", MIMETYPE), ("stack.xml", xml.as_bytes()), ("a.png", &[1; 300]), ("b.png", &[2; 300])], true);
    let read_all = |bytes: &[u8]| -> Result<usize> {
        let mut a = Archive::open(bytes)?;
        Ok(a.entry("a.png", MAX_ENTRY)?.map_or(0, |d| d.len()) + a.entry("b.png", MAX_ENTRY)?.map_or(0, |d| d.len()))
    };
    assert_eq!(read_all(&bytes), Ok(600));
    for cut in 0..bytes.len() {
        let _ = read_all(bytes.get(..cut).unwrap_or_default());
    }
    let mut state = 0x9e37_79b9_u32;
    for _ in 0..300 {
        let mut b = bytes.clone();
        for _ in 0..4 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let i = state as usize % b.len();
            b[i] ^= (state >> 24) as u8 | 1;
        }
        let _ = read_all(&b);
    }
}

#[test]
fn zip_mimetype_reads_only_a_leading_stored_entry() {
    assert_eq!(zip_mimetype(&archive("<image/>", &[])), Some(MIMETYPE));
    assert_eq!(zip_mimetype(b"PK\x03\x04"), None);
    assert_eq!(zip_mimetype(&[]), None);
    assert_eq!(zip_mimetype(&zip_of(&[("stack.xml", b"x"), ("mimetype", MIMETYPE)], false)), None);
    assert!(!is_ora(&zip_of(&[("mimetype", b"application/x-krita")], false)));
    assert_eq!(zip_mimetype(&zip_of(&[("mimetype", &[b'a'; 257])], false)), None);
}

/// A ZIP64 end record can declare millions of entries in a small file: refused on the raw bytes
/// before the archive is indexed.
#[test]
fn an_archive_declaring_too_many_entries_is_refused_before_it_is_indexed() {
    let bomb = craft_zip::zip64_entry_count_sample(MAX_ENTRIES + 1);
    assert_eq!(
        Archive::open(&bomb).err(),
        Some(Error::Invalid(format!("the archive declares {} entries, more than {MAX_ENTRIES}", MAX_ENTRIES + 1)))
    );
}

/// Two entries of one name: `zip` keeps one, and which one the writer meant is not known.
#[test]
fn two_entries_of_one_name_are_refused() {
    let bytes = archive("<image w='1' h='1'><stack/></image>", &[("dupA", vec![1]), ("dupB", vec![2])]);
    assert!(Archive::open(&bytes).is_ok());
    let mut renamed = bytes.clone();
    let mut i = 0;
    while let Some(at) = renamed[i..].windows(4).position(|w| w == b"dupB") {
        renamed[i + at + 3] = b'A';
        i += at + 4;
    }
    assert_eq!(Archive::open(&renamed).err(), Some(Error::Invalid("two entries of the archive share a name".into())));
}
