//! Charset decoding of `.aff` and `.dic` bytes (single-byte sets, UTF-8 with byte flags).

/// ISO 8859-1: every byte is the code point of the same value (the original VERO pt_BR uses it).
fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// ISO 8859-9 (Latin-5, Turkish): ISO 8859-1 with Ğ İ Ş ğ ı ş in place of Ð Ý Þ ð ý þ. The
/// encoding of older Turkish dictionaries (`SET ISO8859-9`).
fn decode_latin5(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0xd0 => 'Ğ',
            0xdd => 'İ',
            0xde => 'Ş',
            0xf0 => 'ğ',
            0xfd => 'ı',
            0xfe => 'ş',
            _ => char::from(b),
        })
        .collect()
}

/// ISO 8859-2 code points 0xA0..=0xFF (0x00..=0x9F map to themselves).
pub(super) const ISO_8859_2_HIGH: &str = "\u{a0}Ą˘Ł¤ĽŚ§¨ŠŞŤŹ\u{ad}ŽŻ°ą˛ł´ľśˇ¸šşťź˝žżŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢßŕáâăäĺćçčéęëěíîďđńňóôőö÷řůúűüýţ˙";

/// ISO 8859-7 (Greek) code points 0xA0..=0xFF; 0xAE, 0xD2 and 0xFF are unassigned and all read
/// as U+FFFD. From the
/// Unicode Consortium's mapping file `MAPPINGS/ISO8859/8859-7.TXT` (2003 edition, with € ₯ ͺ).
/// The encoding of the Greek dictionary (`SET ISO8859-7`).
pub(super) const ISO_8859_7_HIGH: &str =
    "\u{a0}‘’£€₯¦§¨©ͺ«¬\u{ad}\u{fffd}―°±²³΄΅Ά·ΈΉΊ»Ό½ΎΏΐΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡ\u{fffd}ΣΤΥΦΧΨΩΪΫάέήίΰαβγδεζηθικλμνξοπρςστυφχψωϊϋόύώ\u{fffd}";

/// ISO 8859-15 (Latin-9) code points 0xA0..=0xFF: ISO 8859-1 with € Š š Ž ž Œ œ Ÿ in eight
/// places. From the Unicode Consortium's mapping file `MAPPINGS/ISO8859/8859-15.TXT`. The
/// encoding of the Estonian dictionary (`SET ISO8859-15`).
pub(super) const ISO_8859_15_HIGH: &str =
    "\u{a0}¡¢£€¥Š§š©ª«¬\u{ad}®¯°±²³Žµ¶·ž¹º»ŒœŸ¿ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖ×ØÙÚÛÜÝÞßàáâãäåæçèéêëìíîïðñòóôõö÷øùúûüýþÿ";

/// Decode an ISO 8859 part whose bytes 0xA0..=0xFF are `high` (0x00..=0x9F map to themselves).
fn decode_iso(bytes: &[u8], high: &str) -> String {
    let high: Vec<char> = high.chars().collect();
    bytes.iter().map(|&b| if b < 0xa0 { char::from(b) } else { high.get(usize::from(b - 0xa0)).copied().unwrap_or('\u{fffd}') }).collect()
}

/// KOI8-R code points 0x80..=0xFF (0x00..=0x7F are ASCII): the encoding of older Russian
/// dictionaries.
pub(super) const KOI8_R_HIGH: &str =
    "─│┌┐└┘├┤┬┴┼▀▄█▌▐░▒▓⌠■∙√≈≤≥\u{a0}⌡°²·÷═║╒ё╓╔╕╖╗╘╙╚╛╜╝╞╟╠╡Ё╢╣╤╥╦╧╨╩╪╫╬©юабцдефгхийклмнопярстужвьызшэщчъЮАБЦДЕФГХИЙКЛМНОПЯРСТУЖВЬЫЗШЭЩЧЪ";

/// Windows-1251 code points 0x80..=0xFF (0x98 is unassigned): the other one Russian dictionaries
/// use.
pub(super) const CP1251_HIGH: &str = "ЂЃ‚ѓ„…†‡€‰Љ‹ЊЌЋЏђ‘’“”•–—\u{fffd}™љ›њќћџ\u{a0}ЎўЈ¤Ґ¦§Ё©Є«¬\u{ad}®Ї°±Ііґµ¶·ё№є»јЅѕїАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдежзийклмнопрстуфхцчшщъыьэюя";

