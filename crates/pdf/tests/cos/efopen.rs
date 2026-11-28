//! A document that encrypts only its embedded files, authorised when one is opened (`/AuthEvent
//! /EFOpen`, ISO 32000-2 §7.6.6, Table 25): it opens without a password, its pages read as stored,
//! and its embedded files keep their encrypted bytes, flagged as such, through every save.

use std::sync::Arc;

use craft_pdf::cos::{Algorithm, CosError, Dict, Document, NewEncryption, ObjRef, Object, SaveOptions, write_full, write_incremental};
use craft_pdf::crypt::StreamKind;

const PAGE: &str = "BT /F1 12 Tf 20 100 Td (Visible page) Tj ET";
const ATTACHED: &[u8] = b"the attached file's own words";
const PASSWORD: &str = "attach pw";
const EMBEDDED: ObjRef = ObjRef { num: 6, generation: 0 };
const CONTENT: ObjRef = ObjRef { num: 4, generation: 0 };

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// One page in the clear and one attachment encrypted with `/StdCF`, whose `/AuthEvent` is
/// `auth_event`: AES-256 (V5, R6) or AES-128 (V4, R4), `/StmF` and `/StrF` `/Identity`.
fn fixture(algorithm: Algorithm, auth_event: &str) -> Vec<u8> {
    fixture_with(algorithm, auth_event, false)
}

/// [`fixture`], the attachment naming `/StdCF` in a `/Crypt` filter of its own when `named`, with
/// no `/EFF` (ISO 32000-2 §7.4.10).
fn fixture_with(algorithm: Algorithm, auth_event: &str, named: bool) -> Vec<u8> {
    build(algorithm, auth_event, named, true)
}

/// [`fixture`] with `/AuthEvent /EFOpen`, its attachment without `/Type /EmbeddedFile` unless
/// `typed`: the entry is optional (ISO 32000-2 Table 44).
fn fixture_typed(algorithm: Algorithm, typed: bool) -> Vec<u8> {
    build(algorithm, "EFOpen", false, typed)
}

const ID0: [u8; 16] = [7; 16];

/// The handler the fixtures encrypt with.
fn handler(algorithm: Algorithm) -> craft_pdf::crypt::SecurityHandler {
    let params =
        NewEncryption { algorithm, user_password: PASSWORD, owner_password: "owner pw", permissions: -4, encrypt_metadata: true, seed: [3; 32] };
    craft_pdf::crypt::create(&params, &ID0).unwrap()
}

fn build(algorithm: Algorithm, auth_event: &str, named: bool, typed: bool) -> Vec<u8> {
    let id0 = ID0;
    let handler = handler(algorithm);
    let d = handler.dict();
    let kind = if named { StreamKind::Named(b"StdCF") } else { StreamKind::EmbeddedFile };
    let cipher = handler.encrypt_stream(EMBEDDED.num, 0, ATTACHED, kind);
    let (eff, crypt) = if named { ("", " /Filter [/Crypt] /DecodeParms [<< /Name /StdCF >>]") } else { (" /EFF /StdCF", "") };
    let (version, cfm, r6) = match algorithm {
        Algorithm::Aes256 => ("/V 5 /R 6", "/AESV3 /Length 32", format!("/OE <{}> /UE <{}> /Perms <{}>", hex(&d.oe), hex(&d.ue), hex(&d.perms))),
        _ => ("/V 4 /R 4 /Length 128", "/AESV2 /Length 16", String::new()),
    };
    let encrypt = format!(
        "<< /Filter /Standard {version} /O <{}> /U <{}> {r6} /P {} /CF << /StdCF << /AuthEvent /{auth_event} /CFM {cfm} >> >> /StmF /Identity /StrF /Identity{eff} >>",
        hex(&d.o),
        hex(&d.u),
        d.p
    );
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(note.txt) 5 0 R] >> >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 200] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{PAGE}\nendstream", PAGE.len()).into_bytes(),
        b"<< /Type /Filespec /F (note.txt) /UF (note.txt) /EF << /F 6 0 R >> >>".to_vec(),
    ];
    let ty = if typed { "/Type /EmbeddedFile" } else { "" };
    let mut stream = format!("<< {ty}{crypt} /Length {} >>\nstream\n", cipher.len()).into_bytes();
    stream.extend_from_slice(&cipher);
    stream.extend_from_slice(b"\nendstream");
    objs.push(stream);
    objs.push(encrypt.into_bytes());
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    let trailer = format!(
        "trailer\n<< /Size {} /Root 1 0 R /Encrypt 7 0 R /ID [<{}> <{}>] >>\nstartxref\n{xref}\n%%EOF\n",
        objs.len() + 1,
        hex(&id0),
        hex(&id0)
    );
    out.extend_from_slice(trailer.as_bytes());
    out
}

