//! Proofing languages: which dictionary, grammar rules and hyphenation patterns apply to a run of
//! text. A run's language is its BCP 47 tag (`w:lang` in .docx, `CharProps::lang` in the model),
//! resolved with [`ProofLang::from_tag`].

/// A language WordCraft can proof, or [`ProofLang::Other`] for one it can't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProofLang {
    /// American English (`en`, `en-US`, and every region not listed for [`ProofLang::EnGb`], [`ProofLang::EnAu`] or
    /// [`ProofLang::EnCa`]): SCOWL's en-US dictionary ([`crate::scowl`]), the English grammar rules and the
    /// English hyphenation dictionary and patterns. British spellings (`colour`, `centre`,
    /// `organise`) are flagged, with the American form suggested ([`crate::en_variants`]). Also
    /// text with no language at all (WordCraft's default editing language is US English, like
    /// Word's).
    #[default]
    En,
    /// English with British spelling (`en-GB`, `en-NZ`, `en-IE`, `en-ZA`, `en-IN`…):
    /// [`ProofLang::En`]'s tools, but American spellings (`color`, `center`, `analyze`) are
    /// flagged and `-ize` and `-ise` are both right, as in Oxford spelling.
    EnGb,
    /// Australian English (`en-AU`): [`ProofLang::EnGb`]'s spelling policy, against SCOWL's own
    /// en-AU dictionary, which knows Australian words (`billabong`, `brumby`, `larrikin`) and
    /// spells some words the Australian way (`program`, `inquiry`).
    EnAu,
    /// Canadian English (`en-CA`): British `-our`, `-re` and `-ence` (`colour`, `centre`,
    /// `defence`) with American `-ize` (`organize`), `tire` and `program`.
    EnCa,
    /// Polish (`pl`, `pl-PL`): the installed Hunspell dictionary (when there is one), the Polish grammar rules and the Polish
    /// hyphenation patterns.
    Pl,
    /// Bulgarian (`bg`, `bg-BG`): hyphenation by the Bulgarian syllable rules ([`crate::bg`]), and
    /// spelling when a Bulgarian Hunspell dictionary or `bg` pack is found. No grammar rules yet.
    Bg,
    /// Portuguese (`pt`, `pt-BR`; `pt-PT` is [`ProofLang::Pack`] `pt-pt`, with its own dictionary
    /// and these grammar rules and patterns): the installed Hunspell dictionary (VERO and
    /// friends, when there is one), the Portuguese grammar rules and the Portuguese hyphenation
    /// patterns.
    Pt,
    /// Italian (`it`, `it-IT`, `it-CH`): the installed Hunspell dictionary (`it_IT`, when there
    /// is one), the Italian grammar rules and the Italian hyphenation patterns.
    It,
    /// Croatian (`hr`, `hr-HR`)
    Hr,
    /// Turkish (`tr`, `tr-TR`, `tr-CY`): the installed Hunspell dictionary (when there is one),
    /// Turkish grammar rules, spelling suggestions and Turkish syllable hyphenation rules.
    Tr,
    /// Russian (`ru`, `ru-RU`): the installed Hunspell dictionary (when there is one), the Russian
    /// grammar rules and the Russian word-division rules.
    Ru,
    /// Czech (`cs`, `cs-CZ`): the installed Hunspell dictionary, dictionary-driven suggestions
    /// and generic punctuation checks. Hyphenation keeps the fallback English patterns.
    Cs,
    /// Indonesian (`id`, `id-ID`; also the old `in` and ISO 639-2 `ind`): the installed Hunspell
    /// `id_ID` dictionary, dictionary-driven suggestions and generic checks with Indonesian
    /// messages. Hyphenation keeps the fallback English patterns.
    Id,
    /// Spanish of Spain (`es`, `es-ES`, and every region not listed for [`ProofLang::EsMx`],
    /// [`ProofLang::Es419`] or [`ProofLang::EsUy`]; also ISO 639-2 `spa`): the installed
    /// Hunspell `es_ES` dictionary (or a generic Spanish one), dictionary-driven suggestions and
    /// generic checks with Spanish messages ([`crate::es`]). Hyphenation keeps the fallback
    /// English patterns.
    Es,
    /// Mexican Spanish (`es-MX`): [`ProofLang::Es`]'s tools with the installed `es_MX`
    /// dictionary, else a Latin American or generic Spanish one.
    EsMx,
    /// Latin American Spanish (`es-419`, and the Spanish of the Americas other than Mexico and
    /// Uruguay: `es-AR`, `es-CO`, `es-US`…): [`ProofLang::Es`]'s tools with the installed `es_419`
    /// dictionary, else a generic or another Latin American Spanish one.
    Es419,
    /// Uruguayan Spanish (`es-UY`): [`ProofLang::Es`]'s tools with the installed `es_UY`
    /// dictionary (else the neighbouring Rioplatense `es_AR`, a Latin American or a generic one),
    /// and the `vos` forms of verbs (`tenés`, `vení`, `decime`) accepted ([`crate::es::voseo`]).
    EsUy,
    /// German (`de`, `de-DE`, `de-AT`…; `de-CH` and `de-LI` are [`ProofLang::Pack`] `de-ch`, with
    /// their own dictionary and these patterns): hyphenation by the German patterns (reformed
    /// spelling), and spelling when a German Hunspell dictionary or `de` pack is found. No grammar
    /// rules yet.
    De,
    /// Any other language, by its tag (`nl`, `fr-FR`, `sr-Latn`): spelling and suggestions from
    /// its Hunspell dictionary (the dictionaries folder, an installed Language pack, the system's)
    /// through the shared affix engine with default tuning, and hyphenation from its Language
    /// pack. No grammar rules. Without a dictionary or pack it behaves exactly as
    /// [`ProofLang::Other`]: no marks, English hyphenation fallback.
    Pack(PackLocale),
    /// A language without proofing tools here (`x-none`, `zxx`, `und`, a malformed tag). Like Word without
    /// the language's proofing tools: no spelling or grammar marks, so its words are never
    /// flagged as English misspellings. Hyphenation keeps using the English patterns, as before
    /// languages were told apart.
    Other,
}

