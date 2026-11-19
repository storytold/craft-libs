//! Character references (`&amp;`, `&#233;`, `&#xE9;`) and escaping.

/// The Latin-1 names in code point order, from U+00A0 (`nbsp`) to U+00FF (`yuml`).
const LATIN1: [&str; 96] = [
    "nbsp", "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf", "laquo", "not", "shy", "reg", "macr", "deg",
    "plusmn", "sup2", "sup3", "acute", "micro", "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34", "iquest", "Agrave",
    "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute", "Ecirc", "Euml", "Igrave", "Iacute", "Icirc", "Iuml", "ETH",
    "Ntilde", "Ograve", "Oacute", "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute", "Ucirc", "Uuml", "Yacute", "THORN", "szlig",
    "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil", "egrave", "eacute", "ecirc", "euml", "igrave", "iacute", "icirc",
    "iuml", "eth", "ntilde", "ograve", "oacute", "ocirc", "otilde", "ouml", "divide", "oslash", "ugrave", "uacute", "ucirc", "uuml", "yacute",
    "thorn", "yuml",
];

/// Greek capitals from U+0391; U+03A2 is unassigned.
const GREEK_UPPER: [&str; 25] = [
    "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta", "Iota", "Kappa", "Lambda", "Mu", "Nu", "Xi", "Omicron", "Pi", "Rho", "",
    "Sigma", "Tau", "Upsilon", "Phi", "Chi", "Psi", "Omega",
];

/// Greek small letters from U+03B1.
const GREEK_LOWER: [&str; 25] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa", "lambda", "mu", "nu", "xi", "omicron", "pi", "rho",
    "sigmaf", "sigma", "tau", "upsilon", "phi", "chi", "psi", "omega",
];

/// What a numeric reference to 0x80–0x9F means (HTML Living Standard §13.2.5.80: the
/// Windows-1252 characters, by long tradition).
const C1: [u32; 32] = [
    0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019,
    0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
];

/// Longest entity name looked at.
const MAX_NAME: usize = 32;

/// The legacy names HTML also decodes without a `;`, sorted by bytes.
const BARE_NAMES: &[&str] = &[
    "AElig", "AMP", "Aacute", "Acirc", "Agrave", "Aring", "Atilde", "Auml", "COPY", "Ccedil", "Eacute", "Ecirc", "Egrave", "Euml", "GT", "Iacute",
    "Icirc", "Igrave", "Iuml", "LT", "Ntilde", "Oacute", "Ocirc", "Ograve", "Oslash", "Otilde", "Ouml", "QUOT", "REG", "Uacute", "Ucirc", "Ugrave",
    "Uuml", "Yacute", "aacute", "acirc", "aelig", "agrave", "amp", "aring", "atilde", "auml", "ccedil", "cent", "copy", "curren", "deg", "divide",
    "eacute", "ecirc", "egrave", "euml", "frac12", "frac14", "frac34", "gt", "iacute", "icirc", "iexcl", "igrave", "iquest", "iuml", "laquo", "lt",
    "micro", "middot", "nbsp", "ntilde", "oacute", "ocirc", "ograve", "oslash", "otilde", "ouml", "para", "plusmn", "pound", "quot", "raquo", "reg",
    "sect", "shy", "sup1", "sup2", "sup3", "szlig", "times", "uacute", "ucirc", "ugrave", "uuml", "yacute", "yen", "yuml",
];

/// Text as HTML, for content and for attribute values in either kind of quote: `&`, `<`, `>`,
/// `"` and `'` become character references.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len().saturating_add(text.len() / 8));
    push_escaped(&mut out, text, true);
    out
}

/// [`escape`] that leaves `'` as it is: for content and for attribute values in double quotes.
pub fn escape_dq(text: &str) -> String {
    let mut out = String::with_capacity(text.len().saturating_add(text.len() / 8));
    push_escaped(&mut out, text, false);
    out
}

