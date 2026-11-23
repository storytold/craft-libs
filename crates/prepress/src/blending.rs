//! Whether a page blends in ink, read from the file without rendering it.
//!
//! A page is a transparency group, and its blending colour space is the one its `/Group`
//! names in `/CS` (ISO 32000-2 §11.4.7, §11.6.6). `DeviceCMYK`, `Separation` and
//! `DeviceN` are subtractive (§11.3.4); an `ICCBased` space counts by its component count
//! (§8.6.5.5), and `Indexed` by its base (§8.6.6.3). A page that names no space inherits the
//! output device's (§11.4.7). ISO 32000-2 Annex P, which is informative, lets that device
//! space be the output intent's; [`IntentUse`] says whether to take it.
//!
//! A renderer that composites in RGB gives other colours wherever a blend mode meets ink on
//! such a page, so a print or proof workflow can use this to say so, or to send the page
//! through `plates`.

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{Array, Dict, Name, Object, Stream};
use craft_pdf::syntax::page::Resources;

/// Named and array colour spaces followed before giving up, so a space that names itself
/// ends.
const MAX_DEPTH: usize = 8;

/// When a page that names no blending colour space takes the output intent's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IntentUse {
    /// Never: the page blends in the output device's own space, taken as RGB (§11.4.7).
    Never,
    /// Only when the output intent's profile has four or more components, so an RGB or gray
    /// intent never moves a page.
    #[default]
    IfSubtractive,
    /// Whenever the document has an output intent with a profile.
    Always,
}

/// Where a page's blending colour space came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendingFrom {
    /// The page's `/Group` named it in `/CS`.
    PageGroup,
    /// The page named none, and the output device's space applies.
    Device,
    /// The page named none, and the output intent's profile supplied it.
    OutputIntent,
}

/// A page's blending colour space: whether it is subtractive, and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageBlending {
    /// The page blends in ink: its blending colour space is subtractive.
    pub in_ink: bool,
    pub from: BlendingFrom,
}

/// The blending colour space of page `index` of `pdf`, or `None` when there is no such page.
pub fn page_blending(pdf: &Pdf, index: usize, intent: IntentUse) -> Option<PageBlending> {
    let page = pdf.pages().get(index)?;
    // A null entry is an absent one (§7.3.9).
    let declared = page.raw().get::<Dict<'_>>(b"Group").and_then(|g| g.get::<Object<'_>>(b"CS")).filter(|cs| !matches!(cs, Object::Null(_)));
    if let Some(cs) = declared {
        return Some(PageBlending { in_ink: subtractive(&cs, page.resources(), 0), from: BlendingFrom::PageGroup });
    }
    let device = PageBlending { in_ink: false, from: BlendingFrom::Device };
    let Some(components) = intent_components(pdf) else { return Some(device) };
    let in_ink = components >= 4;
    Some(match intent {
        IntentUse::Always => PageBlending { in_ink, from: BlendingFrom::OutputIntent },
        IntentUse::IfSubtractive if in_ink => PageBlending { in_ink, from: BlendingFrom::OutputIntent },
        // An RGB or gray intent decided nothing here: the device's space is the answer.
        IntentUse::IfSubtractive | IntentUse::Never => device,
    })
}

/// [`page_blending`] for every page of `pdf`, in page order.
pub fn pages_blending(pdf: &Pdf, intent: IntentUse) -> Vec<PageBlending> {
    (0..pdf.pages().len()).filter_map(|i| page_blending(pdf, i, intent)).collect()
}

/// Whether colour space `cs` is subtractive. An unreadable space counts as additive, which is
/// what an RGB renderer does with it anyway.
fn subtractive(cs: &Object<'_>, res: &Resources<'_>, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    match cs {
        // The short names are the inline-image forms; a loosely written group may use them.
        Object::Name(name) => match name.as_ref() {
            b"DeviceCMYK" | b"CMYK" => true,
            b"DeviceRGB" | b"RGB" | b"DeviceGray" | b"G" | b"Pattern" => false,
            other => res.get_color_space(&Name::new_unescaped(other)).is_some_and(|named| match &named {
                Object::Name(alias) if alias.as_ref() == other => false,
                _ => subtractive(&named, res, depth + 1),
            }),
        },
        Object::Array(array) => array_subtractive(array, res, depth),
        _ => false,
    }
}

fn array_subtractive(array: &Array<'_>, res: &Resources<'_>, depth: usize) -> bool {
    let mut items = array.iter::<Object<'_>>();
    let Some(Object::Name(family)) = items.next() else { return false };
    match family.as_ref() {
        b"DeviceCMYK" | b"CMYK" | b"Separation" | b"DeviceN" => true,
        b"ICCBased" => items.next().is_some_and(|profile| match profile {
            Object::Stream(stream) => icc_subtractive(&stream, res, depth),
            _ => false,
        }),
        b"Indexed" | b"I" => items.next().is_some_and(|base| subtractive(&base, res, depth + 1)),
        _ => false,
    }
}

/// An `ICCBased` space by its `/N`; without one, by its `/Alternate`.
fn icc_subtractive(stream: &Stream<'_>, res: &Resources<'_>, depth: usize) -> bool {
    match stream.dict().get::<i32>(b"N") {
        Some(n) => n >= 4,
        None => stream.dict().get::<Object<'_>>(b"Alternate").is_some_and(|alt| subtractive(&alt, res, depth + 1)),
    }
}