impl ProofLang {
    /// Every language with a spelling dictionary, in display order.
    pub const PROOFED: [ProofLang; 16] = [
        ProofLang::En,
        ProofLang::EnGb,
        ProofLang::EnAu,
        ProofLang::EnCa,
        ProofLang::Pl,
        ProofLang::Pt,
        ProofLang::It,
        ProofLang::Hr,
        ProofLang::Tr,
        ProofLang::Ru,
        ProofLang::Cs,
        ProofLang::Id,
        ProofLang::Es,
        ProofLang::EsMx,
        ProofLang::Es419,
        ProofLang::EsUy,
    ];

    /// The proofing language of a BCP 47 tag (`pl-PL`, `en_GB`, `PL`…). The primary subtag
    /// counts, and for English and Spanish the region too ([`ProofLang::english`],
    /// [`ProofLang::spanish`]). No tag (or an empty one) is English, WordCraft's default editing
    /// language. Any other valid language tag is [`ProofLang::Pack`] with its [`PackLocale`];
    /// `x-none`, `und`, `zxx`, `mul`, `mis` and malformed tags are [`ProofLang::Other`].
    pub fn from_tag(tag: Option<&str>) -> ProofLang {
        let Some(tag) = tag.map(str::trim).filter(|t| !t.is_empty()) else { return ProofLang::En };
        let primary = tag.split(['-', '_', '.', '@']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("en") || primary.eq_ignore_ascii_case("eng") {
            ProofLang::english(tag)
        } else if primary.eq_ignore_ascii_case("pl") || primary.eq_ignore_ascii_case("pol") {
            ProofLang::Pl
        } else if primary.eq_ignore_ascii_case("bg") || primary.eq_ignore_ascii_case("bul") {
            ProofLang::Bg
        } else if primary.eq_ignore_ascii_case("pt") || primary.eq_ignore_ascii_case("por") {
            // European Portuguese spells differently: its own dictionary (`pt_PT`, a `pt-pt` pack).
            if region(tag).is_some_and(|r| r.eq_ignore_ascii_case("PT")) {
                PackLocale::new("pt-PT").map_or(ProofLang::Pt, ProofLang::Pack)
            } else {
                ProofLang::Pt
            }
        } else if primary.eq_ignore_ascii_case("it") || primary.eq_ignore_ascii_case("ita") {
            ProofLang::It
        } else if primary.eq_ignore_ascii_case("hr") || primary.eq_ignore_ascii_case("hrv") {
            ProofLang::Hr
        } else if primary.eq_ignore_ascii_case("tr") || primary.eq_ignore_ascii_case("tur") {
            ProofLang::Tr
        } else if primary.eq_ignore_ascii_case("ru") || primary.eq_ignore_ascii_case("rus") {
            ProofLang::Ru
        } else if primary.eq_ignore_ascii_case("cs") || primary.eq_ignore_ascii_case("ces") || primary.eq_ignore_ascii_case("cze") {
            ProofLang::Cs
        } else if primary.eq_ignore_ascii_case("id") || primary.eq_ignore_ascii_case("ind") || primary.eq_ignore_ascii_case("in") {
            ProofLang::Id
        } else if primary.eq_ignore_ascii_case("es") || primary.eq_ignore_ascii_case("spa") {
            ProofLang::spanish(tag)
        } else if ["de", "deu", "ger"].iter().any(|c| primary.eq_ignore_ascii_case(c)) {
            // Swiss (and Liechtenstein) German spells without ß: its own dictionary (`de_CH`, a
            // `de-ch` pack).
            if region(tag).is_some_and(|r| r.eq_ignore_ascii_case("CH") || r.eq_ignore_ascii_case("LI")) {
                PackLocale::new("de-CH").map_or(ProofLang::De, ProofLang::Pack)
            } else {
                ProofLang::De
            }
        } else if tag.split_once('@').is_some_and(|(_, m)| m.eq_ignore_ascii_case("latin")) {
            // POSIX `sr_RS@latin`: the Latin-script language (`sr-latn`).
            PackLocale::new(&format!("{primary}-latn")).map_or(ProofLang::Other, ProofLang::Pack)
        } else {
            PackLocale::new(tag).map_or(ProofLang::Other, ProofLang::Pack)
        }
    }

    /// The English of an English tag, by its [`region`]: Canada is [`ProofLang::EnCa`], Australia
    /// [`ProofLang::EnAu`], other regions that spell the British way are [`ProofLang::EnGb`], and the United States, any other
    /// region and no region are [`ProofLang::En`].
    fn english(tag: &str) -> ProofLang {
        match region(tag) {
            Some(r) if r.eq_ignore_ascii_case("CA") => ProofLang::EnCa,
            Some(r) if r.eq_ignore_ascii_case("AU") => ProofLang::EnAu,
            Some(r) if BRITISH_SPELLING.iter().any(|b| b.eq_ignore_ascii_case(r)) => ProofLang::EnGb,
            _ => ProofLang::En,
        }
    }

    /// The Spanish of a Spanish tag, by its [`region`]: Mexico is [`ProofLang::EsMx`], Uruguay
    /// [`ProofLang::EsUy`], Latin America (`419`) and the other countries of the Americas are
    /// [`ProofLang::Es419`], and Spain, any other region and no region are [`ProofLang::Es`].
    fn spanish(tag: &str) -> ProofLang {
        match region(tag) {
            Some(r) if r.eq_ignore_ascii_case("MX") => ProofLang::EsMx,
            Some(r) if r.eq_ignore_ascii_case("UY") => ProofLang::EsUy,
            Some(r) if LATIN_AMERICA.iter().any(|l| l.eq_ignore_ascii_case(r)) => ProofLang::Es419,
            _ => ProofLang::Es,
        }
    }

    /// The built-in language whose hyphenation, grammar and messages a regional
    /// [`ProofLang::Pack`] language shares: German for `de-ch`, Portuguese for `pt-pt` (their
    /// spelling is their own). Any other language is itself.
    pub(crate) fn built_in(self) -> ProofLang {
        match self {
            ProofLang::Pack(l) if l.primary() == "de" => ProofLang::De,
            ProofLang::Pack(l) if l.primary() == "pt" => ProofLang::Pt,
            other => other,
        }
    }

    /// Is this one of the Englishes ([`ProofLang::En`], [`ProofLang::EnGb`], [`ProofLang::EnAu`],
    /// [`ProofLang::EnCa`])?
    pub fn is_english(self) -> bool {
        matches!(self, ProofLang::En | ProofLang::EnGb | ProofLang::EnAu | ProofLang::EnCa)
    }

    /// Is this one of the Spanishes ([`ProofLang::Es`], [`ProofLang::EsMx`], [`ProofLang::Es419`],
    /// [`ProofLang::EsUy`])?
    pub fn is_spanish(self) -> bool {
        matches!(self, ProofLang::Es | ProofLang::EsMx | ProofLang::Es419 | ProofLang::EsUy)
    }

    /// The primary language subtag (`en`, `pl`, `bg`, `tr`, `ru`, `cs`, `id`, `de`, `es`); empty for
    /// [`ProofLang::Other`] and for a [`ProofLang::Pack`] language, whose locale isn't `'static`:
    /// use [`ProofLang::tag`] wherever the code names the language (keys, settings, saved or
    /// reported values), or every pack language would read the same. Kept `'static` so existing
    /// callers (`PROOFED.map(ProofLang::code)`) don't break.
    pub fn code(self) -> &'static str {
        match self {
            ProofLang::Pack(_) => "",
            ProofLang::En | ProofLang::EnGb | ProofLang::EnAu | ProofLang::EnCa => "en",
            ProofLang::Pl => "pl",
            ProofLang::Bg => "bg",
            ProofLang::Pt => "pt",
            ProofLang::It => "it",
            ProofLang::Hr => "hr",
            ProofLang::Tr => "tr",
            ProofLang::Ru => "ru",
            ProofLang::Cs => "cs",
            ProofLang::Id => "id",
            ProofLang::Es | ProofLang::EsMx | ProofLang::Es419 | ProofLang::EsUy => "es",
            ProofLang::De => "de",
            ProofLang::Other => "",
        }
    }

