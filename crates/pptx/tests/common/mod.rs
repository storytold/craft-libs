//! What the integration tests share: a small deck to start from, and package surgery.
#![allow(dead_code)] // each test file uses some of these

use std::sync::Arc;

use craft_pptx::{Defaults, Plain, Presentation, Slide, SlideId};

/// A deck with [`Plain`]'s master and one slide on its layout.
pub fn deck() -> Presentation {
    let mut p = Presentation::default();
    let (m, last) = Plain.master(p.slide_size, 0);
    p.masters.push(Arc::new(m));
    p.next_id = last;
    add_slide(&mut p);
    p
}

/// Appends a blank slide on the first layout.
pub fn add_slide(p: &mut Presentation) -> &mut Slide {
    let layout = p.masters[0].layouts[0].id;
    let id = SlideId(p.alloc_id().unwrap());
    p.slides.push(Arc::new(Slide { id, layout, ..Default::default() }));
    Arc::make_mut(p.slides.last_mut().unwrap())
}

pub fn export(p: &Presentation) -> Vec<u8> {
    craft_pptx::export(p, &Plain).unwrap()
}

pub fn import(bytes: &[u8]) -> Presentation {
    craft_pptx::import(bytes, &Plain).unwrap()
}

pub fn bytes_of(pptx: &[u8], name: &str) -> Vec<u8> {
    craft_opc::Package::open(pptx, &craft_pptx::LIMITS).unwrap().get(name).unwrap_or_else(|| panic!("no part {name}")).to_vec()
}

pub fn part(pptx: &[u8], name: &str) -> String {
    String::from_utf8(bytes_of(pptx, name)).unwrap()
}

/// The package with each part passed through `f`: `None` drops it. `extra` parts are added.
pub fn rewrite(pptx: &[u8], f: impl Fn(&str, &[u8]) -> Option<Vec<u8>>, extra: &[(&str, &str)]) -> Vec<u8> {
    let pkg = craft_opc::Package::open(pptx, &craft_pptx::LIMITS).unwrap();
    let mut out = craft_opc::ZipOut::new();
    for name in pkg.names() {
        if let Some(data) = f(name, pkg.get(name).unwrap()) {
            out.add(name, &data, craft_opc::Compression::Deflate).unwrap();
        }
    }
    for (name, data) in extra {
        out.add(name, data.as_bytes(), craft_opc::Compression::Deflate).unwrap();
    }
    out.finish().unwrap()
}

/// Every element and attribute prefix in `xml` is declared on it or an ancestor.
pub fn assert_prefixes_bound(name: &str, xml: &[u8]) {
    fn rec(e: &craft_xml::El, scope: &mut Vec<String>, name: &str) {
        let n = scope.len();
        scope.extend(e.attrs.iter().filter_map(|(k, _)| k.strip_prefix("xmlns:").map(str::to_string)));
        let bound = |q: &str, scope: &Vec<String>| q.split_once(':').is_none_or(|(p, _)| p == "xml" || p == "xmlns" || scope.iter().any(|s| s == p));
        assert!(bound(&e.name, scope), "{name}: unbound prefix on <{}>", e.name);
        for (k, _) in &e.attrs {
            assert!(bound(k, scope), "{name}: unbound prefix on attribute {k} of <{}>", e.name);
        }
        for c in e.elements() {
            rec(c, scope, name);
        }
        scope.truncate(n);
    }
    let root = craft_xml::Options::default().strict().parse(xml).unwrap_or_else(|e| panic!("{name}: not well-formed: {e}"));
    rec(&root, &mut vec![], name);
}
