//! Standardized media metadata conforming to Adobe XMP and Dublin Core specifications.

use serde::{Deserialize, Serialize};

use crate::timecode::Rational64;

/// Standardized media metadata conforming to Adobe XMP and Dublin Core specifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct XmpMetadata {
    /// Document or media title (`dc:title`).
    #[serde(default)]
    pub title: Option<String>,
    /// Primary author or creator (`dc:creator`).
    #[serde(default)]
    pub creator: Option<String>,
    /// Album or collection name (`xmpDM:album`).
    #[serde(default)]
    pub album: Option<String>,
    /// Track number within the album (`xmpDM:trackNumber`).
    #[serde(default)]
    pub track_number: Option<u32>,
    /// Release year or date (`xmpDM:releaseDate`).
    #[serde(default)]
    pub year: Option<u32>,
    /// Musical or media genre (`xmpDM:genre`).
    #[serde(default)]
    pub genre: Option<String>,
    /// Summary or description (`dc:description`).
    #[serde(default)]
    pub description: Option<String>,
    /// Copyright / intellectual rights notice (`dc:rights`).
    #[serde(default)]
    pub copyright: Option<String>,
    /// Source project or composition name (`xmp:ProjectName`).
    #[serde(default)]
    pub project_name: Option<String>,
    /// Creation or ingest timestamp in ISO 8601 (`xmp:CreateDate`).
    #[serde(default)]
    pub create_date: Option<String>,
    /// Starting timecode string e.g. `"01:00:00:00"` (`xmpDM:startTimecode`).
    #[serde(default)]
    pub start_timecode: Option<String>,
    /// Frame rate of `start_timecode`, which gives the `xmpDM:timeFormat` of the structure.
    #[serde(default)]
    pub start_timecode_rate: Option<Rational64>,
    /// Composer or songwriter (`xmpDM:composer`).
    #[serde(default)]
    pub composer: Option<String>,
    /// International Standard Recording Code (ISRC) for music track identification.
    #[serde(default)]
    pub isrc: Option<String>,
    /// Whether to generate an external sidecar `.xmp` file alongside the output.
    #[serde(default)]
    pub export_sidecar_xmp: bool,
}