    /// The locale whose Hunspell dictionary (dictionaries folder, Language pack, system) spells
    /// this language through the shared affix engine: a [`ProofLang::Pack`] language's own, and
    /// German's and Bulgarian's, which have hyphenation (and Bulgarian rules) of their own but no
    /// dictionary slot. `None` for the languages with a slot of their own, English and `Other`.
    pub(crate) fn spelling_locale(self) -> Option<PackLocale> {
        match self {
            ProofLang::Pack(l) => Some(l),
            ProofLang::De => PackLocale::new("de-DE"),
            ProofLang::Bg => PackLocale::new("bg-BG"),
            _ => None,
        }
    }

    /// The code that names this language, for keys, settings and reported values: [`code`] for
    /// the built-in languages (`en`, `es`, …), the locale for a [`ProofLang::Pack`] one (`nl`,
    /// `sr-latn`), so no two pack languages share one; empty for [`ProofLang::Other`].
    ///
    /// [`code`]: ProofLang::code
    pub fn tag(self) -> std::borrow::Cow<'static, str> {
        match self {
            ProofLang::Pack(l) => std::borrow::Cow::Owned(l.as_str().to_string()),
            other => std::borrow::Cow::Borrowed(other.code()),
        }
    }

    /// The locale of a [`ProofLang::Pack`] language.
    pub fn pack_locale(self) -> Option<PackLocale> {
        if let ProofLang::Pack(l) = self { Some(l) } else { None }
    }

    /// Does WordCraft check spelling in this language? For a [`ProofLang::Pack`] language, German
    /// and Bulgarian: is a dictionary for it available now (read on first use, cached until the dictionaries folder or
    /// the packs change)?
    pub fn is_proofed(self) -> bool {
        match self.spelling_locale() {
            Some(l) => crate::pack_lang::has_dictionary(l),
            None => Self::PROOFED.contains(&self),
        }
    }
}