fn stream(doc: &Document, r: ObjRef) -> craft_pdf::cos::Stream {
    match &*doc.get(r) {
        Object::Stream(s) => s.clone(),
        other => panic!("object {} isn't a stream: {other:?}", r.num),
    }
}

/// The attachment as the password reads it from `bytes`.
fn attachment_with_password(bytes: Vec<u8>) -> Vec<u8> {
    let doc = Document::open_with_password(Arc::new(bytes), Some(PASSWORD)).unwrap();
    assert!(!doc.embedded_files_locked());
    let embedded = doc.root().and_then(|r| doc.get(r).as_dict().cloned()).unwrap();
    assert!(embedded.get(b"Names").is_some());
    stream(&doc, EMBEDDED).decoded().unwrap()
}

#[test]
fn a_document_encrypting_only_its_embedded_files_opens_without_a_password() {
    let bytes = fixture(Algorithm::Aes256, "EFOpen");
    let doc = Document::open(Arc::new(bytes.clone())).unwrap();
    assert!(doc.embedded_files_locked());
    // Not a repair: the specification has it open so.
    assert!(doc.repair_log().is_empty(), "{:?}", doc.repair_log());
    assert!(doc.trailer().get(b"Encrypt").is_some());
    // The page reads as stored; the attachment keeps its encrypted bytes and says so.
    let content = stream(&doc, CONTENT);
    assert_eq!(content.decoded().unwrap(), PAGE.as_bytes());
    assert!(!doc.reads_as_stored(&content));
    let embedded = stream(&doc, EMBEDDED);
    assert!(doc.reads_as_stored(&embedded));
    assert_ne!(embedded.decoded().unwrap(), ATTACHED);
    // The password still opens it, and reads the attachment.
    assert_eq!(attachment_with_password(bytes.clone()), ATTACHED);
    // A wrong password is still refused.
    assert!(matches!(Document::open_with_password(Arc::new(bytes), Some("nope")), Err(CosError::WrongPassword)));
}

#[test]
fn a_crypt_filter_authorised_when_the_document_opens_still_needs_the_password() {
    let bytes = fixture(Algorithm::Aes256, "DocOpen");
    assert!(matches!(Document::open(Arc::new(bytes.clone())), Err(CosError::NeedsPassword)));
    assert_eq!(attachment_with_password(bytes), ATTACHED);
}

