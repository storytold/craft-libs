//! A member of an object stream (ISO 32000-2 §7.5.7) that its stream can't supply is read from a
//! plain `N 0 obj` body elsewhere in the file, as a reader rebuilding the cross-reference data by
//! scanning the file's objects would (Annex C.4), and the substitution is listed with the
//! unreadable objects. Before this, the member read as null and a full save wrote it as null (the
//! page fonts of a damaged public file, storytold/craft-libs#1976).

use std::sync::Arc;

use craft_pdf::cos::{Document, ObjRef, Object, SaveOptions, write_full};

/// The font dictionary the cross-reference stream places in object stream 6.
const FONT: &str = "<< /Type /Font /Subtype /TrueType /BaseFont /Fallback /FontFile2 5 0 R >>";

/// What object stream 6 holds.
enum Container {
    /// Data its filter can't decode.
    Undecodable,
    /// A readable stream that doesn't list object 4.
    WithoutMember,
}

/// A one-page file whose cross-reference stream says object 4 (the page's font) is member 0 of
/// object stream 6. With `plain_body`, the file also holds a plain `4 0 obj` with the font
/// dictionary, which no cross-reference entry points at.
fn file(container: Container, plain_body: bool) -> Arc<Vec<u8>> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    let mut put = |out: &mut Vec<u8>, num: u32, body: &[u8]| {
        offsets.push((num, out.len()));
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    put(&mut out, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    put(&mut out, 2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    put(&mut out, 3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R >>");
    if plain_body {
        // Not in `offsets`: only a scan of the file finds it.
        out.extend_from_slice(format!("4 0 obj\n{FONT}\nendobj\n").as_bytes());
    }
    put(&mut out, 5, b"<< /Length 8 >>\nstream\nTTF-data\nendstream");
    let objstm = match container {
        Container::Undecodable => b"<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /Length 12 >>\nstream\nnot deflated\nendstream".to_vec(),
        Container::WithoutMember => b"<< /Type /ObjStm /N 1 /First 5 /Length 7 >>\nstream\n9 0 42\nendstream".to_vec(),
    };
    put(&mut out, 6, &objstm);
    put(&mut out, 7, b"<< /Length 23 >>\nstream\nBT /F1 12 Tf (A) Tj ET\nendstream");
    let xref = out.len();
    // /W [1 4 2]: type, offset or stream number, generation or index.
    let mut rows = vec![0u8, 0, 0, 0, 0, 0xff, 0xff];
    for num in 1..=8u32 {
        let (kind, field, last) = match num {
            4 => (2u8, 6u32, 0u16),
            8 => (1, u32::try_from(xref).unwrap(), 0),
            _ => (1, offsets.iter().find(|(n, _)| *n == num).map(|(_, o)| u32::try_from(*o).unwrap()).unwrap(), 0),
        };
        rows.push(kind);
        rows.extend_from_slice(&field.to_be_bytes());
        rows.extend_from_slice(&last.to_be_bytes());
    }
    out.extend_from_slice(format!("8 0 obj\n<< /Type /XRef /Size 9 /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n", rows.len()).as_bytes());
    out.extend_from_slice(&rows);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    Arc::new(out)
}

/// The page's font as `doc` reads it.
fn font(doc: &Document) -> Arc<Object> {
    let page = doc.get(ObjRef::new(3, 0));
    let resources = page.as_dict().and_then(|p| p.get(b"Resources")).map(|r| doc.resolve(r)).expect("the page has resources");
    let fonts = resources.as_dict().and_then(|r| r.get(b"Font")).map(|f| doc.resolve(f)).expect("the resources have fonts");
    doc.resolve(fonts.as_dict().and_then(|f| f.get(b"F1")).expect("the page names its font"))
}

/// Whether `font` is the font dictionary whose `/FontFile2` resolves, in `doc`, to the program.
fn has_program(doc: &Document, font: &Object) -> bool {
    let program = font.as_dict().and_then(|f| f.get(b"FontFile2")).map(|p| doc.resolve(p));
    matches!(program.as_deref(), Some(Object::Stream(s)) if s.decoded().is_ok_and(|d| d == b"TTF-data"))
}

#[test]
fn a_member_its_stream_cannot_supply_is_read_from_its_plain_body() {
    for container in [Container::Undecodable, Container::WithoutMember] {
        let doc = Document::open(file(container, true)).unwrap();
        let read = font(&doc);
        assert_eq!(read.as_dict().and_then(|f| f.name(b"BaseFont")), Some(&b"Fallback"[..]), "{read:?}");
        assert!(has_program(&doc, &read), "{read:?}");
        let listed = doc.unreadable_objects();
        assert!(
            listed.len() == 1
                && listed[0].starts_with("object 4 0 ")
                && listed[0].contains("object stream 6")
                && listed[0].contains("was read instead"),
            "the substitution is recorded: {listed:?}"
        );
        // A full save keeps the font and its program instead of writing the member as null.
        for object_streams in [false, true] {
            let saved = write_full(&doc, &SaveOptions { object_streams, ..Default::default() }).unwrap();
            let back = Document::open(Arc::new(saved)).unwrap();
            let kept = font(&back);
            assert!(has_program(&back, &kept), "object streams {object_streams}: {kept:?}");
            assert!(back.unreadable_objects().is_empty(), "{:?}", back.unreadable_objects());
        }
    }
}

/// Without a plain body there is nothing to read instead: the member reads as null and is listed
/// as unreadable, as before.
#[test]
fn a_member_without_a_plain_body_stays_unreadable() {
    let doc = Document::open(file(Container::Undecodable, false)).unwrap();
    assert!(matches!(*doc.get(ObjRef::new(4, 0)), Object::Null));
    let listed = doc.unreadable_objects();
    assert!(listed.len() == 1 && listed[0].starts_with("object 4 0 could not be read") && !listed[0].contains("instead"), "{listed:?}");
}

/// A container stream freed or replaced by an edit keeps its members out: the plain body isn't
/// read for them either.
#[test]
fn an_edited_container_keeps_its_members_out() {
    for free in [true, false] {
        let mut edited = Document::open(file(Container::Undecodable, true)).unwrap();
        if free {
            edited.free(ObjRef::new(6, 0));
        } else {
            edited.set(ObjRef::new(6, 0), Object::Int(0));
        }
        assert!(matches!(*edited.get(ObjRef::new(4, 0)), Object::Null), "freed {free}: {:?}", edited.get(ObjRef::new(4, 0)));
    }
}