/// The locale of a [`ProofLang::Pack`] language: a lowercase BCP 47 tag (`nl`, `fr-fr`,
/// `sr-latn`) of at most [`PackLocale::MAX_LEN`] bytes, stored inline so [`ProofLang`] stays `Copy`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackLocale {
    len: u8,
    bytes: [u8; PackLocale::MAX_LEN],
}

impl PackLocale {
    /// Longest locale kept; a longer tag is cut at a subtag boundary.
    pub const MAX_LEN: usize = 15;

    /// The locale of a language tag (`nl-NL`, `sr_Latn`, `fr-FR.UTF-8`): lowercased, `_` as `-`,
    /// without a POSIX `.…`/`@…` tail or anything from a singleton on (`-x-…`, `-u-…`). `None` for
    /// a tag that isn't a language (`x-none`, `i-…`, `und`, `zxx`, `mul`, `mis`, malformed ones).
    pub fn new(tag: &str) -> Option<PackLocale> {
        let tag = tag.trim().split(['.', '@']).next().unwrap_or("");
        let mut parts = tag.split(['-', '_']);
        let primary = parts.next()?.to_ascii_lowercase();
        if !(2..=8).contains(&primary.len())
            || !primary.bytes().all(|b| b.is_ascii_lowercase())
            || ["und", "zxx", "mul", "mis"].contains(&primary.as_str())
        {
            return None;
        }
        let mut out = primary;
        for p in parts {
            if p.len() <= 1 || p.len() > 8 || !p.bytes().all(|b| b.is_ascii_alphanumeric()) || out.len() + 1 + p.len() > Self::MAX_LEN {
                break;
            }
            out.push('-');
            out.push_str(&p.to_ascii_lowercase());
        }
        let mut bytes = [0u8; Self::MAX_LEN];
        for (b, c) in bytes.iter_mut().zip(out.bytes()) {
            *b = c;
        }
        Some(PackLocale { len: u8::try_from(out.len()).ok()?, bytes })
    }