#[test]
fn saves_keep_the_encrypted_embedded_files_as_stored() {
    for algorithm in [Algorithm::Aes256, Algorithm::Aes128] {
        let mut doc = Document::open(Arc::new(fixture(algorithm, "EFOpen"))).unwrap();
        assert!(doc.embedded_files_locked(), "{algorithm:?}");
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| c.set(b"PageMode".to_vec(), Object::name("UseAttachments"))).unwrap();
        // An incremental save keeps /Encrypt, and the attachment decrypts with the password.
        let saved = write_incremental(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(attachment_with_password(saved), ATTACHED, "{algorithm:?}");
        // A full save renumbers: AES-256 keys don't depend on the number, AES-128 keys do.
        match algorithm {
            Algorithm::Aes256 => {
                let full = write_full(&doc, &SaveOptions::default()).unwrap();
                let reopened = Document::open(Arc::new(full.clone())).unwrap();
                assert!(reopened.embedded_files_locked());
                let doc = Document::open_with_password(Arc::new(full), Some(PASSWORD)).unwrap();
                let names = doc.root().and_then(|r| doc.get(r).as_dict().cloned()).unwrap();
                let tree = doc.resolve(names.get(b"Names").unwrap()).as_dict().cloned().unwrap();
                let files = doc.resolve(tree.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
                let list = doc.resolve(files.get(b"Names").unwrap()).as_array().cloned().unwrap();
                let spec = doc.resolve(&list[1]).as_dict().cloned().unwrap();
                let ef = doc.resolve(spec.get(b"EF").unwrap()).as_dict().cloned().unwrap();
                let file = match &*doc.resolve(ef.get(b"F").unwrap()) {
                    Object::Stream(s) => s.decoded().unwrap(),
                    other => panic!("{other:?}"),
                };
                assert_eq!(file, ATTACHED);
            }
            _ => assert!(matches!(write_full(&doc, &SaveOptions::default()), Err(CosError::Security(_))), "{algorithm:?}"),
        }
    }
}

#[test]
fn security_cannot_change_without_the_embedded_files_password() {
    let mut doc = Document::open(Arc::new(fixture(Algorithm::Aes256, "EFOpen"))).unwrap();
    doc.remove_encryption();
    assert!(matches!(write_full(&doc, &SaveOptions::default()), Err(CosError::Security(_))));
    assert!(matches!(write_incremental(&doc, &SaveOptions::default()), Err(CosError::Security(_))));
}

/// From the review of the EFOpen change: the empty password opens it too, and the credentials of a
/// document opened without the key open no other.
#[test]
fn the_empty_password_opens_it_and_keyless_credentials_open_nothing_more() {
    let bytes = fixture(Algorithm::Aes256, "EFOpen");
    let doc = Document::open_with_password(Arc::new(bytes.clone()), Some("")).unwrap();
    assert!(doc.embedded_files_locked());
    assert!(doc.open_with_same_credentials(Arc::new(bytes)).unwrap().embedded_files_locked());
    // The same /Encrypt values and /ID, authorised when the document opens.
    let doc_open = fixture(Algorithm::Aes256, "DocOpen");
    assert!(matches!(doc.open_with_same_credentials(Arc::new(doc_open.clone())), Err(CosError::NeedsPassword)));
    assert!(matches!(Document::open_with_password(Arc::new(doc_open), Some("")), Err(CosError::WrongPassword)));
}

/// An embedded file that names an EFOpen crypt filter of its own, with no `/EFF`, opens the same way.
#[test]
fn an_embedded_file_naming_its_own_crypt_filter_opens_without_a_password() {
    let bytes = fixture_with(Algorithm::Aes256, "EFOpen", true);
    let doc = Document::open(Arc::new(bytes.clone())).unwrap();
    assert!(doc.embedded_files_locked());
    assert!(doc.reads_as_stored(&stream(&doc, EMBEDDED)));
    assert_eq!(stream(&doc, CONTENT).decoded().unwrap(), PAGE.as_bytes());
    assert_eq!(attachment_with_password(bytes), ATTACHED);
}

/// A save can't encrypt without the key: removing `/Encrypt` through the trailer, or putting in an
/// embedded file, is refused rather than written in the clear.
#[test]
fn saves_refuse_what_would_need_the_key() {
    let open = || Document::open(Arc::new(fixture(Algorithm::Aes256, "EFOpen"))).unwrap();
    let mut doc = open();
    doc.trailer_mut().remove(b"Encrypt");
    assert!(matches!(write_incremental(&doc, &SaveOptions::default()), Err(CosError::Security(_))));
    assert!(matches!(write_full(&doc, &SaveOptions::default()), Err(CosError::Security(_))));
    let mut doc = open();
    let mut replaced = stream(&doc, EMBEDDED);
    replaced.raw = b"new words".to_vec().into();
    doc.set(EMBEDDED, Object::Stream(replaced));
    assert!(matches!(write_incremental(&doc, &SaveOptions::default()), Err(CosError::Security(_))));
}

/// The password given after opening unlocks the embedded files; a wrong one doesn't.
#[test]
fn the_password_given_later_unlocks_the_embedded_files() {
    let mut doc = Document::open(Arc::new(fixture(Algorithm::Aes256, "EFOpen"))).unwrap();
    assert!(matches!(doc.unlock("nope"), Err(CosError::WrongPassword)));
    assert!(doc.embedded_files_locked());
    doc.unlock(PASSWORD).unwrap();
    assert!(!doc.embedded_files_locked());
    assert_eq!(stream(&doc, EMBEDDED).decoded().unwrap(), ATTACHED);
    assert_eq!(stream(&doc, CONTENT).decoded().unwrap(), PAGE.as_bytes());
    // Not while an embedded file read as stored has been edited.
    let mut doc = Document::open(Arc::new(fixture(Algorithm::Aes256, "EFOpen"))).unwrap();
    let edited = stream(&doc, EMBEDDED);
    doc.set(EMBEDDED, Object::Stream(edited));
    assert!(matches!(doc.unlock(PASSWORD), Err(CosError::Security(_))));
}

/// The attachment the names tree lists, as `doc` holds it, and its object number.
fn listed_attachment(doc: &Document) -> (u32, craft_pdf::cos::Stream) {
    let catalog = doc.root().and_then(|r| doc.get(r).as_dict().cloned()).unwrap();
    let names = doc.resolve(catalog.get(b"Names").unwrap()).as_dict().cloned().unwrap();
    let files = doc.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
    let list = doc.resolve(files.get(b"Names").unwrap()).as_array().cloned().unwrap();
    let spec = doc.resolve(&list[1]).as_dict().cloned().unwrap();
    let ef = doc.resolve(spec.get(b"EF").unwrap()).as_dict().cloned().unwrap();
    let Some(Object::Ref(r)) = ef.get(b"F") else { panic!("the file specification's /EF /F isn't a reference") };
    (r.num, stream(doc, *r))
}

/// An embedded file stream without `/Type` (optional, ISO 32000-2 Table 44) is one because a file
/// specification's `/EF` refers to it (§7.11.4), and `/EFF` applies to it as to one with the entry:
/// without the password its data reads as stored, and an edit of it, which only the key could
/// encrypt, isn't saved. The page's content, in the clear, still reads as it is.
#[test]
fn an_embedded_file_without_a_type_reads_as_stored_without_the_password() {
    for typed in [true, false] {
        let mut doc = Document::open(Arc::new(fixture_typed(Algorithm::Aes256, typed))).unwrap();
        assert!(doc.embedded_files_locked(), "typed: {typed}");
        let embedded = stream(&doc, EMBEDDED);
        assert_eq!(embedded.dict.get(b"Type").is_some(), typed);
        assert!(doc.reads_as_stored_at(EMBEDDED.num, &embedded), "typed: {typed}");
        assert_ne!(embedded.decoded().unwrap(), ATTACHED);
        let content = stream(&doc, CONTENT);
        assert!(!doc.reads_as_stored_at(CONTENT.num, &content), "typed: {typed}");
        assert_eq!(content.decoded().unwrap(), PAGE.as_bytes());
        let mut replaced = embedded;
        replaced.raw = b"new words".to_vec().into();
        doc.set(EMBEDDED, Object::Stream(replaced));
        assert!(matches!(write_incremental(&doc, &SaveOptions::default()), Err(CosError::Security(_))), "typed: {typed}");
    }
}

/// With the password, an embedded file stream without `/Type` decrypts with `/EFF` on reading, and
/// a full save, which renumbers it, encrypts it again under its new number: AES-128 keys depend on
/// the number, so data written as it was read would decrypt to noise.
#[test]
fn an_embedded_file_without_a_type_is_encrypted_again_under_its_new_number() {
    for typed in [true, false] {
        let bytes = fixture_typed(Algorithm::Aes128, typed);
        assert_eq!(attachment_with_password(bytes.clone()), ATTACHED, "typed: {typed}");
        let mut doc = Document::open_with_password(Arc::new(bytes), Some(PASSWORD)).unwrap();
        // An object the catalog refers to, which the save numbers before the attachment.
        let mut outlines = Dict::new();
        outlines.set(b"Type".to_vec(), Object::name("Outlines"));
        outlines.set(b"Count".to_vec(), Object::Int(0));
        let outlines = doc.add(Object::Dict(outlines));
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| c.set(b"Outlines".to_vec(), Object::Ref(outlines))).unwrap();
        let full = write_full(&doc, &SaveOptions::default()).unwrap();
        // Without the password the save's attachment is held as it is stored.
        let stored = Document::open(Arc::new(full.clone())).unwrap();
        let (num, written) = listed_attachment(&stored);
        assert_ne!(num, EMBEDDED.num, "the full save renumbers the attachment");
        assert!(stored.reads_as_stored_at(num, &written), "typed: {typed}");
        assert_eq!(handler(Algorithm::Aes128).decrypt_stream(num, 0, &written.raw, StreamKind::EmbeddedFile), ATTACHED, "typed: {typed}");
        // And the password reads it.
        let reopened = Document::open_with_password(Arc::new(full), Some(PASSWORD)).unwrap();
        assert_eq!(listed_attachment(&reopened).1.decoded().unwrap(), ATTACHED, "typed: {typed}");
    }
}