/// Decode a single-byte encoding whose bytes 0x80..=0xFF are `high` (ASCII below).
pub(super) fn decode_high(bytes: &[u8], high: &str) -> String {
    let high: Vec<char> = high.chars().collect();
    bytes.iter().map(|&b| if b < 0x80 { char::from(b) } else { high.get(usize::from(b - 0x80)).copied().unwrap_or('\u{fffd}') }).collect()
}

/// Decode an `.aff`/`.dic` pair to UTF-8 from the encoding the `.aff`'s `SET` line names
/// (UTF-8 when there is none; ISO 8859-2 is the other one Polish dictionaries use, ISO 8859-1 the
/// one the original Portuguese VERO uses, ISO 8859-7 the Greek one, ISO 8859-15 the Estonian one,
/// ISO 8859-9 the one older Turkish dictionaries use, KOI8-R
/// and Windows-1251 the ones older Russian dictionaries use).
///
/// Under a UTF-8 `SET` with single-byte flags (no `FLAG` line, or `FLAG DEFAULT`), flags are bytes,
/// as in Hunspell: the Hungarian dictionary writes its affix flags and flag aliases as bytes above
/// 0x7F that aren't UTF-8. Those fields are read byte by byte (each byte becomes U+0000..U+00FF,
/// one flag per byte); the words, affixes and conditions around them are UTF-8. With any other
/// `FLAG` (`UTF-8`, `num`, `long`) flags are text and the whole file is UTF-8: `SFX 1234` keeps
/// its number (the Turkish dictionary's `FLAG num`).
pub fn decode(aff: &[u8], dic: &[u8]) -> Result<(String, String), String> {
    let (set, flag) = directives(aff);
    let set = set.unwrap_or_else(|| "UTF-8".into());
    match set.as_str() {
        "UTF-8" | "UTF8" => {
            if flag.as_deref().is_some_and(|f| f != "DEFAULT") {
                let text = |b: &[u8], what: &str| String::from_utf8(b.to_vec()).map_err(|e| format!("{what}: not UTF-8: {e}"));
                return Ok((text(aff, ".aff")?, text(dic, ".dic")?));
            }
            Ok((decode_utf8_aff(aff)?, decode_utf8_dic(dic)?))
        }
        "ISO8859-1" | "ISO-8859-1" => Ok((decode_latin1(aff), decode_latin1(dic))),
        "ISO8859-2" | "ISO-8859-2" => Ok((decode_iso(aff, ISO_8859_2_HIGH), decode_iso(dic, ISO_8859_2_HIGH))),
        "ISO8859-7" | "ISO-8859-7" => Ok((decode_iso(aff, ISO_8859_7_HIGH), decode_iso(dic, ISO_8859_7_HIGH))),
        "ISO8859-15" | "ISO-8859-15" => Ok((decode_iso(aff, ISO_8859_15_HIGH), decode_iso(dic, ISO_8859_15_HIGH))),
        "ISO8859-9" | "ISO-8859-9" => Ok((decode_latin5(aff), decode_latin5(dic))),
        "KOI8-R" | "KOI8R" => Ok((decode_high(aff, KOI8_R_HIGH), decode_high(dic, KOI8_R_HIGH))),
        "CP1251" | "WINDOWS-1251" | "MICROSOFT-CP1251" => Ok((decode_high(aff, CP1251_HIGH), decode_high(dic, CP1251_HIGH))),
        other => Err(format!("unsupported encoding {other}")),
    }
}

/// The first arguments of the `.aff`'s first `SET` and `FLAG` lines, in capitals, found in one pass
/// over the whole file: a `FLAG UTF-8` after the rules still decides how the `.dic` flags are read,
/// as in Hunspell. Only the first argument counts: `FLAG UTF-8 # Unicode flags` is UTF-8.
fn directives(aff: &[u8]) -> (Option<String>, Option<String>) {
    let (mut set, mut flag) = (None, None);
    for l in aff.split(|&b| b == b'\n') {
        if set.is_some() && flag.is_some() {
            break;
        }
        let l = l.strip_prefix("\u{feff}".as_bytes()).unwrap_or(l);
        let slot = match l.first() {
            Some(b'S') if set.is_none() => (&mut set, &b"SET"[..]),
            Some(b'F') if flag.is_none() => (&mut flag, &b"FLAG"[..]),
            _ => continue,
        };
        let arg = l
            .strip_prefix(slot.1)
            .filter(|rest| rest.first().is_some_and(|b| matches!(b, b' ' | b'\t')))
            .and_then(|rest| rest.split(|b| matches!(b, b' ' | b'\t' | b'\r')).find(|f| !f.is_empty()));
        if let Some(a) = arg {
            *slot.0 = Some(String::from_utf8_lossy(a).to_ascii_uppercase());
        }
    }
    (set, flag)
}