impl XmpMetadata {
    /// The `xmpDM:timeFormat` value for `start_timecode`, when its rate is a standard one.
    /// A `;` before the frames marks drop-frame counting.
    fn start_timecode_format(&self) -> Option<&'static str> {
        let raw = self.start_timecode_rate?;
        let rate = Rational64::new(raw.num, raw.den).ok().filter(|r| r.num > 0 && r.den > 0)?;
        let drop = self.start_timecode.as_deref().is_some_and(|t| t.contains(';'));
        Some(match (rate.num, rate.den, drop) {
            (24_000, 1_001, _) => "23976Timecode",
            (24, 1, _) => "24Timecode",
            (25, 1, _) => "25Timecode",
            (30_000, 1_001, true) => "2997DropTimecode",
            (30_000, 1_001, false) => "2997NonDropTimecode",
            (30, 1, _) => "30Timecode",
            (50, 1, _) => "50Timecode",
            (60_000, 1_001, true) => "5994DropTimecode",
            (60_000, 1_001, false) => "5994NonDropTimecode",
            (60, 1, _) => "60Timecode",
            _ => return None,
        })
    }

    /// Generates a standardized, Adobe-compliant XMP packet in XML/RDF format.
    ///
    /// The packet includes standard `xpacket` PI headers and Dublin Core / XMP Dynamic Media tags.
    pub fn to_xmp_packet(&self) -> String {
        let mut dc_elements = String::new();

        if let Some(ref title) = self.title {
            let escaped = escape_xml(title);
            dc_elements.push_str(&format!("      <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{escaped}</rdf:li></rdf:Alt></dc:title>\n"));
        }

        if let Some(ref creator) = self.creator {
            let escaped = escape_xml(creator);
            dc_elements.push_str(&format!("      <dc:creator><rdf:Seq><rdf:li>{escaped}</rdf:li></rdf:Seq></dc:creator>\n"));
        }

        if let Some(ref desc) = self.description {
            let escaped = escape_xml(desc);
            dc_elements
                .push_str(&format!("      <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{escaped}</rdf:li></rdf:Alt></dc:description>\n"));
        }

        if let Some(ref rights) = self.copyright {
            let escaped = escape_xml(rights);
            dc_elements.push_str(&format!("      <dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">{escaped}</rdf:li></rdf:Alt></dc:rights>\n"));
        }

        let mut xmp_elements = String::new();
        xmp_elements.push_str("      <xmp:CreatorTool>encoderCraft 0.1.0 (ArtCraft)</xmp:CreatorTool>\n");

        if let Some(ref date) = self.create_date {
            let escaped = escape_xml(date);
            xmp_elements.push_str(&format!("      <xmp:CreateDate>{escaped}</xmp:CreateDate>\n"));
        }

        if let Some(ref proj) = self.project_name {
            let escaped = escape_xml(proj);
            xmp_elements.push_str(&format!("      <xmp:ProjectName>{escaped}</xmp:ProjectName>\n"));
        }

        let mut dm_elements = String::new();
        if let Some(ref tc) = self.start_timecode {
            let value = escape_xml(tc);
            // `xmpDM:startTimecode` is a Timecode structure: a time value and the format that
            // gives its frame rate. Without a rate only the value is known.
            let format = match self.start_timecode_format() {
                Some(f) => format!(" xmpDM:timeFormat=\"{f}\""),
                None => String::new(),
            };
            dm_elements.push_str(&format!(
                "      <xmpDM:startTimecode>\n        <rdf:Description{format} xmpDM:timeValue=\"{value}\"/>\n      </xmpDM:startTimecode>\n"
            ));
        }
        if let Some(ref album) = self.album {
            let escaped = escape_xml(album);
            dm_elements.push_str(&format!("      <xmpDM:album>{escaped}</xmpDM:album>\n"));
        }
        if let Some(track) = self.track_number {
            dm_elements.push_str(&format!("      <xmpDM:trackNumber>{track}</xmpDM:trackNumber>\n"));
        }
        if let Some(year) = self.year {
            dm_elements.push_str(&format!("      <xmpDM:releaseDate>{year}</xmpDM:releaseDate>\n"));
        }
        if let Some(ref genre) = self.genre {
            let escaped = escape_xml(genre);
            dm_elements.push_str(&format!("      <xmpDM:genre>{escaped}</xmpDM:genre>\n"));
        }
        if let Some(ref composer) = self.composer {
            let escaped = escape_xml(composer);
            dm_elements.push_str(&format!("      <xmpDM:composer>{escaped}</xmpDM:composer>\n"));
        }
        if let Some(ref isrc) = self.isrc {
            let escaped = escape_xml(isrc);
            dm_elements.push_str(&format!("      <xmpDM:ISRC>{escaped}</xmpDM:ISRC>\n"));
        }

        format!(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
             <x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"encoderCraft XMP Core 0.1.0\">\n\
             \x20<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
             \x20 <rdf:Description rdf:about=\"\"\n\
             \x20   xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n\
             \x20   xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n\
             \x20   xmlns:xmpDM=\"http://ns.adobe.com/xmp/1.0/DynamicMedia/\">\n\
             {dc_elements}\
             {xmp_elements}\
             {dm_elements}\
             \x20 </rdf:Description>\n\
             \x20</rdf:RDF>\n\
             </x:xmpmeta>\n\
             <?xpacket end=\"w\"?>"
        )
    }
}

/// Escapes `s` for XML 1.0 text and attribute values. Characters XML forbids (NUL, most C0
/// controls, U+FFFE and U+FFFF) are replaced with U+FFFD so the packet stays well-formed.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c < ' ' || c == '\u{FFFE}' || c == '\u{FFFF}' => out.push('\u{FFFD}'),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Rational64, XmpMetadata, escape_xml};

    #[test]
    fn test_xmp_packet_generation() {
        let meta = XmpMetadata {
            title: Some("Cinematic Master 4K".into()),
            creator: Some("ArtCraft Team".into()),
            album: Some("ArtCraft OST".into()),
            track_number: Some(3),
            year: Some(2026),
            genre: Some("Soundtrack".into()),
            description: Some("Trailer cut & audio mix".into()),
            copyright: Some("© 2026 Storytold Organization".into()),
            project_name: Some("FilmCraft Sequence 1".into()),
            create_date: Some("2026-11-02T12:00:00Z".into()),
            start_timecode: Some("01:00:00:00".into()),
            start_timecode_rate: Some(Rational64::FPS_25),
            composer: Some("ArtCraft Audio Lab".into()),
            isrc: Some("US-S1Z-26-00001".into()),
            export_sidecar_xmp: true,
        };

        let packet = meta.to_xmp_packet();
        assert!(packet.contains("<?xpacket begin="));
        assert!(packet.contains("Cinematic Master 4K"));
        assert!(packet.contains("ArtCraft Team"));
        assert!(packet.contains("<xmpDM:composer>ArtCraft Audio Lab</xmpDM:composer>"));
        assert!(packet.contains("<xmpDM:ISRC>US-S1Z-26-00001</xmpDM:ISRC>"));
        assert!(packet.contains("ArtCraft OST"));
        assert!(packet.contains("<xmpDM:trackNumber>3</xmpDM:trackNumber>"));
        assert!(packet.contains("<xmpDM:releaseDate>2026</xmpDM:releaseDate>"));
        assert!(packet.contains("Soundtrack"));
        assert!(packet.contains("&copy; 2026 Storytold Organization") || packet.contains("© 2026 Storytold Organization"));
        assert!(packet.contains("FilmCraft Sequence 1"));
        assert!(packet.contains("xmpDM:timeFormat=\"25Timecode\" xmpDM:timeValue=\"01:00:00:00\""));
        assert!(packet.contains("<?xpacket end=\"w\"?>"));
    }

    #[test]
    fn xml_forbidden_characters_never_reach_the_packet() {
        assert_eq!(escape_xml("a\u{0}b\u{1b}c\u{FFFE}"), "a\u{FFFD}b\u{FFFD}c\u{FFFD}");
        assert_eq!(escape_xml("tab\tnew\nline <&>"), "tab\tnew\nline &lt;&amp;&gt;");
        let meta = XmpMetadata { title: Some("bad\u{0}title".into()), ..XmpMetadata::default() };
        assert!(!meta.to_xmp_packet().contains('\u{0}'));
    }

    #[test]
    fn drop_frame_start_timecode_names_its_format() {
        let meta =
            XmpMetadata { start_timecode: Some("01:00:00;00".into()), start_timecode_rate: Some(Rational64::FPS_29_97), ..XmpMetadata::default() };
        assert!(meta.to_xmp_packet().contains("2997DropTimecode"));

        let unreduced = XmpMetadata {
            start_timecode: Some("01:00:00:00".into()),
            start_timecode_rate: Some(Rational64 { num: 50, den: 2 }),
            ..XmpMetadata::default()
        };
        assert!(unreduced.to_xmp_packet().contains("25Timecode"));
    }
}