/// The page's content stream, through the page tree.
fn page_content(doc: &Document) -> (u32, craft_pdf::cos::Stream) {
    let catalog = doc.root().and_then(|r| doc.get(r).as_dict().cloned()).unwrap();
    let pages = doc.resolve(catalog.get(b"Pages").unwrap()).as_dict().cloned().unwrap();
    let kids = doc.resolve(pages.get(b"Kids").unwrap()).as_array().cloned().unwrap();
    let page = doc.resolve(&kids[0]).as_dict().cloned().unwrap();
    let Some(Object::Ref(r)) = page.get(b"Contents") else { panic!("the page's /Contents isn't a reference") };
    (r.num, stream(doc, *r))
}

/// A file specification can be anywhere, so every object of the file is looked in. Ones that are
/// their own `/EF`, or each other's, or sit deep in arrays naming objects the file doesn't have
/// or generations its objects don't have, open and save, and change how nothing else is read.
#[test]
fn hostile_embedded_file_dictionaries_change_nothing_else() {
    let mut doc = Document::open(Arc::new(fixture_typed(Algorithm::Aes256, false))).unwrap();
    let own = doc.add(Object::Null);
    let mut d = Dict::new();
    d.set(b"EF".to_vec(), Object::Ref(own));
    doc.set(own, Object::Dict(d));
    let (a, b) = (doc.add(Object::Null), doc.add(Object::Null));
    for (x, y) in [(a, b), (b, a)] {
        let mut d = Dict::new();
        d.set(b"EF".to_vec(), Object::Ref(y));
        d.set(b"Next".to_vec(), Object::Ref(y));
        doc.set(x, Object::Dict(d));
    }
    let mut ef = Dict::new();
    ef.set(b"F".to_vec(), Object::Ref(ObjRef { num: 999_999, generation: 0 }));
    ef.set(b"UF".to_vec(), Object::Ref(ObjRef { num: u32::MAX, generation: 0 }));
    // The page's content under a generation it doesn't have: a reference to null (§7.3.10).
    ef.set(b"DOS".to_vec(), Object::Ref(ObjRef { num: CONTENT.num, generation: 1 }));
    let mut spec = Dict::new();
    spec.set(b"EF".to_vec(), Object::Dict(ef));
    let mut nested = Object::Dict(spec);
    for _ in 0..100 {
        nested = Object::Array(vec![nested]);
    }
    let deep = doc.add(nested);
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"AF".to_vec(), Object::Array([own, a, deep].into_iter().map(Object::Ref).collect()))).unwrap();
    let saved = write_incremental(&doc, &SaveOptions::default()).unwrap();
    for bytes in [saved.clone(), write_full(&Document::open(Arc::new(saved)).unwrap(), &SaveOptions::default()).unwrap()] {
        let doc = Document::open(Arc::new(bytes)).unwrap();
        assert!(doc.embedded_files_locked());
        let (num, attachment) = listed_attachment(&doc);
        assert!(doc.reads_as_stored_at(num, &attachment));
        let (num, content) = page_content(&doc);
        assert!(!doc.reads_as_stored_at(num, &content));
        assert_eq!(content.decoded().unwrap(), PAGE.as_bytes());
    }
}
