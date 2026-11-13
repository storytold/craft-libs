//! The former name of [`craft_svg`]'s geometry reader, CSS colour parser and SVG writer.
//!
//! Everything here is a re-export, so code written against `craft-svg-paths` keeps building.
//! New code depends on `craft-svg`: [`craft_svg::paths`], [`craft_svg::color`] and
//! [`craft_svg::write`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub use craft_svg::paths::*;
pub use craft_svg::{color, write};

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    /// The names the crate had before it became part of `craft-svg`, used the way callers do.
    #[test]
    fn the_old_paths_still_name_the_reader_the_colour_parser_and_the_writer() {
        #[derive(Clone, Default, PartialEq, Debug)]
        struct Cut(bool);
        impl crate::Kind for Cut {
            const PROPERTIES: &'static [&'static str] = &["data-cut"];
            fn of(style: &crate::StyleMap, _class: &str) -> Option<Self> {
                style.get("data-cut").map(|v| Cut(v == "yes"))
            }
        }
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm" viewBox="0 0 10 10"><rect width="4" height="2" stroke="red" data-cut="yes"/></svg>"#;
        let read: crate::Imported<Cut> = crate::import(svg, "a.svg").unwrap();
        assert_eq!((read.width_mm, read.height_mm, read.shapes.len()), (10.0, 10.0, 1));
        let shape: &crate::Shape<Cut> = &read.shapes[0];
        assert_eq!((shape.kind.clone(), shape.stroke.as_deref()), (Cut(true), Some("red")));
        assert!(crate::color::parse_color("red").is_ok());
        let _: (usize, usize, usize, usize, usize) =
            (crate::MAX_SVG_BYTES, crate::MAX_SVG_NODES, crate::MAX_SVG_DEPTH, crate::MAX_SVG_WARNINGS, crate::MAX_CSS_BYTES);
        let _: Option<(crate::Clip, crate::FillRule, crate::Error)> = None;
        let (mut warnings, mut dropped) = (Vec::new(), 0);
        crate::push_warning(&mut warnings, &mut dropped, "w".into());
        assert_eq!(warnings.len(), 1);

        let mut path = kurbo::BezPath::new();
        path.move_to((1.0, 2.0));
        path.line_to((3.5, 2.0));
        let mut w = crate::write::SvgWriter::fragment();
        w.tag("path").attr("d", crate::write::path_data(&path)).empty();
        assert_eq!(w.finish(), r#"<path d="M1,2 L3.5,2"/>"#);
    }
}
