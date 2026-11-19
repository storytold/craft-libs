//! What a reader or writer needs from the package around the part it works on. The caller
//! implements these over its package layer, so nothing here opens or writes a package.

use craft_xml::El;

use crate::{MediaId, SlideId};

/// A relationship of the part being read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rel {
    /// The part name for an internal relationship, the address for an external one.
    pub target: String,
    pub external: bool,
}

/// The part being read.
pub trait Source {
    /// The relationship with this id.
    fn rel(&self, id: &str) -> Option<Rel>;
    /// The media item for a relationship's target: the part named `target`, or a link to the
    /// address `target` when `external`. `None` when the package has no such part.
    fn media(&mut self, target: &str, external: bool) -> Option<MediaId>;
    /// The slide stored in the part named `target`, in a presentation.
    fn slide(&self, _target: &str) -> Option<SlideId> {
        None
    }
    /// The name of the custom show with this id, in a presentation.
    fn custom_show(&self, _id: &str) -> Option<String> {
        None
    }
    /// `el` as text that can be written back into another part: with the namespace
    /// declarations it needs from the part's root (see [`crate::xmlx::keep_xml`]).
    fn keep(&self, el: &El) -> String {
        el.to_xml()
    }
    /// [`Source::keep`], also declaring prefixes that only an enclosing element binds.
    fn keep_inner(&self, el: &El) -> String {
        self.keep(el)
    }
}

/// A [`Source`] for a fragment with no package around it: no relationships, no media.
pub struct NoSource;

impl Source for NoSource {
    fn rel(&self, _id: &str) -> Option<Rel> {
        None
    }
    fn media(&mut self, _target: &str, _external: bool) -> Option<MediaId> {
        None
    }
}

/// The part being written.
pub trait Sink {
    /// The name of the part (`ppt/slides/slide1.xml`); ids derived from content are seeded with it.
    fn part_name(&self) -> &str;
    /// Adds a relationship from this part and returns its id. `kind` is the last segment of the
    /// relationship type (`hyperlink`, `image`).
    fn rel(&mut self, kind: &str, target: &str, external: bool) -> String;
    /// A relationship to the media item, written into the package if it is not there yet.
    /// `kind` is `image`, `audio` or `video`. `None` when there is no such item.
    fn media_rel(&mut self, id: MediaId, kind: &str) -> Option<String>;
    /// A relationship to a stand-in picture, for a picture fill whose media is missing.
    fn placeholder_rel(&mut self) -> String;
    /// Is the item a link to a file outside the package (no bytes)?
    fn media_linked(&self, id: MediaId) -> bool;
    /// The item's file name.
    fn media_name(&self, id: MediaId) -> Option<String>;
    /// A relationship to the part of that slide, in a presentation.
    fn slide_rel(&mut self, _slide: SlideId) -> Option<String> {
        None
    }
    /// The position of the custom show with this name, in a presentation.
    fn custom_show_index(&self, _name: &str) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::color::SchemeSlot;
    use crate::style::{ColorRef, Fill, PictureFill};
    use crate::text::{Action, Hyperlink};
    use crate::theme::Theme;
    use crate::xmlx::parse;

    /// A part with `rId1` → an internal picture, `rId2` → a web page, `rId3` → slide 2.
    #[derive(Default)]
    struct Part {
        asked: Vec<(String, bool)>,
        rels: HashMap<String, (String, bool)>,
        written: Vec<(String, String, bool)>,
    }

    impl Part {
        fn new() -> Part {
            let mut p = Part::default();
            p.rels.insert("rId1".into(), ("ppt/media/image1.png".into(), false));
            p.rels.insert("rId2".into(), ("https://example.com/".into(), true));
            p.rels.insert("rId3".into(), ("ppt/slides/slide2.xml".into(), false));
            p
        }
    }

    impl Source for Part {
        fn rel(&self, id: &str) -> Option<Rel> {
            self.rels.get(id).map(|(target, external)| Rel { target: target.clone(), external: *external })
        }
        fn media(&mut self, target: &str, external: bool) -> Option<MediaId> {
            self.asked.push((target.into(), external));
            (target == "ppt/media/image1.png").then_some(MediaId(7))
        }
        fn slide(&self, target: &str) -> Option<SlideId> {
            (target == "ppt/slides/slide2.xml").then_some(SlideId(2))
        }
    }

