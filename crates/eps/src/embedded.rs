//! Embedded Type 1 and CFF font programs in PostScript / EPS source.

/// A font program embedded in the PostScript source of an EPS file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedFontProgram {
    /// The PostScript `/FontName` when one is present in the source. CFF `StartData` blocks store
    /// their name inside the CFF Name INDEX, which the font parser reads from `program`.
    pub name: Option<String>,
    /// The bytes of the Type 1 program or CFF font set.
    pub program: Vec<u8>,
    /// Whether `program` is a CFF font set rather than a Type 1 program.
    pub cff: bool,
}

/// Embedded font programs in a PostScript section, in source order within each format.
///
/// Type 1 programs are recognized from their `eexec` and `cleartomark` delimiters and are named
/// by `/FontName`. CFF `StartData` blocks are returned without a name because the name is inside
/// the binary CFF data. Malformed or truncated blocks are skipped, as in the original EPS reader.
pub fn embedded_font_programs(ps: &[u8]) -> Vec<EmbeddedFontProgram> {
    let mut out = Vec::new();
    let mut heads = positions(ps, b"%!PS-AdobeFont");
    heads.extend(positions(ps, b"%!FontType1"));
    heads.sort_unstable();
    let names = positions(ps, b"/FontName");
    let mut from = 0;
    let mut used = 0;
    while let Some(ee) = find_from(ps, b"eexec", from) {
        from = ee + 5;
        let last_before = |v: &[usize]| v[..v.partition_point(|&p| p < ee)].last().copied().filter(|&p| p >= used);
        // The fallback look-back stops at the previous font's end, so it cannot pick up its header.
        let start = last_before(&heads).or_else(|| last_before(&names).map(|p| p.saturating_sub(512).max(used)));
        let Some(start) = start else { continue };
        let end = find_from(ps, b"cleartomark", ee).map_or(ps.len(), |p| p + 11);
        if let Some(name) = name_after(&ps[start..ee], b"/FontName") {
            out.push(EmbeddedFontProgram { name: Some(name), program: ps[start..end].to_vec(), cff: false });
        }
        from = end;
        used = end;
    }

    let mut from = 0;
    while let Some(sd) = find_from(ps, b"StartData", from) {
        from = sd + 9;
        let head = String::from_utf8_lossy(&ps[sd.saturating_sub(64)..sd]).into_owned();
        let Some(len) = head.split_whitespace().last().and_then(|t| t.parse::<usize>().ok()) else { continue };
        let s = sd + 10;
        let Some(data) = ps.get(s..s.saturating_add(len)) else { continue };
        out.push(EmbeddedFontProgram { name: None, program: data.to_vec(), cff: true });
        from = s + len;
    }
    out
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn positions(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(p) = find_from(hay, needle, from) {
        out.push(p);
        from = p + needle.len();
    }
    out
}

fn name_after(b: &[u8], key: &[u8]) -> Option<String> {
    let p = find_from(b, key, 0)? + key.len();
    let rest = &b[p..];
    let s = rest.iter().position(|c| *c == b'/')? + 1;
    let e = rest[s..].iter().position(|c| c.is_ascii_whitespace() || matches!(c, b'/' | b'[' | b'(' | b'{')).map_or(rest.len(), |q| s + q);
    (e > s).then(|| String::from_utf8_lossy(&rest[s..e]).into_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::embedded_font_programs;

    #[test]
    fn finds_type1_font_programs_by_name() {
        let ps = b"%!PS-Adobe-3.0 EPSF-3.0\n%!PS-AdobeFont-1.0: Test\n/FontName /Test def\ncurrentfile eexec\n0000\ncleartomark\n";
        let fonts = embedded_font_programs(ps);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].name.as_deref(), Some("Test"));
        assert!(!fonts[0].cff);
        assert!(String::from_utf8_lossy(&fonts[0].program).contains("cleartomark"));
    }

    #[test]
    fn finds_cff_startdata_without_interpreting_its_name_index() {
        let ps = b"%!PS-Adobe-3.0 EPSF-3.0\n10 StartData\n0123456789\n";
        let fonts = embedded_font_programs(ps);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].name, None);
        assert!(fonts[0].cff);
        assert_eq!(fonts[0].program, b"0123456789");
    }

    #[test]
    fn ignores_truncated_startdata() {
        let fonts = embedded_font_programs(b"%!PS-Adobe-3.0 EPSF-3.0\n100 StartData\nshort");
        assert!(fonts.is_empty());
    }

    #[test]
    fn many_headerless_eexec_words_do_not_hide_later_font() {
        let mut ps = b"%!PS-Adobe-3.0 EPSF-3.0\n".to_vec();
        ps.extend_from_slice(b"eexec ".repeat(20_000).as_slice());
        ps.extend_from_slice(b"\n%!PS-AdobeFont-1.0: Real\n/FontName /Real def\ncurrentfile eexec\n0000\ncleartomark\n");
        let fonts = embedded_font_programs(&ps);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].name.as_deref(), Some("Real"));
    }

    #[test]
    fn headerless_font_does_not_reuse_previous_font_header() {
        let first = b"%!PS-AdobeFont-1.0: A\n/FontName /A def\ncurrentfile eexec\n0000\ncleartomark\n";
        let second = b"/FontName /B def\ncurrentfile eexec\n0000\ncleartomark\n";
        let mut ps = b"%!PS-Adobe-3.0 EPSF-3.0\n".to_vec();
        ps.extend_from_slice(first);
        ps.extend_from_slice(second);
        let fonts = embedded_font_programs(&ps);
        assert_eq!(fonts.len(), 2);
        let b = fonts.iter().find(|font| font.name.as_deref() == Some("B")).unwrap();
        assert!(!String::from_utf8_lossy(&b.program).contains("AdobeFont"));
    }
}