    /// The locale (`nl`, `fr-fr`).
    pub fn as_str(&self) -> &str {
        self.bytes.get(..usize::from(self.len)).and_then(|b| std::str::from_utf8(b).ok()).unwrap_or_default()
    }

    /// The primary language subtag (`fr`).
    pub fn primary(&self) -> &str {
        self.as_str().split('-').next().unwrap_or_default()
    }
}

impl std::fmt::Debug for PackLocale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PackLocale({:?})", self.as_str())
    }
}

impl std::fmt::Display for PackLocale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Is a subtag a script (four ASCII letters: `Latn`, `cyrl`)?
pub(crate) fn is_script(subtag: &str) -> bool {
    subtag.len() == 4 && subtag.bytes().all(|b| b.is_ascii_alphabetic())
}

/// The script a language is written in when its tag names none, lowercase, for the languages
/// written in more than one (or whose packs may name it): Serbian is Cyrillic, Bosnian and
/// Croatian Latin… `None`: the language doesn't say, and a pack naming a script never serves it.
pub(crate) fn default_script(primary: &str) -> Option<&'static str> {
    Some(match primary {
        "sr" | "ru" | "uk" | "be" | "bg" | "mk" | "kk" | "ky" | "tg" | "mn" | "tt" | "ba" | "cv" | "os" | "ce" => "cyrl",
        "bs" | "hr" | "sh" | "az" | "uz" | "tk" | "ms" | "ha" | "ro" | "sq" | "tr" | "ku" | "so" => "latn",
        "zh" => "hans",
        "pa" => "guru",
        _ => return None,
    })
}

/// The script of a tag (`sr-latn`, `sr_Latn`): the one it names, else its language's default.
pub(crate) fn effective_script(tag: &str) -> Option<String> {
    let mut parts = tag.split(['-', '_']);
    let primary = parts.next().unwrap_or_default().to_ascii_lowercase();
    parts.find(|p| is_script(p)).map(str::to_ascii_lowercase).or_else(|| default_script(&primary).map(str::to_string))
}

/// The region of a tag: the first two-letter or three-digit subtag after the language and
/// script, before any singleton; a POSIX `.UTF-8` or `@…` tail is ignored.
pub(crate) fn region(tag: &str) -> Option<&str> {
    let tag = tag.split(['.', '@']).next().unwrap_or("");
    // Subtags after a singleton (`x-…` private use, `u-…` extensions) aren't regions.
    tag.split(['-', '_'])
        .skip(1)
        .take_while(|p| p.len() != 1)
        .find(|p| (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic())) || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit())))
}

/// Regions whose Spanish is Latin American ([`ProofLang::Es419`]): Latin America and the
/// Caribbean (`419`) and its parts (South America `005`, Central America `013`, the Caribbean
/// `029`), the Spanish-speaking countries of the Americas other than Mexico and Uruguay, and the
/// United States.
const LATIN_AMERICA: [&str; 22] =
    ["419", "005", "013", "029", "AR", "BO", "CL", "CO", "CR", "CU", "DO", "EC", "GT", "HN", "NI", "PA", "PE", "PR", "PY", "SV", "US", "VE"];

