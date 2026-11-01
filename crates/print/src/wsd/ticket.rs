//! The print ticket and the document description, built element by element in schema order.
//!
//! The ticket has a job part (name, user, copies) and a document part (media, orientation,
//! resolution, quality, sides). `MustHonor` marks a value the client insists on: a printer that
//! cannot do it must refuse the job instead of substituting. There is no colour setting in the
//! print schema (`MediaColor` is the colour of the paper): colour is chosen when the document is
//! produced, see [`super::encoder::ColorMode`].

use super::adapter::xml::esc;
use super::error::{Result, WsdError};

/// The largest copy count the client will ask for.
pub const MAX_COPIES: u32 = 9999;
const MAX_NAME: usize = 255;

/// Duplex setting (`Sides`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sides {
    OneSided,
    TwoSidedLongEdge,
    TwoSidedShortEdge,
}

impl Sides {
    /// The variant spelled `token` (as `token()` writes it).
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "OneSided" => Some(Self::OneSided),
            "TwoSidedLongEdge" => Some(Self::TwoSidedLongEdge),
            "TwoSidedShortEdge" => Some(Self::TwoSidedShortEdge),
            _ => None,
        }
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::OneSided => "OneSided",
            Self::TwoSidedLongEdge => "TwoSidedLongEdge",
            Self::TwoSidedShortEdge => "TwoSidedShortEdge",
        }
    }
}

/// Page orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
    ReversePortrait,
    ReverseLandscape,
}

impl Orientation {
    /// The variant spelled `token` (as `token()` writes it).
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "Portrait" => Some(Self::Portrait),
            "Landscape" => Some(Self::Landscape),
            "ReversePortrait" => Some(Self::ReversePortrait),
            "ReverseLandscape" => Some(Self::ReverseLandscape),
            _ => None,
        }
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::Portrait => "Portrait",
            Self::Landscape => "Landscape",
            Self::ReversePortrait => "ReversePortrait",
            Self::ReverseLandscape => "ReverseLandscape",
        }
    }
}

/// Print quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Draft,
    Normal,
    High,
    Photo,
}

impl Quality {
    /// The variant spelled `token` (as `token()` writes it).
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "Draft" => Some(Self::Draft),
            "Normal" => Some(Self::Normal),
            "High" => Some(Self::High),
            "Photo" => Some(Self::Photo),
            _ => None,
        }
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Normal => "Normal",
            Self::High => "High",
            Self::Photo => "Photo",
        }
    }
}

/// Document compression. Only `None` is produced by this client; the others exist so a printer's
/// list can be read and matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Deflate,
    Compress,
}

impl Compression {
    pub const fn token(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Gzip => "Gzip",
            Self::Deflate => "Deflate",
            Self::Compress => "Compress",
        }
    }
}

/// A name token as the schema's NMTOKEN-style values allow: letters, digits and `. _ - :`.
pub fn valid_token(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
}

/// A document format string (a MIME type, possibly with parameters), as the schema's pattern allows.
pub fn valid_format(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && s.len() <= MAX_NAME
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | ';' | '_' | '-' | '+' | '.' | '='))
}

/// How much of a ticket is written. Everything but `Full` exists for printers whose embedded server
/// chokes on a full ticket: it is never the default, and settings the ticket would have carried
/// are dropped (the job flow reports them as warnings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TicketShape {
    /// `JobDescription` (name and originating user), `JobProcessing` (copies) and `DocumentProcessing`
    /// for whatever is set. The normal shape.
    #[default]
    Full,
    /// `JobDescription` with both its required children (name and user) and nothing else: the
    /// smallest ticket the schema accepts.
    Description,
    /// Only `JobDescription/JobName`: no user name, no `JobProcessing`, no `DocumentProcessing`
    /// (also none in `SendDocument`).
    JobNameOnly,
    /// An empty `PrintTicket` element; the printer applies its defaults. The schema makes
    /// `JobDescription` optional, the service description's prose calls it mandatory, so this is the
    /// most permissive reading.
    Empty,
}

impl TicketShape {
    /// The shape spelled `token` (`full`, `job_name`, `empty`), as tools and the command line write it.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "full" => Some(Self::Full),
            "description" => Some(Self::Description),
            "job_name" => Some(Self::JobNameOnly),
            "empty" => Some(Self::Empty),
            _ => None,
        }
    }

    /// Whether the ticket carries `JobName` (every shape but [`Self::Empty`]).
    pub const fn carries_name(self) -> bool {
        !matches!(self, Self::Empty)
    }

    /// Whether the ticket carries `JobOriginatingUserName` ([`Self::Full`] and [`Self::Description`]).
    pub const fn carries_user(self) -> bool {
        matches!(self, Self::Full | Self::Description)
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Description => "description",
            Self::JobNameOnly => "job_name",
            Self::Empty => "empty",
        }
    }
}