/// Directives whose one argument is a flag.
const FLAG_DIRECTIVES: &[&str] = &[
    "CIRCUMFIX",
    "COMPOUNDBEGIN",
    "COMPOUNDEND",
    "COMPOUNDFLAG",
    "COMPOUNDFORBIDFLAG",
    "COMPOUNDLAST",
    "COMPOUNDFIRST",
    "COMPOUNDMIDDLE",
    "COMPOUNDPERMITFLAG",
    "COMPOUNDROOT",
    "FORBIDDENWORD",
    "FORCEUCASE",
    "GENERATE",
    "HU_KOTOHANGZO",
    "KEEPCASE",
    "LEMMA_PRESENT",
    "NEEDAFFIX",
    "NOSUGGEST",
    "ONLYINCOMPOUND",
    "ONLYROOT",
    "PSEUDOROOT",
    "SUBSTANDARD",
    "WARN",
];

/// Directives that only describe the dictionary; a non-UTF-8 byte in them (the Hungarian
/// dictionary's `NAME` is ISO 8859-2) is replaced instead of failing the load.
const METADATA_DIRECTIVES: &[&str] = &["NAME", "HOME", "VERSION", "LANG", "AM"];

/// `text/flags`: the text as UTF-8 and the flags byte by byte (`\/` is a literal slash). `what`
/// names the line in an error.
fn text_and_flags(field: &[u8], what: impl Fn() -> String) -> Result<String, String> {
    let mut slash = None;
    let mut i = 0;
    while let Some(&b) = field.get(i) {
        if b == b'\\' && field.get(i + 1) == Some(&b'/') {
            i += 2;
            continue;
        }
        if b == b'/' {
            slash = Some(i);
            break;
        }
        i += 1;
    }
    let (text, flags) = match slash {
        Some(k) => (field.get(..k).unwrap_or_default(), field.get(k..).unwrap_or_default()),
        None => (field, &[][..]),
    };
    let mut out = std::str::from_utf8(text).map_err(|e| format!("{}: not UTF-8: {e}", what()))?.to_string();
    out.push_str(&decode_latin1(flags));
    Ok(out)
}

/// Split a line into fields on spaces and tabs, keeping the separators (so a line rebuilt from
/// its parts is the original).
fn fields(line: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_sep = line.first().is_some_and(|b| matches!(b, b' ' | b'\t'));
    for (i, b) in line.iter().enumerate() {
        let sep = matches!(b, b' ' | b'\t');
        if sep != in_sep {
            out.push(line.get(start..i).unwrap_or_default());
            start = i;
            in_sep = sep;
        }
    }
    out.push(line.get(start..).unwrap_or_default());
    out
}

/// A UTF-8 `.aff` with single-byte flags (see [`decode`]).
fn decode_utf8_aff(aff: &[u8]) -> Result<String, String> {
    let mut out = String::with_capacity(aff.len());
    for (n, line) in aff.split(|&b| b == b'\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if let Ok(s) = std::str::from_utf8(line)
            && s.is_ascii()
        {
            out.push_str(s);
            continue;
        }
        let parts = fields(line);
        // Field index among the non-separator parts.
        let words: Vec<usize> =
            parts.iter().enumerate().filter(|(_, p)| !p.first().is_some_and(|b| matches!(b, b' ' | b'\t'))).map(|(i, _)| i).collect();
        let dir = words
            .first()
            .and_then(|&i| parts.get(i))
            .map(|p| String::from_utf8_lossy(p).trim_start_matches('\u{feff}').to_string())
            .unwrap_or_default();
        let what = || format!(".aff line {}", n + 1);
        if dir.starts_with('#') || METADATA_DIRECTIVES.contains(&dir.as_str()) {
            out.push_str(&String::from_utf8_lossy(line));
            continue;
        }
        // A trailing `# comment` (the Hungarian `AF` lines number themselves) is never a word or
        // a flag; from there on the line is kept as text. It can only start after the directive's
        // own fields, since a field such as an affix condition may itself begin with `#`.
        let field_text = |k: usize| words.get(k).and_then(|&w| parts.get(w)).map(|p| String::from_utf8_lossy(p).into_owned());
        let first_free = comment_field(&dir, field_text);
        let comment_at = words.iter().skip(first_free).position(|&w| parts.get(w).is_some_and(|f| f.first() == Some(&b'#'))).map(|k| k + first_free);
        // The field index of the next non-separator part.
        let mut next_field = 0;
        for (i, p) in parts.iter().enumerate() {
            let field = (!p.first().is_some_and(|b| matches!(b, b' ' | b'\t'))).then(|| {
                next_field += 1;
                next_field - 1
            });
            let in_comment = comment_at.is_some_and(|c| words.get(c).is_some_and(|&w| i >= w));
            let decoded = match (dir.as_str(), field) {
                _ if in_comment => String::from_utf8_lossy(p).into_owned(),
                ("AF" | "COMPOUNDRULE" | "SYLLABLENUM", Some(1)) => decode_latin1(p),
                ("PFX" | "SFX", Some(3)) | ("CHECKCOMPOUNDPATTERN", Some(1 | 2)) => text_and_flags(p, what)?,
                // A field that names one flag keeps its first byte, as Hunspell's `decode_flag`
                // does: `SFX é` is the flag 0xC3, the first of the two flags `kert/é` carries.
                ("PFX" | "SFX", Some(1)) => decode_latin1(p.get(..1).unwrap_or_default()),
                (d, Some(1)) if FLAG_DIRECTIVES.contains(&d) => decode_latin1(p.get(..1).unwrap_or_default()),
                _ => std::str::from_utf8(p).map_err(|e| format!("{}: not UTF-8: {e}", what()))?.to_string(),
            };
            out.push_str(&decoded);
        }
    }
    Ok(out)
}

