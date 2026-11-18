//! Borrowed views of XFL's bitmap, sound, and video library-item records.
//!
//! These views expose native XFL metadata and archive member names. Decoding those bytes and
//! mapping the records into an application's media model remain consumer responsibilities.

use crate::shape::XflElement;

/// Archive member named by a library item's native `href` attribute.
pub fn library_member(href: &str) -> String {
    format!("LIBRARY/{href}")
}

/// Archive member named by an XFL item's native `*DataHRef` attribute.
pub fn data_member(href: &str) -> String {
    format!("bin/{href}")
}

/// A packaged `./...` external video source, normalized to ZIP's slash separators.
pub fn packaged_external_member(path: &str) -> Option<String> {
    path.strip_prefix("./").map(|path| path.replace('\\', "/"))
}

/// Media library records in an XFL document, in source order.
#[derive(Clone, Copy, Debug)]
pub struct Catalog<'a, E: XflElement> {
    media: Option<&'a E>,
}

impl<'a, E: XflElement> Catalog<'a, E> {
    pub fn new(document: &'a E) -> Self {
        Self { media: document.child("media") }
    }

    pub fn bitmaps(&self) -> impl Iterator<Item = Bitmap<'a, E>> + 'a {
        let media = self.media;
        media.into_iter().flat_map(|media| media.elements_named("DOMBitmapItem")).map(|element| Bitmap { element })
    }

    pub fn sounds(&self) -> impl Iterator<Item = Sound<'a, E>> + 'a {
        let media = self.media;
        media.into_iter().flat_map(|media| media.elements_named("DOMSoundItem")).map(|element| Sound { element })
    }

    pub fn videos(&self) -> impl Iterator<Item = Video<'a, E>> + 'a {
        let media = self.media;
        media.into_iter().flat_map(|media| media.elements_named("DOMVideoItem")).map(|element| Video { element })
    }
}

/// Native bitmap library metadata.
#[derive(Clone, Copy, Debug)]
pub struct Bitmap<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> Bitmap<'a, E> {
    pub fn name(&self) -> Option<&'a str> {
        self.element.attr("name")
    }

    pub fn href(&self) -> Option<&'a str> {
        self.element.attr("href")
    }

    pub fn data_href(&self) -> Option<&'a str> {
        self.element.attr("bitmapDataHRef")
    }

    pub fn allow_smoothing(&self) -> Option<bool> {
        self.element.attr("allowSmoothing").map(|value| value.trim() == "true")
    }
}

/// Native sound library metadata.
#[derive(Clone, Copy, Debug)]
pub struct Sound<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> Sound<'a, E> {
    pub fn name(&self) -> Option<&'a str> {
        self.element.attr("name")
    }

    pub fn href(&self) -> Option<&'a str> {
        self.element.attr("href")
    }

    pub fn data_href(&self) -> Option<&'a str> {
        self.element.attr("soundDataHRef")
    }
}

/// Native video library metadata.
#[derive(Clone, Copy, Debug)]
pub struct Video<'a, E: XflElement> {
    element: &'a E,
}

impl<'a, E: XflElement> Video<'a, E> {
    pub fn name(&self) -> Option<&'a str> {
        self.element.attr("name")
    }

    pub fn href(&self) -> Option<&'a str> {
        self.element.attr("href")
    }

    pub fn external_path(&self) -> Option<&'a str> {
        self.element.attr("sourceExternalFilepath")
    }

    pub fn data_href(&self) -> Option<&'a str> {
        self.element.attr("videoDataHRef")
    }

    pub fn length(&self) -> Option<f32> {
        number(self.element, "length")
    }

    pub fn frame_count(&self) -> Option<f32> {
        number(self.element, "frameCount")
    }

    pub fn frames_per_second(&self) -> Option<f32> {
        number(self.element, "fps")
    }
}

fn number(element: &impl XflElement, name: &str) -> Option<f32> {
    element.attr(name)?.trim().parse::<f32>().ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &[u8]) -> craft_xml::El {
        craft_xml::Options::default().parse(source).unwrap()
    }

    #[test]
    fn reads_native_media_metadata_in_order_and_ignores_other_records() {
        let root = parse(
            br#"<DOMDocument><media>
            <DOMBitmapItem name="folder/a" href="a.png" bitmapDataHRef="Bitmap 1" allowSmoothing="false"/>
            <DOMSoundItem name="s" href="s.wav" soundDataHRef="Sound 1"/>
            <DOMVideoItem name="v" sourceExternalFilepath="./LIBRARY/v.flv" videoDataHRef="Video 1" length="3.5" frameCount="70" fps="20"/>
            <Other name="ignored"/>
            <DOMBitmapItem name="b"/>
        </media></DOMDocument>"#,
        );
        let catalog = Catalog::new(&root);
        let bitmaps: Vec<_> = catalog.bitmaps().collect();
        assert_eq!(bitmaps.len(), 2);
        assert_eq!(bitmaps[0].name(), Some("folder/a"));
        assert_eq!(bitmaps[0].href(), Some("a.png"));
        assert_eq!(bitmaps[0].data_href(), Some("Bitmap 1"));
        assert_eq!(bitmaps[0].allow_smoothing(), Some(false));
        assert_eq!(
            catalog.sounds().next().map(|item| (item.name(), item.href(), item.data_href())),
            Some((Some("s"), Some("s.wav"), Some("Sound 1")))
        );
        let video = catalog.videos().next().expect("video record");
        assert_eq!(video.external_path(), Some("./LIBRARY/v.flv"));
        assert_eq!(video.data_href(), Some("Video 1"));
        assert_eq!(video.length(), Some(3.5));
        assert_eq!(video.frame_count(), Some(70.0));
        assert_eq!(video.frames_per_second(), Some(20.0));
    }

    #[test]
    fn missing_or_invalid_media_metadata_stays_optional() {
        let root = parse(br#"<DOMDocument><media><DOMVideoItem name="v" length="NaN" fps="?"/></media></DOMDocument>"#);
        let video = Catalog::new(&root).videos().next().expect("video record");
        assert_eq!(video.name(), Some("v"));
        assert_eq!(video.length(), None);
        assert_eq!(video.frames_per_second(), None);
        let empty = parse(b"<DOMDocument/>");
        assert_eq!(Catalog::new(&empty).bitmaps().count(), 0);
    }

    #[test]
    fn native_media_references_resolve_to_the_legacy_archive_members() {
        assert_eq!(library_member("folder/a.png"), "LIBRARY/folder/a.png");
        assert_eq!(data_member("Bitmap 1"), "bin/Bitmap 1");
        assert_eq!(packaged_external_member("./LIBRARY\\v.flv").as_deref(), Some("LIBRARY/v.flv"));
        assert_eq!(packaged_external_member("/outside/v.flv"), None);
    }
}