/// Job-level and document-level settings of one print job.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PrintTicket {
    pub job_name: String,
    pub user_name: String,
    pub copies: Option<u32>,
    /// `MediaSizeName`, such as `iso_a4_210x297mm` or `na_letter_8.5x11in`.
    pub media_size: Option<String>,
    pub orientation: Option<Orientation>,
    /// Resolution in dots per inch, width and height.
    pub resolution: Option<(u32, u32)>,
    pub quality: Option<Quality>,
    pub sides: Option<Sides>,
    /// Write `MustHonor="true"` on every setting above.
    pub must_honor: bool,
    /// How much of the above is written; see [`TicketShape`].
    pub shape: TicketShape,
}

impl PrintTicket {
    /// Check every value; nothing invalid is ever written to the wire.
    pub fn validate(&self) -> Result<()> {
        let bad = |s: String| Err(WsdError::Invalid(s));
        if self.job_name.chars().count() > MAX_NAME || self.user_name.chars().count() > MAX_NAME {
            return bad("job or user name: at most 255 characters".into());
        }
        if let Some(c) = self.copies
            && !(1..=MAX_COPIES).contains(&c)
        {
            return bad(format!("copies {c}: use 1 to {MAX_COPIES}"));
        }
        if let Some(m) = &self.media_size
            && !valid_token(m)
        {
            return bad(format!("media size {m:?}: not a name token"));
        }
        if let Some((w, h)) = self.resolution
            && (w == 0 || h == 0 || w > 100_000 || h > 100_000)
        {
            return bad(format!("resolution {w}x{h}"));
        }
        Ok(())
    }

    /// The `<wprt:PrintTicket>` element (the body of CreatePrintJobRequest).
    pub fn to_xml(&self) -> Result<String> {
        self.validate()?;
        match self.shape {
            TicketShape::Full => {}
            TicketShape::Description => {
                return Ok(format!(
                    "<wprt:PrintTicket><wprt:JobDescription><wprt:JobName>{}</wprt:JobName><wprt:JobOriginatingUserName>{}</wprt:JobOriginatingUserName></wprt:JobDescription></wprt:PrintTicket>",
                    esc(&self.job_name),
                    esc(&self.user_name)
                ));
            }
            TicketShape::JobNameOnly => {
                return Ok(format!(
                    "<wprt:PrintTicket><wprt:JobDescription><wprt:JobName>{}</wprt:JobName></wprt:JobDescription></wprt:PrintTicket>",
                    esc(&self.job_name)
                ));
            }
            TicketShape::Empty => return Ok("<wprt:PrintTicket/>".to_owned()),
        }
        let mh = if self.must_honor { " wprt:MustHonor=\"true\"" } else { "" };
        let mut x = String::from("<wprt:PrintTicket>");
        x.push_str(&format!(
            "<wprt:JobDescription><wprt:JobName>{}</wprt:JobName><wprt:JobOriginatingUserName>{}</wprt:JobOriginatingUserName></wprt:JobDescription>",
            esc(&self.job_name),
            esc(&self.user_name)
        ));
        if let Some(c) = self.copies {
            x.push_str(&format!("<wprt:JobProcessing><wprt:Copies{mh}>{c}</wprt:Copies></wprt:JobProcessing>"));
        }
        x.push_str(&self.document_processing_xml());
        x.push_str("</wprt:PrintTicket>");
        Ok(x)
    }