fn push_escaped(out: &mut String, text: &str, apos: bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' if apos => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

/// The character an entity body names (`amp`, `#38`, `#x26`: what stands between `&` and `;`).
pub fn decode_entity(body: &str) -> Option<char> {
    match body.strip_prefix('#') {
        Some(_) => reference(body, false).filter(|(_, used)| *used == body.len()).map(|(c, _)| c),
        None => named(body),
    }
}

/// Character references in text decoded: named (the HTML 4 set and more), decimal and hex.
/// Anything that is no reference stays as it is. A legacy name decodes without its `;`
/// (`&copy 2024`), as in browsers.
pub fn unescape(text: &str) -> String {
    unescape_in(text, false)
}

/// [`unescape`] for an attribute value: a legacy name without its `;` stays text when a letter,
/// a digit or `=` follows, so `?a=1&copy=2` is a query string and not a copyright sign.
pub fn unescape_attr(text: &str) -> String {
    unescape_in(text, true)
}

fn unescape_in(text: &str, attr: bool) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 1..).unwrap_or("");
        match reference(after, attr) {
            Some((c, used)) => {
                out.push(c);
                rest = after.get(used..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character a reference names and the bytes it takes, given the text after its `&`
/// (`attr`: inside an attribute value, see [`unescape_attr`]). At most [`MAX_NAME`] bytes are
/// looked at for a name, so a text of stray `&`s is read in linear time.
pub fn reference(s: &str, attr: bool) -> Option<(char, usize)> {
    if let Some(num) = s.strip_prefix('#') {
        let (digits, radix, skip) = match num.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16, 2),
            None => (num, 10, 1),
        };
        let len = digits.bytes().take_while(|b| char::from(*b).is_digit(radix)).count();
        if len == 0 {
            return None;
        }
        // Too many digits is not a character; neither is 0, a surrogate, or anything past U+10FFFF.
        let mut code: u32 = 0;
        for d in digits.bytes().take(len) {
            let d = char::from(d).to_digit(radix).unwrap_or(0);
            code = code.checked_mul(radix).and_then(|c| c.checked_add(d)).unwrap_or(u32::MAX);
        }
        let semicolon = usize::from(digits.as_bytes().get(len) == Some(&b';'));
        return Some((numeric(code), skip + len + semicolon));
    }
    let len = s.bytes().take(MAX_NAME).take_while(u8::is_ascii_alphanumeric).count();
    if s.as_bytes().get(len) == Some(&b';')
        && let Some(c) = s.get(..len).and_then(named)
    {
        return Some((c, len + 1));
    }
    // Without its `;`: the longest legacy name (2 to 6 letters) the text starts with.
    let len = (2..=len.min(6)).rev().find(|&l| s.get(..l).is_some_and(|p| BARE_NAMES.binary_search(&p).is_ok()))?;
    if attr && s.as_bytes().get(len).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'=') {
        return None;
    }
    s.get(..len).and_then(named).map(|c| (c, len))
}

fn numeric(code: u32) -> char {
    let code = match code {
        0x80..=0x9F => C1.get((code - 0x80) as usize).copied().unwrap_or(code),
        c => c,
    };
    match char::from_u32(code) {
        Some('\0') | None => char::REPLACEMENT_CHARACTER,
        Some(c) => c,
    }
}

