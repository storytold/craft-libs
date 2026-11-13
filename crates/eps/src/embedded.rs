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
    while let Some(ee) = find_eexec(ps, from) {
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

/// Find an executable `eexec` operator, ignoring comments, literal strings and names.
fn find_eexec(ps: &[u8], from: usize) -> Option<usize> {
    let mut pos = from.min(ps.len());
    while pos < ps.len() {
        let c = ps[pos];
        if is_white(c) {
            pos += 1;
            continue;
        }
        if c == b'%' {
            let start = pos;
            while pos < ps.len() && !matches!(ps[pos], b'\n' | b'\r') {
                pos += 1;
            }
            let line = &ps[start..pos];
            let marker: Option<&[u8]> = if line.starts_with(b"%%BeginData:") {
                Some(b"%%EndData")
            } else if line.starts_with(b"%%BeginBinary:") {
                Some(b"%%EndBinary")
            } else if line.starts_with(b"%%BeginPreview") {
                Some(b"%%EndPreview")
            } else {
                None
            };
            if let Some(marker) = marker {
                let rest = ps.get(pos..)?;
                pos = rest.windows(marker.len()).position(|window| window == marker).map_or(ps.len(), |offset| pos + offset);
            }
            continue;
        }
        if c == b'(' {
            pos += 1;
            let mut depth = 1usize;
            while pos < ps.len() && depth > 0 {
                match ps[pos] {
                    b'\\' => pos = pos.saturating_add(2).min(ps.len()),
                    b'(' => {
                        depth = depth.saturating_add(1);
                        pos += 1;
                    }
                    b')' => {
                        depth -= 1;
                        pos += 1;
                    }
                    _ => pos += 1,
                }
            }
            continue;
        }
        if c == b'<' {
            match ps.get(pos + 1) {
                Some(b'<') => pos += 2,
                Some(b'~') => {
                    pos += 2;
                    pos = ps
                        .get(pos..)
                        .and_then(|rest| {
                            rest.windows(2).position(|window| window == b"~>").map(|offset| pos.saturating_add(offset).saturating_add(2))
                        })
                        .unwrap_or(ps.len());
                }
                _ => {
                    pos += 1;
                    while pos < ps.len() && ps[pos] != b'>' {
                        pos += 1;
                    }
                    pos = pos.saturating_add(1).min(ps.len());
                }
            }
            continue;
        }
        if c == b'/' {
            pos += 1;
            if ps.get(pos) == Some(&b'/') {
                pos += 1;
            }
            while pos < ps.len() && !is_white(ps[pos]) && !is_delimiter(ps[pos]) {
                pos += 1;
            }
            continue;
        }
        if is_delimiter(c) {
            pos += 1;
            continue;
        }
        let start = pos;
        while pos < ps.len() && !is_white(ps[pos]) && !is_delimiter(ps[pos]) {
            pos += 1;
        }
        if &ps[start..pos] == b"eexec" {
            return Some(start);
        }
    }
    None
}

fn is_white(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delimiter(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
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
    let e = rest[s..].iter().position(|c| is_white(*c) || is_delimiter(*c)).map_or(rest.len(), |q| s + q);
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

    #[test]
    fn eexec_in_comments_strings_names_and_font_names_is_not_an_operator() {
        let ps = b"%!PS-Adobe-3.0 EPSF-3.0\n%!PS-AdobeFont-1.0: Test\n%% eexec in a comment\n%%BeginBinary: 10\neexec junk\n%%EndBinary\n/FontName /eexecTest def\n(nested (ignored eexec)) pop\n<6565786563> pop\n<~eexec~> pop\n/eexec def\ncurrentfile eexec\n0000\ncleartomark\n";
        let fonts = embedded_font_programs(ps);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].name.as_deref(), Some("eexecTest"));
    }

    #[test]
    fn font_name_stops_at_postscript_delimiters_and_comments() {
        let ps = b"%!PS-AdobeFont-1.0: Test\n/FontName /Test%comment\n def\ncurrentfile eexec\n0000\ncleartomark\n";
        let fonts = embedded_font_programs(ps);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].name.as_deref(), Some("Test"));
    }

    #[test]
    fn font_names_stop_at_every_postscript_delimiter() {
        for delimiter in b"()<>[]{}/%" {
            let mut source = b"/FontName /Test".to_vec();
            source.push(*delimiter);
            source.extend_from_slice(b"ignored");
            assert_eq!(super::name_after(&source, b"/FontName").as_deref(), Some("Test"));
        }
    }

    #[test]
    fn literal_name_backslash_does_not_escape_a_comment_delimiter() {
        let ps = b"%!PS-AdobeFont-1.0: Test\n/FontName /Test def\n/unused\\%(\npop\ncurrentfile eexec\n0000\ncleartomark\n";
        let fonts = embedded_font_programs(ps);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].name.as_deref(), Some("Test"));
    }
}