    /// The `<wprt:DocumentProcessing>` element, or an empty string when no document setting is made.
    pub fn document_processing_xml(&self) -> String {
        if self.shape != TicketShape::Full {
            return String::new();
        }
        let mh = if self.must_honor { " wprt:MustHonor=\"true\"" } else { "" };
        let mut inner = String::new();
        // Schema order: MediaSizeName, MediaType, MediaColor, NumberUp, Orientation, Resolution, PrintQuality, Sides.
        if let Some(m) = &self.media_size {
            inner.push_str(&format!("<wprt:MediaSizeName{mh}>{}</wprt:MediaSizeName>", esc(m)));
        }
        if let Some(o) = self.orientation {
            inner.push_str(&format!("<wprt:Orientation{mh}>{}</wprt:Orientation>", o.token()));
        }
        if let Some((w, h)) = self.resolution {
            inner.push_str(&format!("<wprt:Resolution{mh}><wprt:Width>{w}</wprt:Width><wprt:Height>{h}</wprt:Height></wprt:Resolution>"));
        }
        if let Some(q) = self.quality {
            inner.push_str(&format!("<wprt:PrintQuality{mh}>{}</wprt:PrintQuality>", q.token()));
        }
        if let Some(s) = self.sides {
            inner.push_str(&format!("<wprt:Sides{mh}>{}</wprt:Sides>", s.token()));
        }
        if inner.is_empty() { String::new() } else { format!("<wprt:DocumentProcessing>{inner}</wprt:DocumentProcessing>") }
    }
}

/// `DocumentDescription`: which document this is and what it is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentDescription {
    /// 1-based, increasing within the job.
    pub id: u32,
    pub compression: Compression,
    /// The format, spelled as the printer lists it.
    pub format: String,
    pub name: Option<String>,
    pub must_honor_format: bool,
}