fn named(name: &str) -> Option<char> {
    if name.is_empty() {
        return None;
    }
    let c = match name {
        "amp" | "AMP" => '&',
        "lt" | "LT" => '<',
        "gt" | "GT" => '>',
        "quot" | "QUOT" => '"',
        "apos" | "APOS" => '\'',
        "nbsp" | "NBSP" => '\u{00A0}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "shy" => '\u{00AD}',
        "copy" | "COPY" => '©',
        "reg" | "REG" => '®',
        "trade" | "TRADE" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "minus" => '−',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "deg" => '°',
        "plusmn" => '±',
        "times" => '×',
        "divide" => '÷',
        "frac12" => '½',
        "frac14" => '¼',
        "frac34" => '¾',
        "frac13" => '⅓',
        "frac23" => '⅔',
        "frac18" => '⅛',
        "frac38" => '⅜',
        "frac58" => '⅝',
        "frac78" => '⅞',
        "sup1" => '¹',
        "sup2" => '²',
        "sup3" => '³',
        "micro" => 'µ',
        "permil" => '‰',
        "prime" => '′',
        "Prime" => '″',
        "oline" => '‾',
        "frasl" => '⁄',
        "sect" => '§',
        "para" => '¶',
        "euro" | "EURO" => '€',
        "pound" | "POUND" => '£',
        "yen" | "YEN" => '¥',
        "cent" | "CENT" => '¢',
        "curren" => '¤',
        "dagger" => '†',
        "Dagger" => '‡',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "harr" => '↔',
        "lArr" => '⇐',
        "rArr" => '⇒',
        "uArr" => '⇑',
        "dArr" => '⇓',
        "hArr" => '⇔',
        "crarr" => '↵',
        "forall" => '∀',
        "part" => '∂',
        "exist" => '∃',
        "empty" => '∅',
        "nabla" => '∇',
        "isin" => '∈',
        "notin" => '∉',
        "ni" => '∋',
        "prod" => '∏',
        "sum" => '∑',
        "radic" => '√',
        "prop" => '∝',
        "ang" => '∠',
        "and" => '∧',
        "or" => '∨',
        "cap" => '∩',
        "cup" => '∪',
        "int" => '∫',
        "there4" => '∴',
        "sim" => '∼',
        "cong" => '≅',
        "asymp" | "approx" => '≈',
        "ne" => '≠',
        "equiv" => '≡',
        "le" => '≤',
        "ge" => '≥',
        "sub" => '⊂',
        "sup" => '⊃',
        "nsub" => '⊄',
        "sube" => '⊆',
        "supe" => '⊇',
        "oplus" => '⊕',
        "otimes" => '⊗',
        "perp" => '⊥',
        "sdot" => '⋅',
        "infin" => '∞',
        "loz" => '◊',
        "spades" => '♠',
        "clubs" => '♣',
        "hearts" => '♥',
        "diams" => '♦',
        "check" => '✓',
        "iexcl" => '¡',
        "iquest" => '¿',
        "szlig" => 'ß',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "eacute" => 'é',
        "egrave" => 'è',
        "ecirc" => 'ê',
        "euml" => 'ë',
        "Eacute" => 'É',
        "Egrave" => 'È',
        "Ecirc" => 'Ê',
        "Euml" => 'Ë',
        "aacute" => 'á',
        "agrave" => 'à',
        "acirc" => 'â',
        "atilde" => 'ã',
        "Agrave" => 'À',
        "Aacute" => 'Á',
        "Acirc" => 'Â',
        "Atilde" => 'Ã',
        "iacute" => 'í',
        "igrave" => 'ì',
        "icirc" => 'î',
        "iuml" => 'ï',
        "Iacute" => 'Í',
        "Igrave" => 'Ì',
        "Icirc" => 'Î',
        "Iuml" => 'Ï',
        "oacute" => 'ó',
        "ograve" => 'ò',
        "ocirc" => 'ô',
        "otilde" => 'õ',
        "Oacute" => 'Ó',
        "Ograve" => 'Ò',
        "Ocirc" => 'Ô',
        "Otilde" => 'Õ',
        "uacute" => 'ú',
        "ugrave" => 'ù',
        "ucirc" => 'û',
        "Uacute" => 'Ú',
        "Ugrave" => 'Ù',
        "Ucirc" => 'Û',
        "yacute" => 'ý',
        "yuml" => 'ÿ',
        "Yacute" => 'Ý',
        "ntilde" => 'ñ',
        "Ntilde" => 'Ñ',
        "ccedil" => 'ç',
        "Ccedil" => 'Ç',
        "aring" => 'å',
        "Aring" => 'Å',
        "oslash" => 'ø',
        "Oslash" => 'Ø',
        "aelig" => 'æ',
        "AElig" => 'Æ',
        "alpha" => 'α',
        "beta" => 'β',
        "gamma" => 'γ',
        "delta" => 'δ',
        "epsilon" => 'ε',
        "zeta" => 'ζ',
        "eta" => 'η',
        "theta" => 'θ',
        "iota" => 'ι',
        "kappa" => 'κ',
        "lambda" => 'λ',
        "mu" => 'μ',
        "nu" => 'ν',
        "xi" => 'ξ',
        "pi" => 'π',
        "rho" => 'ρ',
        "sigma" => 'σ',
        "tau" => 'τ',
        "upsilon" => 'υ',
        "phi" => 'φ',
        "chi" => 'χ',
        "psi" => 'ψ',
        "omega" => 'ω',
        "Gamma" => 'Γ',
        "Delta" => 'Δ',
        "Theta" => 'Θ',
        "Lambda" => 'Λ',
        "Xi" => 'Ξ',
        "Pi" => 'Π',
        "Sigma" => 'Σ',
        "Upsilon" => 'Υ',
        "Phi" => 'Φ',
        "Psi" => 'Ψ',
        "Omega" => 'Ω',
        "zwj" => '\u{200D}',
        "zwnj" => '\u{200C}',
        "cross" => '\u{2717}',
        _ => {
            let at = |table: &[&str], base: u32| table.iter().position(|n| *n == name).and_then(|i| char::from_u32(base + i as u32));
            return at(&LATIN1, 0xA0).or_else(|| at(&GREEK_UPPER, 0x391)).or_else(|| at(&GREEK_LOWER, 0x3B1));
        }
    };
    Some(c)
}

#[cfg(test)]
mod tests;