/// The first field of an `.aff` line where a trailing `# comment` may start: after the
/// directive's own fields. A table's count line (`REP 12`, `PFX A Y 3`) has fewer fields than its
/// rows (`REP a b`, `PFX A 0 x .`). `field(k)` is the line's `k`-th field.
pub(super) fn comment_field(dir: &str, field: impl Fn(usize) -> Option<String>) -> usize {
    let number = |k: usize| field(k).is_some_and(|f| f.parse::<usize>().is_ok());
    match dir {
        "PFX" | "SFX" if field(2).is_some_and(|f| f == "Y" || f == "N") && number(3) => 4,
        "PFX" | "SFX" => 5,
        "REP" | "ICONV" | "OCONV" | "PHONE" | "MAP" | "BREAK" | "AF" | "AM" | "COMPOUNDRULE" | "CHECKCOMPOUNDPATTERN"
            if number(1) && field(2).is_none_or(|f| f.starts_with('#')) =>
        {
            2
        }
        "REP" | "ICONV" | "OCONV" | "PHONE" => 3,
        "CHECKCOMPOUNDPATTERN" => 4,
        _ => 2,
    }
}

/// Where the `word/flags` entry of a `.dic` line ends, as Hunspell reads it: at a tab or at the
/// first morphological field (a space or tab, two characters and `:`, as in ` po:noun`), whichever
/// comes first, so an entry may hold spaces (`ad hoc`); with neither, at the end of the line
/// (`kot/B 12` has the flags `B 12`, as in Hunspell).
pub(crate) fn dic_entry_end(line: &[u8]) -> usize {
    let blank = |b: &u8| matches!(b, b' ' | b'\t');
    let morph = line.windows(4).position(|w| matches!(w, [a, b, c, b':'] if blank(a) && !blank(b) && !blank(c)));
    let tab = line.iter().position(|&b| b == b'\t');
    let end = match (morph, tab) {
        (Some(m), Some(t)) => m.min(t),
        (m, t) => match m.or(t) {
            Some(e) => e,
            None => line.len(),
        },
    };
    // Blanks before the morphological field or the line end aren't part of the entry.
    line.get(..end).map_or(end, |e| e.iter().rposition(|b| !blank(b)).map_or(0, |k| k + 1))
}

/// A UTF-8 `.dic` with single-byte flags: the word is UTF-8, the flags after `/` are bytes.
fn decode_utf8_dic(dic: &[u8]) -> Result<String, String> {
    let mut out = String::with_capacity(dic.len());
    for (n, line) in dic.split(|&b| b == b'\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if let Ok(s) = std::str::from_utf8(line)
            && s.split_once('/').is_none_or(|(_, flags)| flags.is_ascii())
        {
            out.push_str(s);
            continue;
        }
        let what = || format!(".dic line {}", n + 1);
        // The first field is `word/flags` (it may hold spaces, see [`dic_entry_end`]);
        // morphological fields after it are UTF-8 text.
        let end = dic_entry_end(line);
        out.push_str(&text_and_flags(line.get(..end).unwrap_or_default(), what)?);
        let rest = line.get(end..).unwrap_or_default();
        out.push_str(std::str::from_utf8(rest).map_err(|e| format!("{}: not UTF-8: {e}", what()))?);
    }
    Ok(out)
}