impl DocumentDescription {
    pub fn to_xml(&self) -> Result<String> {
        if self.id == 0 || self.id > i32::MAX as u32 {
            return Err(WsdError::Invalid(format!("document id {}", self.id)));
        }
        if !valid_format(&self.format) {
            return Err(WsdError::Invalid(format!("document format {:?}", self.format)));
        }
        if self.name.as_ref().is_some_and(|n| n.chars().count() > MAX_NAME) {
            return Err(WsdError::Invalid("document name: at most 255 characters".into()));
        }
        let mh = if self.must_honor_format { " wprt:MustHonor=\"true\"" } else { "" };
        let name = self.name.as_ref().map(|n| format!("<wprt:DocumentName>{}</wprt:DocumentName>", esc(n))).unwrap_or_default();
        Ok(format!(
            "<wprt:DocumentDescription><wprt:DocumentId>{}</wprt:DocumentId><wprt:Compression>{}</wprt:Compression><wprt:Format{mh}>{}</wprt:Format>{name}</wprt:DocumentDescription>",
            self.id,
            self.compression.token(),
            esc(&self.format)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsd::adapter::xml;
    use crate::wsd::ns;

    fn wrapped(inner: &str) -> String {
        format!("<r xmlns:wprt=\"{}\">{inner}</r>", ns::WPRT)
    }

    fn full() -> PrintTicket {
        PrintTicket {
            job_name: "a <b> & c".into(),
            user_name: "craft".into(),
            copies: Some(3),
            media_size: Some("iso_a4_210x297mm".into()),
            orientation: Some(Orientation::Landscape),
            resolution: Some((600, 300)),
            quality: Some(Quality::High),
            sides: Some(Sides::TwoSidedLongEdge),
            must_honor: false,
            shape: TicketShape::Full,
        }
    }

    #[test]
    fn minimal_shapes_write_only_what_they_name() {
        let job = PrintTicket { shape: TicketShape::JobNameOnly, ..full() };
        assert_eq!(
            job.to_xml().unwrap(),
            "<wprt:PrintTicket><wprt:JobDescription><wprt:JobName>a &lt;b&gt; &amp; c</wprt:JobName></wprt:JobDescription></wprt:PrintTicket>"
        );
        assert_eq!(job.document_processing_xml(), "");
        let desc = PrintTicket { shape: TicketShape::Description, ..full() };
        assert!(
            desc.to_xml().unwrap().ends_with("</wprt:JobOriginatingUserName></wprt:JobDescription></wprt:PrintTicket>")
                && !desc.to_xml().unwrap().contains("Copies")
        );
        let empty = PrintTicket { shape: TicketShape::Empty, ..full() };
        assert_eq!(empty.to_xml().unwrap(), "<wprt:PrintTicket/>");
        assert_eq!(empty.document_processing_xml(), "");
        // Still validated, and the default is the full ticket.
        assert!(PrintTicket { shape: TicketShape::Empty, copies: Some(0), ..full() }.to_xml().is_err());
        assert_eq!(TicketShape::default(), TicketShape::Full);
        assert!(full().to_xml().unwrap().contains("JobOriginatingUserName"));
        for s in [TicketShape::Full, TicketShape::Description, TicketShape::JobNameOnly, TicketShape::Empty] {
            assert_eq!(TicketShape::parse(s.token()), Some(s));
        }
        assert_eq!(TicketShape::parse("minimal"), None);
    }

    #[test]
    fn the_ticket_is_in_schema_order_and_escaped() {
        let x = wrapped(&full().to_xml().unwrap());
        let doc = xml::parse(&x).unwrap();
        let t = xml::find(doc.root_element(), ns::WPRT, "PrintTicket").unwrap();
        let top: Vec<_> = t.children().filter(|c| c.is_element()).map(|c| c.tag_name().name().to_owned()).collect();
        assert_eq!(top, ["JobDescription", "JobProcessing", "DocumentProcessing"]);
        let jd = xml::child(t, ns::WPRT, "JobDescription").unwrap();
        assert_eq!(xml::text(xml::child(jd, ns::WPRT, "JobName").unwrap()), "a <b> & c");
        let dp = xml::child(t, ns::WPRT, "DocumentProcessing").unwrap();
        let order: Vec<_> = dp.children().filter(|c| c.is_element()).map(|c| c.tag_name().name().to_owned()).collect();
        assert_eq!(order, ["MediaSizeName", "Orientation", "Resolution", "PrintQuality", "Sides"]);
        let res = xml::child(dp, ns::WPRT, "Resolution").unwrap();
        assert_eq!(xml::text(xml::child(res, ns::WPRT, "Width").unwrap()), "600");
        assert_eq!(xml::text(xml::child(res, ns::WPRT, "Height").unwrap()), "300");
        assert!(!x.contains("MustHonor"));
    }

    #[test]
    fn must_honor_is_a_qualified_attribute_on_each_setting() {
        let t = PrintTicket { must_honor: true, ..full() };
        let x = wrapped(&t.to_xml().unwrap());
        let doc = xml::parse(&x).unwrap();
        let marked = doc.descendants().filter(|n| n.attribute((ns::WPRT, "MustHonor")) == Some("true")).count();
        assert_eq!(marked, 6); // copies, media, orientation, resolution, quality, sides
    }

    #[test]
    fn a_minimal_ticket_has_no_optional_parts() {
        let t = PrintTicket { job_name: "j".into(), user_name: "u".into(), ..Default::default() };
        let x = t.to_xml().unwrap();
        assert!(!x.contains("JobProcessing") && !x.contains("DocumentProcessing"));
    }

    #[test]
    fn invalid_values_are_refused_before_anything_is_written() {
        let ok = full();
        for t in [
            PrintTicket { copies: Some(0), ..ok.clone() },
            PrintTicket { copies: Some(MAX_COPIES + 1), ..ok.clone() },
            PrintTicket { media_size: Some("a b".into()), ..ok.clone() },
            PrintTicket { media_size: Some("<x/>".into()), ..ok.clone() },
            PrintTicket { media_size: Some(String::new()), ..ok.clone() },
            PrintTicket { resolution: Some((0, 300)), ..ok.clone() },
            PrintTicket { resolution: Some((300, 1_000_000)), ..ok.clone() },
            PrintTicket { job_name: "x".repeat(256), ..ok.clone() },
        ] {
            assert!(matches!(t.to_xml(), Err(WsdError::Invalid(_))), "{t:?}");
        }
    }

    #[test]
    fn the_document_description_is_checked_and_ordered() {
        let d = DocumentDescription {
            id: 1,
            compression: Compression::None,
            format: "image/pwg-raster".into(),
            name: Some("n&m".into()),
            must_honor_format: false,
        };
        let x = wrapped(&d.to_xml().unwrap());
        let doc = xml::parse(&x).unwrap();
        let dd = xml::find(doc.root_element(), ns::WPRT, "DocumentDescription").unwrap();
        let order: Vec<_> = dd.children().filter(|c| c.is_element()).map(|c| c.tag_name().name().to_owned()).collect();
        assert_eq!(order, ["DocumentId", "Compression", "Format", "DocumentName"]);
        for bad in [
            DocumentDescription { id: 0, ..d.clone() },
            DocumentDescription { id: u32::MAX, ..d.clone() },
            DocumentDescription { format: "a b".into(), ..d.clone() },
            DocumentDescription { format: "<x>".into(), ..d.clone() },
            DocumentDescription { format: String::new(), ..d.clone() },
            DocumentDescription { format: "/x".into(), ..d.clone() },
            DocumentDescription { name: Some("n".repeat(300)), ..d.clone() },
        ] {
            assert!(bad.to_xml().is_err(), "{bad:?}");
        }
        assert!(valid_format("text/plain;charset=utf-8") && valid_format("application/vnd.hp-PCL"));
    }
}