/// The component count of the first output intent that has a destination profile.
fn intent_components(pdf: &Pdf) -> Option<i32> {
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id())?;
    let intents = catalog.get::<Array<'_>>(b"OutputIntents")?;
    intents.iter::<Dict<'_>>().find_map(|oi| oi.get::<Stream<'_>>(b"DestOutputProfile")?.dict().get::<i32>(b"N"))
}

#[cfg(test)]
mod tests {
    use super::{BlendingFrom, IntentUse, PageBlending, page_blending, pages_blending};
    use craft_pdf::syntax::Pdf;

    /// One page, with `page` added to its dictionary, `resources` as its resources and
    /// `catalog` added to the catalog; `extra` objects are numbered from 4.
    fn pdf(page: &str, resources: &str, catalog: &str, extra: &[&str]) -> Pdf {
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << {resources} >> {page} >>"),
        ];
        objects.extend(extra.iter().map(|o| (*o).to_string()));
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        Pdf::new(out).expect("fixture parses")
    }

    fn group(cs: &str) -> String {
        format!("/Group << /S /Transparency /CS {cs} >>")
    }

    const PROFILE_CMYK: &str = "<< /N 4 /Length 0 >>\nstream\n\nendstream";
    const PROFILE_RGB: &str = "<< /N 3 /Length 0 >>\nstream\n\nendstream";
    const INTENT: &str = "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /DestOutputProfile 4 0 R >>]";

    fn blending(doc: &Pdf, intent: IntentUse) -> PageBlending {
        page_blending(doc, 0, intent).expect("page 0 exists")
    }

    #[test]
    fn subtractive_page_groups_blend_in_ink_and_additive_ones_do_not() {
        let ink = [
            group("/DeviceCMYK"),
            group("[/Separation /Gold /DeviceCMYK 4 0 R]"),
            group("[/DeviceN [/Gold /Silver] /DeviceCMYK 4 0 R]"),
            group("[/ICCBased 4 0 R]"),
            group("[/Indexed /DeviceCMYK 1 <0000000000000000>]"),
            group("/Press"),
        ];
        for page in &ink {
            let doc = pdf(page, "/ColorSpace << /Press [/ICCBased 4 0 R] >>", "", &[PROFILE_CMYK]);
            assert_eq!(blending(&doc, IntentUse::Never), PageBlending { in_ink: true, from: BlendingFrom::PageGroup }, "{page}");
        }
        let screen = [group("/DeviceRGB"), group("[/ICCBased 4 0 R]"), group("[/Indexed /DeviceRGB 1 <000000000000>]"), group("/Screen")];
        for page in &screen {
            let doc = pdf(page, "/ColorSpace << /Screen [/ICCBased 4 0 R] >>", "", &[PROFILE_RGB]);
            assert_eq!(blending(&doc, IntentUse::Never), PageBlending { in_ink: false, from: BlendingFrom::PageGroup }, "{page}");
        }
    }

    #[test]
    fn an_icc_space_without_n_counts_by_its_alternate() {
        let doc = pdf(&group("[/ICCBased 4 0 R]"), "", "", &["<< /Alternate /DeviceCMYK /Length 0 >>\nstream\n\nendstream"]);
        assert!(blending(&doc, IntentUse::Never).in_ink);
    }

    #[test]
    fn a_declared_page_group_is_not_overruled_by_the_output_intent() {
        let doc = pdf(&group("/DeviceRGB"), "", INTENT, &[PROFILE_CMYK]);
        assert_eq!(blending(&doc, IntentUse::Always), PageBlending { in_ink: false, from: BlendingFrom::PageGroup });
    }

    #[test]
    fn an_undeclared_page_takes_a_cmyk_output_intent_only_when_asked() {
        let doc = pdf("", "", INTENT, &[PROFILE_CMYK]);
        assert_eq!(blending(&doc, IntentUse::Never), PageBlending { in_ink: false, from: BlendingFrom::Device });
        for intent in [IntentUse::IfSubtractive, IntentUse::Always] {
            assert_eq!(blending(&doc, intent), PageBlending { in_ink: true, from: BlendingFrom::OutputIntent }, "{intent:?}");
        }
    }

    #[test]
    fn a_null_page_group_space_is_no_space() {
        let doc = pdf(&group("null"), "", INTENT, &[PROFILE_CMYK]);
        assert_eq!(blending(&doc, IntentUse::IfSubtractive), PageBlending { in_ink: true, from: BlendingFrom::OutputIntent });
    }

    #[test]
    fn an_rgb_output_intent_is_credited_only_when_always_taken() {
        let doc = pdf("", "", INTENT, &[PROFILE_RGB]);
        assert_eq!(blending(&doc, IntentUse::IfSubtractive), PageBlending { in_ink: false, from: BlendingFrom::Device });
        assert_eq!(blending(&doc, IntentUse::Always), PageBlending { in_ink: false, from: BlendingFrom::OutputIntent });
    }

    #[test]
    fn a_colour_space_that_names_itself_ends() {
        let doc = pdf(&group("/Loop"), "/ColorSpace << /Loop /Loop /Ring [/Indexed /Ring 1 <00>] >>", "", &[]);
        assert!(!blending(&doc, IntentUse::Never).in_ink);
        let doc = pdf(&group("/Ring"), "/ColorSpace << /Ring [/Indexed /Ring 1 <00>] >>", "", &[]);
        assert!(!blending(&doc, IntentUse::Never).in_ink);
    }

    #[test]
    fn every_page_is_answered_and_a_missing_page_is_none() {
        let doc = pdf(&group("/DeviceCMYK"), "", "", &[]);
        assert_eq!(pages_blending(&doc, IntentUse::default()).len(), 1);
        assert_eq!(page_blending(&doc, 1, IntentUse::default()), None);
    }
}