/// Regions whose English uses British spelling ([`ProofLang::EnGb`]; Australia has its own,
/// [`ProofLang::EnAu`]): the United Kingdom (and the non-standard `UK`), Ireland, New Zealand, South Africa, India and other
/// Commonwealth countries, the Crown dependencies and Europe (`150`).
const BRITISH_SPELLING: [&str; 25] = [
    "GB", "UK", "IE", "NZ", "ZA", "IN", "SG", "HK", "MY", "MT", "CY", "GI", "IM", "JE", "GG", "JM", "TT", "BS", "BB", "BZ", "KE", "NG", "GH", "PK",
    "150",
];

/// A BCP 47 tag in canonical case (`pl-PL`, `en-GB`, `zh-Hant-TW`, `x-none`), or `None` when
/// `tag` isn't shaped like one: 1–35 ASCII letters, digits and `-`/`_` separators, starting with
/// a 2–8 letter language (or `x`/`i` for private and grandfathered tags). `_` becomes `-`.
pub fn normalize_tag(tag: &str) -> Option<String> {
    let tag = tag.trim();
    if tag.is_empty() || tag.len() > 35 || !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let parts: Vec<&str> = tag.split(['-', '_']).collect();
    if parts.iter().any(|p| p.is_empty() || p.len() > 8) {
        return None;
    }
    let first = parts.first()?;
    let language = (2..=8).contains(&first.len()) && first.chars().all(|c| c.is_ascii_alphabetic());
    if !language && !first.eq_ignore_ascii_case("x") && !first.eq_ignore_ascii_case("i") {
        return None;
    }
    let letters = |p: &str| p.chars().all(|c| c.is_ascii_alphabetic());
    // After a singleton (`x-…` private use, `u-…` extensions) subtags are lower case.
    let singleton = parts.iter().position(|p| p.len() == 1).unwrap_or(usize::MAX);
    let out: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i >= singleton {
                p.to_ascii_lowercase()
            } else if i > 0 && p.len() == 2 && letters(p) {
                p.to_ascii_uppercase() // region
            } else if i > 0 && p.len() == 4 && letters(p) {
                let lower = p.to_ascii_lowercase();
                let mut c = lower.chars();
                c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default() // script
            } else {
                p.to_ascii_lowercase()
            }
        })
        .collect();
    Some(out.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_normalize() {
        assert_eq!(normalize_tag("PL-pl").as_deref(), Some("pl-PL"));
        assert_eq!(normalize_tag(" pl_pl ").as_deref(), Some("pl-PL"));
        assert_eq!(normalize_tag("pl").as_deref(), Some("pl"));
        assert_eq!(normalize_tag("zh-hant-tw").as_deref(), Some("zh-Hant-TW"));
        assert_eq!(normalize_tag("es-419").as_deref(), Some("es-419"));
        assert_eq!(normalize_tag("x-none").as_deref(), Some("x-none"));
        assert_eq!(normalize_tag("de-DE-u-co-PHONEBK").as_deref(), Some("de-DE-u-co-phonebk"));
        for bad in ["", " ", "p", "1pl", "pl--PL", "pl PL", "pl-PL;", "polski język", "-pl", "pl-", "abcdefghi", &"a".repeat(40)] {
            assert_eq!(normalize_tag(bad), None, "{bad:?}");
        }
    }

    fn assert_tags(tags: &[&str], expected: ProofLang) {
        for tag in tags {
            assert_eq!(ProofLang::from_tag(Some(tag)), expected, "{tag}");
        }
    }

    #[test]
    fn tags_resolve_primary_languages() {
        assert_tags(&["pl", "pl-PL", "PL-pl", "pl_PL", "pl_PL.UTF-8", " pl-PL ", "pol", "pl@euro", "pl.UTF-8"], ProofLang::Pl);
        // craft-libs#482 review: a POSIX charset or modifier with no region.
        assert_tags(&["en.UTF-8", "en@euro"], ProofLang::En);
        assert_tags(&["de.UTF-8"], ProofLang::De);
        let sr_latn = ProofLang::Pack(PackLocale::new("sr-latn").unwrap());
        assert_tags(&["sr@latin", "sr_RS@latin", "sr_RS.UTF-8@latin", "sr-Latn"], sr_latn);
        assert_tags(&["sr", "sr@cyrillic"], ProofLang::Pack(PackLocale::new("sr").unwrap()));
        assert_tags(&["pt", "pt-BR", "PT_br", "pt-BR.UTF-8", "pt-AO", "por"], ProofLang::Pt);
        let pt_pt = ProofLang::Pack(PackLocale::new("pt-pt").unwrap());
        assert_tags(&[" pt-PT ", "pt_PT.UTF-8", "por-PT"], pt_pt);
        assert_eq!((pt_pt.built_in(), ProofLang::Pt.built_in()), (ProofLang::Pt, ProofLang::Pt));
        assert_tags(&["it", "it-IT", "IT_it", "it-CH", "it-CH.UTF-8", " it-IT ", "ita"], ProofLang::It);
        assert_tags(&["cs", "cs-CZ", "CS-cz", "cs_CZ", " ces ", "CES", "cze", "CZE"], ProofLang::Cs);
        assert_tags(&["id", "id-ID", "ID-id", "id_ID", "id_ID.UTF-8", " id-ID ", "ind", "in", "in_ID"], ProofLang::Id);
        assert_eq!(ProofLang::Cs.code(), "cs");
        assert!(ProofLang::Cs.is_proofed() && ProofLang::PROOFED.contains(&ProofLang::Cs));
        assert_eq!(ProofLang::Id.code(), "id");
        assert!(ProofLang::Id.is_proofed() && ProofLang::PROOFED.contains(&ProofLang::Id));
    }

    #[test]
    fn tags_resolve_english_regions() {
        assert_tags(&["en", "en-US", "EN_us", "en-Latn-US", "en-PH", "en-001", "eng", "en_US.UTF-8", "en-x-gb"], ProofLang::En);
        assert_tags(
            &["en-GB", "EN_gb", "en_GB.UTF-8", "en-NZ", "en-IE", "en-ZA", "en-IN", "en-Latn-GB", "en-UK", "en-150", "eng-GB", "en_GB@euro"],
            ProofLang::EnGb,
        );
        assert_tags(&["en-CA", "en_CA", "EN-ca", "en_CA.UTF-8", "en-Latn-CA"], ProofLang::EnCa);
        assert_tags(&["en-AU", "en_AU", "EN-au", "en_AU.UTF-8", "en-Latn-AU"], ProofLang::EnAu);
        for language in [ProofLang::En, ProofLang::EnGb, ProofLang::EnAu, ProofLang::EnCa] {
            assert!(language.is_english() && language.is_proofed() && ProofLang::PROOFED.contains(&language), "{language:?}");
            assert_eq!(language.code(), "en");
        }
        assert!(!ProofLang::Pl.is_english() && !ProofLang::Other.is_english());
        // The region is the first two-letter or three-digit subtag: a script or variant isn't one.
        assert_eq!(ProofLang::from_tag(Some("en-Brai")), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("en-GB-oxendict")), ProofLang::EnGb);
    }

    #[test]
    fn tags_resolve_additional_locales_and_defaults() {
        assert_tags(&["bg", "bg-BG", "BG_bg", "bg_BG.UTF-8", "bul"], ProofLang::Bg);
        assert_tags(&["tr", "tr-TR", "tr-CY", "TR_tr", "tur"], ProofLang::Tr);
        assert_tags(&["de", "de-DE", "DE_at", "deu", "ger"], ProofLang::De);
        let de_ch = ProofLang::Pack(PackLocale::new("de-ch").unwrap());
        assert_tags(&["de-CH", "de_CH.UTF-8", "de-LI"], de_ch);
        assert_eq!(de_ch.built_in(), ProofLang::De);
        assert_tags(&["ru", "ru-RU", "RU-ru", "ru_RU", "ru_RU.UTF-8", " ru-RU ", "rus"], ProofLang::Ru);
        // German and Bulgarian are proofed when a dictionary is found (craft-libs#485 review: an
        // installed hunspell-de-de must not fail this test), so only their locales are fixed here.
        assert_eq!(ProofLang::De.spelling_locale(), PackLocale::new("de-de"));
        assert_eq!(ProofLang::Bg.spelling_locale(), PackLocale::new("bg-bg"));
        assert!(!ProofLang::PROOFED.contains(&ProofLang::De) && !ProofLang::PROOFED.contains(&ProofLang::Bg));
        assert_eq!(ProofLang::Bg.code(), "bg");
        assert!(ProofLang::Pt.is_proofed() && !ProofLang::Other.is_proofed());
        assert!(ProofLang::Tr.is_proofed());
        assert!(ProofLang::Ru.is_proofed());
        assert_eq!(ProofLang::Ru.code(), "ru");
        assert!(ProofLang::PROOFED.contains(&ProofLang::Ru));
        assert_eq!(ProofLang::Pl.code(), "pl");
        assert_eq!(ProofLang::Pt.code(), "pt");
        assert!(ProofLang::It.is_proofed());
        assert_eq!(ProofLang::It.code(), "it");
        assert!(ProofLang::Hr.is_proofed());
        assert_eq!(ProofLang::Hr.code(), "hr");
        assert_eq!(ProofLang::Tr.code(), "tr");
        assert!(ProofLang::PROOFED.contains(&ProofLang::Pt), "every proofed language is listed");
        assert_eq!(ProofLang::from_tag(None), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("")), ProofLang::En);
        assert_eq!(ProofLang::from_tag(Some("  ")), ProofLang::En);
        assert!(ProofLang::Pl.is_proofed() && !ProofLang::Other.is_proofed());
    }

    /// Two pack languages never share a key: `tag()` is the locale, while `code()` is empty.
    #[test]
    fn tag_names_each_pack_language() {
        let (nl, fr) = (ProofLang::from_tag(Some("nl")), ProofLang::from_tag(Some("fr-FR")));
        assert_eq!((nl.code(), fr.code()), ("", ""));
        assert_eq!((nl.tag().as_ref(), fr.tag().as_ref()), ("nl", "fr-fr"));
        for l in ProofLang::PROOFED {
            assert_eq!(l.tag(), l.code());
        }
        assert_eq!(ProofLang::Other.tag(), "");
    }

    #[test]
    fn tags_reject_unknown_or_partial_language_codes() {
        // A partial code is another language, never the one it starts like.
        for tag in ["fr-FR", "dex", "ja-JP", "plx", "bgx", "trx", "csx", "idx", "inx", "esx", "spax"] {
            let lang = ProofLang::from_tag(Some(tag));
            assert_eq!(lang.pack_locale().map(|l| l.to_string()), Some(tag.to_ascii_lowercase()), "{tag}");
            assert_eq!(lang.code(), "", "{tag}: a pack language's locale is pack_locale()");
            assert_eq!(lang.tag(), lang.pack_locale().map(|l| l.as_str().to_string()).unwrap_or_default(), "{tag}");
        }
        assert_tags(&["x-none", "e", "-", "und", "zxx", "i-klingon"], ProofLang::Other);
    }

    /// #3360, #3553: Spanish is proofed, by region: Spain (and no region), Mexico, Uruguay, and
    /// Latin America with the other countries of the Americas.
    #[test]
    fn spanish_tags_resolve_by_region() {
        for t in ["es", "es-ES", "ES_es", "es_ES.UTF-8", " es-ES ", "spa", "es-Latn-ES", "es-GQ", "es-PH", "es-x-mx", "es_ES@euro"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Es, "{t}");
        }
        for t in ["es-MX", "es_MX", "ES-mx", "es_MX.UTF-8", "spa-MX", "es-Latn-MX"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::EsMx, "{t}");
        }
        for t in ["es-419", "es_419", "es-AR", "es-CO", "es_CL.UTF-8", "es-US", "ES-ve", "es-PR", "es-Latn-419", "es-005", "spa-PE"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::Es419, "{t}");
        }
        for t in ["es-UY", "es_UY", "ES-uy", "es_UY.UTF-8", "spa-UY", "es-Latn-UY"] {
            assert_eq!(ProofLang::from_tag(Some(t)), ProofLang::EsUy, "{t}");
        }
        for l in [ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy] {
            assert!(l.is_spanish() && l.is_proofed() && ProofLang::PROOFED.contains(&l), "{l:?}");
            assert!(!l.is_english());
            assert_eq!(l.code(), "es");
        }
        assert!(!ProofLang::En.is_spanish() && !ProofLang::Pt.is_spanish() && !ProofLang::Other.is_spanish());
        assert_eq!(normalize_tag("ES_mx").as_deref(), Some("es-MX"));
    }
}