    impl Sink for Part {
        fn part_name(&self) -> &str {
            "ppt/slides/slide1.xml"
        }
        fn rel(&mut self, kind: &str, target: &str, external: bool) -> String {
            self.written.push((kind.into(), target.into(), external));
            format!("rId{}", self.written.len())
        }
        fn media_rel(&mut self, id: MediaId, kind: &str) -> Option<String> {
            (id == MediaId(7)).then(|| Sink::rel(self, kind, "../media/image1.png", false))
        }
        fn placeholder_rel(&mut self) -> String {
            Sink::rel(self, "image", "../media/missing.png", false)
        }
        fn media_linked(&self, _id: MediaId) -> bool {
            false
        }
        fn media_name(&self, _id: MediaId) -> Option<String> {
            None
        }
        fn slide_rel(&mut self, slide: SlideId) -> Option<String> {
            (slide == SlideId(2)).then(|| Sink::rel(self, "slide", "slide2.xml", false))
        }
    }

    #[test]
    fn a_picture_fill_finds_its_media_through_the_source_and_writes_through_the_sink() {
        let el = parse(br#"<a:blipFill xmlns:a="A" xmlns:r="R"><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>"#).unwrap();
        let mut part = Part::new();
        let Some(Fill::Picture(pf)) = crate::read::fill_el(&mut part, &el) else { panic!("no picture fill") };
        assert_eq!(pf.media, MediaId(7));
        assert_eq!(part.asked, [("ppt/media/image1.png".to_string(), false)]);
        let mut w = craft_xml::Writer::new();
        crate::write::blip_fill(&mut w, &mut part, "a:blipFill", &pf);
        assert!(w.finish().contains(r#"<a:blip r:embed="rId1""#));
        assert_eq!(part.written, [("image".to_string(), "../media/image1.png".to_string(), false)]);
        // A missing picture gets the stand-in.
        let mut part = Part::new();
        let mut w = craft_xml::Writer::new();
        crate::write::blip_fill(&mut w, &mut part, "a:blipFill", &PictureFill { media: MediaId(99), ..pf });
        assert_eq!(part.written[0].1, "../media/missing.png");
    }

    #[test]
    fn hyperlinks_resolve_to_urls_and_slides() {
        let mut part = Part::new();
        let url = parse(br#"<a:hlinkClick xmlns:a="A" xmlns:r="R" r:id="rId2"/>"#).unwrap();
        let slide = parse(br#"<a:hlinkClick xmlns:a="A" xmlns:r="R" r:id="rId3" action="ppaction://hlinksldjump"/>"#).unwrap();
        let none = parse(br#"<a:hlinkClick xmlns:a="A" xmlns:r="R" r:id="rId9"/>"#).unwrap();
        let h = crate::read::hyperlink(&mut part, &url).unwrap();
        assert_eq!(h.action, Action::Url { url: "https://example.com/".into() });
        let s = crate::read::hyperlink(&mut part, &slide).unwrap();
        assert_eq!(s.action, Action::Slide { slide: SlideId(2) });
        assert!(crate::read::hyperlink(&mut part, &none).is_none());
        assert!(crate::read::hyperlink(&mut NoSource, &slide).is_none());
        let mut w = craft_xml::Writer::new();
        crate::write::hyperlink(&mut w, &mut part, "a:hlinkClick", &Hyperlink { action: Action::Slide { slide: SlideId(2) }, ..h });
        assert_eq!(w.finish(), r#"<a:hlinkClick r:id="rId1" action="ppaction://hlinksldjump"/>"#);
    }

    #[test]
    fn a_theme_survives_writing_and_reading() {
        let mut t = Theme { name: "Round trip".into(), ..Theme::default() };
        t.fonts.major.latin = "Head".into();
        t.format.fills[0] = Fill::solid(ColorRef::scheme(SchemeSlot::PhClr));
        let xml = crate::theme::theme_xml(&mut Part::new(), &t);
        let back = crate::theme::read_theme(&mut Part::new(), &parse(&xml).unwrap());
        assert_eq!(back.name, t.name);
        assert_eq!(back.colors.colors, t.colors.colors);
        assert_eq!(back.fonts.major.latin, "Head");
        assert_eq!(back.format.fills, t.format.fills);
        assert_eq!(back.format.lines.len(), 3);
    }

    #[test]
    fn extra_theme_colours_and_tint_rows_survive_writing_and_reading() {
        use crate::color::{ExtraColor, MAX_EXTRA_COLORS, MAX_TINT_ROWS, Rgba, VARIANT_ROWS};
        let mut t = Theme::default();
        let c = &mut t.colors;
        c.computed_tints = false;
        c.tints.insert(SchemeSlot::Accent5, vec![Rgba::rgb(0xC7, 0xEB, 0xBA), Rgba::rgb(0x0D, 0x4A, 0x21)]);
        c.tints.insert(SchemeSlot::Accent6, vec![]);
        c.tints.insert(SchemeSlot::Dk2, vec![Rgba::WHITE; VARIANT_ROWS]);
        c.extra.push(ExtraColor { name: "Sand".into(), color: Rgba::rgb(0xEB, 0xE5, 0xE0), tints: None });
        c.extra.push(ExtraColor { name: "Broken white".into(), color: Rgba::rgb(0xFC, 0xFC, 0xFC), tints: Some(vec![]) });
        c.extra.push(ExtraColor {
            name: "Grey & \"tones\"".into(),
            color: Rgba::rgb(0xA5, 0xA5, 0xA5),
            tints: Some(vec![Rgba::rgb(0xE6, 0xE6, 0xE6), Rgba::rgb(0x32, 0x32, 0x32)]),
        });
        let xml = String::from_utf8(crate::theme::theme_xml(&mut Part::new(), &t)).unwrap();
        let back = crate::theme::read_theme(&mut Part::new(), &parse(xml.as_bytes()).unwrap());
        assert_eq!(back.colors, t.colors, "lists of 0, 2 and 5 rows, no list, and computed_tints");
        let (cs_start, cs_end) = (xml.find("<a:clrScheme").unwrap(), xml.find("</a:clrScheme>").unwrap());
        let ext = xml.find("<a:extLst>").unwrap();
        assert!(cs_start < ext && ext < cs_end, "the extension belongs to the colour scheme");
        assert!(xml.contains("computed=\"0\""), "{xml}");
        assert!(xml.contains("<dc:color name=\"Sand\" val=\"EBE5E0\"></dc:color>"), "{xml}");
        assert!(xml.contains("<dc:tints slot=\"accent6\"></dc:tints>"), "{xml}");
        assert!(xml.contains("Grey &amp; &quot;tones&quot;"), "{xml}");
        // A plain scheme writes no extension; `computed_tints = false` alone is kept.
        let plain = String::from_utf8(crate::theme::theme_xml(&mut Part::new(), &Theme::default())).unwrap();
        assert!(!plain.contains("extraColors"), "{plain}");
        let mut t = Theme::default();
        t.colors.computed_tints = false;
        let xml = crate::theme::theme_xml(&mut Part::new(), &t);
        assert!(!crate::theme::read_theme(&mut Part::new(), &parse(&xml).unwrap()).colors.computed_tints);
        // Lists and extras past the limits are cut the same way on writing and reading.
        let mut t = Theme::default();
        let long: Vec<Rgba> = (0..MAX_TINT_ROWS + 3).map(|i| Rgba::rgb(i as u8, 0, 0)).collect();
        t.colors.tints.insert(SchemeSlot::Accent1, long.clone());
        for i in 0..MAX_EXTRA_COLORS + 7 {
            t.colors.extra.push(ExtraColor { name: format!("Extra {i}"), color: Rgba::rgb((i % 256) as u8, 0, 0), tints: Some(long.clone()) });
        }
        let xml = crate::theme::theme_xml(&mut Part::new(), &t);
        let back = crate::theme::read_theme(&mut Part::new(), &parse(&xml).unwrap()).colors;
        assert_eq!(back.tints.get(&SchemeSlot::Accent1).map(Vec::as_slice), Some(&long[..MAX_TINT_ROWS]));
        assert_eq!(back.extra.len(), MAX_EXTRA_COLORS);
        assert_eq!(back.extra[0].tints.as_deref(), Some(&long[..MAX_TINT_ROWS]));
    }
}
