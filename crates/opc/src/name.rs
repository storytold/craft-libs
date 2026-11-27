//! Part names: `word/document.xml`, without the leading `/`.

/// The key a part name is looked up by: `\` as `/`, no leading `/`, ASCII lower case (part
/// names are case-insensitive, ECMA-376 Part 2 §6.2.2.3).
pub fn normalize(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

/// `xl/worksheets/sheet1.xml` → (`xl/worksheets`, `sheet1.xml`).
pub fn split_dir(part: &str) -> (&str, &str) {
    let p = part.trim_start_matches('/');
    match p.rfind('/') {
        Some(i) => (p.get(..i).unwrap_or(""), p.get(i + 1..).unwrap_or("")),
        None => ("", p),
    }
}

/// The folder of a part: `ppt/slides/slide1.xml` → `ppt/slides`.
pub fn dir_of(part: &str) -> &str {
    split_dir(part).0
}

/// The last segment of a part name.
pub fn file_of(part: &str) -> &str {
    part.rsplit('/').next().unwrap_or(part)
}

/// The relationships part of `part`: `xl/workbook.xml` → `xl/_rels/workbook.xml.rels`. The
/// package's own relationships are those of `""`: `_rels/.rels`.
pub fn rels_path(part: &str) -> String {
    let (dir, file) = split_dir(part);
    if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") }
}

/// `%hh` escapes decoded; a `%` not followed by two hex digits stays.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%'
            && let Some(h) = b
                .get(i + 1..i + 3)
                .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves a relationship target against the source part's folder into a part name. A
/// fragment is dropped, `\` reads as `/`, and `..` cannot climb out of the package. `%hh`
/// escapes are kept as written: [`Rels`](crate::Rels) decodes them when no part has the
/// escaped name.
pub fn resolve(dir: &str, target: &str) -> String {
    let target = target.split('#').next().unwrap_or("").replace('\\', "/");
    let mut segs: Vec<&str> = if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// The relative reference from part `from` to part `to`, as a relationship target:
/// `word/document.xml` → `word/charts/chart1.xml` is `charts/chart1.xml`.
pub fn relative(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = dir_of(from).split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let to_dir = to.get(..to.len().saturating_sub(1)).unwrap_or(&[]);
    let common = from_dir.iter().zip(to_dir).take_while(|(a, b)| a == b).count();
    let mut out: Vec<&str> = vec![".."; from_dir.len().saturating_sub(common)];
    out.extend(to.get(common..).unwrap_or(&[]));
    let s = out.join("/");
    // A relative reference whose first segment has a colon would read as a URI scheme.
    if s.split('/').next().is_some_and(|seg| seg.contains(':')) { format!("./{s}") } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_targets() {
        assert_eq!(resolve("word", "media/image1.png"), "word/media/image1.png");
        assert_eq!(resolve("word", "../customXml/item1.xml"), "customXml/item1.xml");
        assert_eq!(resolve("word", "/word/styles.xml"), "word/styles.xml");
        assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
        assert_eq!(resolve("word", "../../../x"), "x");
        assert_eq!(resolve("xl/worksheets", "../drawings/d1.xml"), "xl/drawings/d1.xml");
        assert_eq!(resolve("ppt/slides", "../media/a.png#frag"), "ppt/media/a.png");
        assert_eq!(resolve("ppt/slides", "..\\media\\image1.png"), "ppt/media/image1.png");
        assert_eq!(resolve("ppt/slides", "\\ppt\\media\\a.png"), "ppt/media/a.png");
        // An escaped `#` is part of the name, not a fragment.
        assert_eq!(percent_decode(&resolve("ppt/slides", "../media/a%23b.png")), "ppt/media/a#b.png");
    }

    #[test]
    fn percent_decoding_requires_two_hex_digits() {
        assert_eq!(percent_decode("a%+5b.png"), "a%+5b.png");
        assert_eq!(percent_decode("a%20b.png"), "a b.png");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%e2%82%ac"), "€");
    }

    #[test]
    fn splits_and_names_relationship_parts() {
        assert_eq!(split_dir("xl/workbook.xml"), ("xl", "workbook.xml"));
        assert_eq!(split_dir("x.xml"), ("", "x.xml"));
        assert_eq!(split_dir("/word/document.xml"), ("word", "document.xml"));
        assert_eq!(rels_path("ppt/presentation.xml"), "ppt/_rels/presentation.xml.rels");
        assert_eq!(rels_path("[Content_Types].xml"), "_rels/[Content_Types].xml.rels");
        assert_eq!(rels_path(""), "_rels/.rels");
        assert_eq!(normalize("\\PPT\\Slides/Slide1.xml"), "ppt/slides/slide1.xml");
    }

    #[test]
    fn relative_targets_resolve_back() {
        for (from, to, want) in [
            ("word/document.xml", "word/charts/chart1.xml", "charts/chart1.xml"),
            ("word/charts/chart1.xml", "word/embeddings/book.xlsx", "../embeddings/book.xlsx"),
            ("word/charts/chart1.xml", "word/charts/colors1.xml", "colors1.xml"),
            ("word/document.xml", "customXml/item1.xml", "../customXml/item1.xml"),
            ("word/document.xml", "word/a:b.bin", "./a:b.bin"),
            ("ppt/slides/slide1.xml", "ppt/media/image1.png", "../media/image1.png"),
            ("ppt/presentation.xml", "ppt/slides/slide1.xml", "slides/slide1.xml"),
            ("", "word/document.xml", "word/document.xml"),
        ] {
            let r = relative(from, to);
            assert_eq!(r, want);
            assert_eq!(resolve(dir_of(from), &r), to);
        }
    }
}
