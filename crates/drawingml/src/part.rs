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

    const THEME_FILE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements>
<a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme>
<a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light" panose="020F0302020204030204" pitchFamily="34" charset="0"/><a:ea typeface="" panose="02010600030101010101" charset="-122"/><a:cs typeface="" panose="020B0604020202020204"/></a:majorFont><a:minorFont><a:latin typeface="Calibri" panose="020F0502020204030204"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>
<a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements>
<a:objectDefaults/><a:extraClrSchemeLst/><a:custClrLst><a:custClr name="Brand red"><a:srgbClr val="C00000"/></a:custClr><a:custClr name="Brand teal"><a:srgbClr val="008080"/></a:custClr></a:custClrLst>
<a:extLst><a:ext uri="{05A4C25C-085E-4340-85A3-A5531E510DB2}"><thm15:themeFamily xmlns:thm15="http://schemas.microsoft.com/office/thememl/2012/main" name="Office Theme" id="{62F939B6-93AF-4DB8-9C6B-D6C7DFDC589F}" vid="{4A3C46E8-61CC-4603-A589-7422A47A8E4A}"/></a:ext></a:extLst></a:theme>"#;

    fn theme_round_trip(t: &Theme) -> (String, Theme) {
        let xml = String::from_utf8(crate::theme::theme_xml(&mut Part::new(), t)).unwrap();
        let back = crate::theme::read_theme(&mut Part::new(), &parse(xml.as_bytes()).unwrap());
        (xml, back)
    }

    #[test]
    fn a_theme_keeps_system_colours_custom_colours_family_id_and_font_panose() {
        let t = crate::theme::read_theme(&mut Part::new(), &parse(THEME_FILE.as_bytes()).unwrap());
        assert_eq!(t.sys_colors.len(), 2);
        let (xml, back) = theme_round_trip(&t);
        assert!(xml.contains(r#"<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>"#), "{xml}");
        assert!(xml.contains(r#"<a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>"#), "{xml}");
        assert!(xml.contains(r#"<a:dk2><a:srgbClr val="44546A"/></a:dk2>"#), "{xml}");
        assert!(xml.contains("Brand red") && xml.contains("C00000") && xml.contains("Brand teal") && xml.contains("008080"), "{xml}");
        assert!(
            xml.contains("thm15:themeFamily")
                && xml.contains("{62F939B6-93AF-4DB8-9C6B-D6C7DFDC589F}")
                && xml.contains("{4A3C46E8-61CC-4603-A589-7422A47A8E4A}"),
            "{xml}"
        );
        assert!(xml.contains(r#"<a:latin typeface="Calibri Light" panose="020F0302020204030204" pitchFamily="34" charset="0"/>"#), "{xml}");
        assert!(xml.contains(r#"<a:ea typeface="" panose="02010600030101010101" charset="-122"/>"#), "{xml}");
        assert!(xml.contains(r#"<a:cs typeface="" panose="020B0604020202020204"/>"#), "{xml}");
        assert!(xml.contains(r#"<a:latin typeface="Calibri" panose="020F0502020204030204"/>"#), "{xml}");
        assert_eq!(
            (&back.sys_colors, &back.colors.colors, &back.fonts, &back.raw_extra),
            (&t.sys_colors, &t.colors.colors, &t.fonts, &t.raw_extra),
            "read, write, read changes nothing"
        );
        // Order: the colour list comes before the extension list, both after the theme elements.
        let at = |n: &str| xml.find(n).unwrap_or_else(|| panic!("{n} in {xml}"));
        assert!(at("</a:themeElements>") < at("<a:custClrLst>") && at("<a:custClrLst>") < at("<a:extLst><a:ext uri=\"{05A4"), "{xml}");
    }

    #[test]
    fn a_system_colour_slot_that_was_changed_is_written_as_srgb_and_absent_parts_stay_absent() {
        let mut t = crate::theme::read_theme(&mut Part::new(), &parse(THEME_FILE.as_bytes()).unwrap());
        t.colors.set(SchemeSlot::Dk1, crate::color::Rgba::rgb(0x11, 0x22, 0x33));
        let (xml, _) = theme_round_trip(&t);
        assert!(xml.contains(r#"<a:dk1><a:srgbClr val="112233"/></a:dk1>"#), "{xml}");
        assert!(xml.contains(r#"<a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>"#), "{xml}");
        let (plain, back) = theme_round_trip(&Theme::default());
        assert!(!plain.contains("sysClr") && !plain.contains("custClrLst") && !plain.contains("themeFamily") && !plain.contains("panose"), "{plain}");
        assert!(back.sys_colors.is_empty() && back.fonts.minor.latin_attrs.is_empty());
    }

    #[test]
    fn font_panose_is_dropped_when_the_typeface_changes() {
        let mut t = crate::theme::read_theme(&mut Part::new(), &parse(THEME_FILE.as_bytes()).unwrap());
        t.fonts.minor.latin = "Arial".into();
        let (xml, _) = theme_round_trip(&t);
        assert!(xml.contains(r#"<a:latin typeface="Arial"/>"#), "{xml}");
        assert!(!xml.contains("020F0502020204030204"), "{xml}");
        assert!(xml.contains(r#"<a:latin typeface="Calibri Light" panose="020F0302020204030204""#), "the unchanged major font keeps it: {xml}");
    }

    #[test]
    fn system_colour_without_last_clr_gets_it_and_an_unknown_name_becomes_srgb() {
        let file = THEME_FILE
            .replace(r#"<a:sysClr val="windowText" lastClr="000000"/>"#, r#"<a:sysClr val="windowText"/>"#)
            .replace(r#"<a:sysClr val="window" lastClr="FFFFFF"/>"#, r#"<a:sysClr val="notAColor" lastClr="123456"/>"#);
        let t = crate::theme::read_theme(&mut Part::new(), &parse(file.as_bytes()).unwrap());
        let (xml, _) = theme_round_trip(&t);
        assert!(xml.contains(r#"<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>"#), "{xml}");
        assert!(xml.contains(r#"<a:lt1><a:srgbClr val="123456"/></a:lt1>"#), "{xml}");
    }

    #[test]
    fn custom_colours_and_family_id_without_object_defaults_follow_an_empty_one() {
        let file = THEME_FILE.replace("<a:objectDefaults/><a:extraClrSchemeLst/>", "");
        let t = crate::theme::read_theme(&mut Part::new(), &parse(file.as_bytes()).unwrap());
        let (xml, back) = theme_round_trip(&t);
        let at = |n: &str| xml.find(n).unwrap_or_else(|| panic!("{n} in {xml}"));
        assert!(at("</a:themeElements>") < at("<a:objectDefaults") && at("<a:objectDefaults") < at("<a:custClrLst>"), "{xml}");
        assert!(at("<a:custClrLst>") < at("<a:extLst><a:ext uri=\"{05A4"), "{xml}");
        let raw = back.raw_extra.unwrap_or_default();
        assert!(raw.contains("Brand teal") && raw.contains("{62F939B6-93AF-4DB8-9C6B-D6C7DFDC589F}"), "{raw}");
    }

    #[test]
    fn list_styles_keep_def_ppr_in_every_place_they_are_written() {
        use crate::text::{ListStyle, TextBody};
        let src = r#"<p:x xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><a:defPPr marL="457200" algn="ctr"><a:defRPr lang="de-DE" sz="1800"/></a:defPPr><a:lvl1pPr marL="0"><a:defRPr sz="4400"/></a:lvl1pPr></p:x>"#;
        let ls = crate::read::list_style(&mut Part::new(), &parse(src.as_bytes()).unwrap());
        let d = ls.default_level.as_ref().expect("defPPr read");
        assert_eq!(
            (d.para.margin_left, d.para.align, d.run.lang.as_deref(), d.run.size),
            (Some(36.0), Some(crate::text::Align::Center), Some("de-DE"), Some(18.0))
        );
        assert_eq!(ls.level(0).and_then(|l| l.run.size), Some(44.0));
        for tag in ["p:defaultTextStyle", "p:titleStyle", "p:bodyStyle", "p:otherStyle", "p:notesStyle", "a:lstStyle"] {
            let mut w = craft_xml::Writer::new();
            crate::write::list_style(&mut w, &mut Part::new(), tag, &ls);
            let xml = format!(r#"<p:x xmlns:a="a" xmlns:p="p">{}</p:x>"#, w.finish());
            assert!(xml.find("<a:defPPr").unwrap() < xml.find("<a:lvl1pPr").unwrap(), "{tag}: {xml}");
            let back = crate::read::list_style(&mut Part::new(), parse(xml.as_bytes()).unwrap().child(craft_xml::Local(&tag[2..])).unwrap());
            assert_eq!(back, ls, "{tag}");
        }
        // A text body's own list style, and an explicit but empty `a:defPPr`.
        let tb = TextBody { list_style: ls.clone(), ..TextBody::from_text("x") };
        let mut w = craft_xml::Writer::new();
        crate::write::text_body(&mut w, &mut Part::new(), "p:txBody", &tb, false);
        let xml = format!(r#"<r xmlns:a="a" xmlns:p="p">{}</r>"#, w.finish());
        let back = crate::read::text_body(&mut Part::new(), parse(xml.as_bytes()).unwrap().child(craft_xml::Local("txBody")).unwrap());
        assert_eq!(back.list_style, ls);
        let empty = ListStyle { default_level: Some(Box::default()), ..Default::default() };
        assert!(!empty.is_empty());
        let mut w = craft_xml::Writer::new();
        crate::write::list_style(&mut w, &mut Part::new(), "p:defaultTextStyle", &empty);
        assert_eq!(w.finish(), "<p:defaultTextStyle><a:defPPr/></p:defaultTextStyle>");
        // Without `a:defPPr` none is written.
        let mut w = craft_xml::Writer::new();
        crate::write::list_style(&mut w, &mut Part::new(), "p:defaultTextStyle", &ListStyle { default_level: None, ..ls });
        assert!(!w.finish().contains("defPPr"));
    }
}
