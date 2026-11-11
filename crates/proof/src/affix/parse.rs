//! Reading a Hunspell `.aff` into [`Rules`]: conditions, compound-rule patterns, conversion tables, and the directive loop.

use super::*;

/// How often a `COMPOUNDRULE` token's flag may repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Repeat {
    Once,
    /// `?`
    Optional,
    /// `*`
    Any,
}

/// A `COMPOUNDRULE` pattern as written (`n*1t`, or `(aa)*(bb)` with long or numeric flags): its
/// tokens, each a flag and how often it repeats.
pub(super) fn parse_compound_rule(pattern: &str, mode: FlagMode) -> Result<Vec<(FlagKey, Repeat)>, String> {
    let mut out: Vec<(FlagKey, Repeat)> = Vec::new();
    let mut chars = pattern.chars().filter(|&c| c != EMOJI_SELECTOR);
    // With long or numeric flags, what isn't a flag in parentheses (the Mongolian dictionary
    // writes such rules) is read as the CLI answers (black-box): a bracketed list (`[a0,a1]`) is
    // skipped with its `*` or `?`, and a punctuation mark (`%?`, `.`) makes a rule that matches
    // nothing (a token no stem carries).
    let mut skipped = false;
    while let Some(c) = chars.next() {
        if mode != FlagMode::Char {
            let skip = match c {
                '[' => {
                    if !chars.by_ref().any(|c| c == ']') {
                        return Err(format!("COMPOUNDRULE `{pattern}`: `[` is not closed"));
                    }
                    true
                }
                '*' | '?' => skipped,
                c if !c.is_alphanumeric() && !matches!(c, '(' | ')') => return Ok(vec![(FlagKey::MAX, Repeat::Once)]),
                _ => false,
            };
            skipped = skip;
            if skip {
                continue;
            }
        }
        match c {
            '*' | '?' => {
                let Some(last) = out.last_mut().filter(|t| t.1 == Repeat::Once) else {
                    return Err(format!("COMPOUNDRULE `{pattern}`: `{c}` follows no flag"));
                };
                last.1 = if c == '*' { Repeat::Any } else { Repeat::Optional };
            }
            '(' => {
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some(')') => break,
                        Some(c) if inner.len() < MAX_COMPOUND_BYTES => inner.push(c),
                        _ => return Err(format!("COMPOUNDRULE `{pattern}`: `(` is not closed")),
                    }
                }
                let key = mode.parse_one(&inner).map_err(|e| format!("COMPOUNDRULE `{pattern}`: {e}"))?;
                out.push((key, Repeat::Once));
            }
            // In one-character mode a `)` that closes no group is a flag (Swedish: `)k`), as the
            // CLI reads it.
            ')' if mode == FlagMode::Char => out.push((FlagKey::from(c), Repeat::Once)),
            ')' => return Err(format!("COMPOUNDRULE `{pattern}`: `)` closes no group")),
            c if mode == FlagMode::Char => out.push((FlagKey::from(c), Repeat::Once)),
            _ => return Err(format!("COMPOUNDRULE `{pattern}`: long and numeric flags are written in ( )")),
        }
        if out.len() > MAX_COMPOUND_TOKENS {
            return Err(format!("COMPOUNDRULE `{pattern}`: more than {MAX_COMPOUND_TOKENS} flags"));
        }
    }
    if out.is_empty() {
        return Err("COMPOUNDRULE with no flags".into());
    }
    Ok(out)
}

/// One character class of a rule's condition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Cond {
    Any,
    Char(char),
    /// Sorted and deduplicated.
    Set {
        negated: bool,
        chars: Box<[char]>,
    },
}

impl Cond {
    pub(super) fn matches(&self, c: char) -> bool {
        match self {
            Cond::Any => true,
            Cond::Char(x) => *x == c,
            Cond::Set { negated, chars } => chars.binary_search(&c).is_ok() != *negated,
        }
    }
}

/// Parse a Hunspell condition (`.`, `[^i]e`, `[km]anie`…).
pub(super) fn parse_cond(text: &str) -> Result<Box<[Cond]>, String> {
    if text == "." {
        return Ok(Box::new([]));
    }
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '.' => out.push(Cond::Any),
            '[' => {
                let mut set = Vec::new();
                let mut negated = false;
                let mut closed = false;
                let mut first = true;
                for d in chars.by_ref() {
                    if d == ']' {
                        closed = true;
                        break;
                    }
                    if first && d == '^' {
                        negated = true;
                    } else {
                        set.push(d);
                    }
                    first = false;
                }
                if !closed {
                    return Err(format!("unclosed `[` in condition `{text}`"));
                }
                // Sorted, so a character is looked up, not compared with each (a hostile
                // condition can list thousands).
                set.sort_unstable();
                set.dedup();
                out.push(Cond::Set { negated, chars: set.into_boxed_slice() });
            }
            ']' => return Err(format!("stray `]` in condition `{text}`")),
            c => out.push(Cond::Char(c)),
        }
    }
    Ok(out.into_boxed_slice())
}

/// An `ICONV` or `OCONV` table: `(from, to)` pairs in source order, found by the first character
/// of `from` so that converting a word costs a look at the few pairs that could start at each
/// character, not at the whole table (the Korean tables hold 11,172 pairs each).
#[derive(Clone, Debug, Default)]
pub(super) struct Conv {
    pub(super) pairs: Vec<(String, String)>,
    /// `from` → index into `pairs` (of equal ones, the later pair: it is the one that wins).
    by_from: FxMap<String, u32>,
    /// Longest `from`, in bytes.
    longest: usize,
}

impl Conv {
    /// The table of `pairs` (checked by [`Rules::push_conv`]: non-empty `from`, bounded sizes).
    pub(super) fn new(pairs: Vec<(String, String)>) -> Conv {
        let mut by_from: FxMap<String, u32> = FxMap::default();
        for (i, (from, _)) in pairs.iter().enumerate() {
            if let Ok(i) = u32::try_from(i) {
                by_from.insert(from.clone(), i);
            }
        }
        let longest = pairs.iter().map(|p| p.0.len()).max().unwrap_or(0);
        Conv { pairs, by_from, longest }
    }

    fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The pair to apply at the start of `rest`: the longest `from` that starts it. One hash
    /// lookup per prefix of `rest` up to the longest `from` ([`MAX_CONV_BYTES`]), however many
    /// pairs there are (craft-libs#489 review: 65,536 pairs on one first character).
    fn at(&self, rest: &str) -> Option<&(String, String)> {
        let ends = rest.char_indices().map(|(i, _)| i).skip(1).chain(std::iter::once(rest.len()));
        let ends: Vec<usize> = ends.take_while(|&e| e <= self.longest).collect();
        ends.iter().rev().find_map(|&e| self.by_from.get(rest.get(..e)?)).and_then(|&i| self.pairs.get(i as usize))
    }

    /// `w` converted: at each position the longest matching `from` is replaced by its `to`, and
    /// characters in `drop` are removed (outside the replacements too). Borrowed when nothing
    /// changes.
    pub(super) fn apply<'a>(&self, w: &'a str, drop: &[char]) -> std::borrow::Cow<'a, str> {
        if self.is_empty() && drop.is_empty() {
            return std::borrow::Cow::Borrowed(w);
        }
        // One pass: the input is copied only from the first change on.
        let mut out: Option<String> = None;
        let mut rest = w;
        while let Some(c) = rest.chars().next() {
            let done = w.get(..w.len() - rest.len()).unwrap_or_default();
            match self.at(rest) {
                Some((from, to)) => {
                    out.get_or_insert_with(|| done.to_string()).extend(to.chars().filter(|c| !drop.contains(c)));
                    rest = rest.get(from.len()..).unwrap_or_default();
                }
                None => {
                    if drop.contains(&c) {
                        out.get_or_insert_with(|| done.to_string());
                    } else if let Some(o) = out.as_mut() {
                        o.push(c);
                    }
                    rest = rest.get(c.len_utf8()..).unwrap_or_default();
                }
            }
        }
        out.map_or(std::borrow::Cow::Borrowed(w), std::borrow::Cow::Owned)
    }
}

/// Parsed rules and tables, shared by the Hunspell reader and the compact format.
#[derive(Default)]
pub(super) struct Rules {
    pub(super) prefixes: Vec<RawRule>,
    pub(super) suffixes: Vec<RawRule>,
    pub(super) try_chars: String,
    pub(super) key: String,
    pub(super) map: Vec<Vec<char>>,
    pub(super) rep: Vec<(String, String)>,
    /// The `FORBIDDENWORD` flag: stems carrying it are listed but never correct.
    pub(super) forbidden: Option<FlagKey>,
    pub(super) no_suggest: Option<FlagKey>,
    pub(super) substandard: Option<FlagKey>,
    /// The `KEEPCASE` flag.
    pub(super) keepcase: Option<FlagKey>,
    pub(super) need_affix: Option<FlagKey>,
    pub(super) circumfix: Option<FlagKey>,
    pub(super) full_strip: bool,
    pub(super) check_sharps: bool,
    pub(super) iconv: Vec<(String, String)>,
    pub(super) oconv: Vec<(String, String)>,
    pub(super) ignore: Vec<char>,
    /// `COMPOUNDRULE` patterns, as written (`n*1t`); read in the `FLAG` mode by
    /// [`AffixDict::build`], since the `FLAG` line may come after them.
    pub(super) compound_rules: Vec<String>,
    /// The `ONLYINCOMPOUND` flag.
    pub(super) only_in_compound: Option<FlagKey>,
    /// `COMPOUNDMIN`: the shortest part of a compound, in characters (Hunspell's default is 3).
    pub(super) compound_min: Option<usize>,
    /// Flag-based compounding directives.
    pub(super) cpd: compound::Raw,
    /// `AF` flag aliases: alias `n` (1-based) stands for `aliases[n - 1]`.
    pub(super) aliases: Vec<String>,
    /// The `AF` table opened with its count line, so flag fields are alias numbers. A table
    /// without one is rejected, as in Hunspell: its lines are ignored.
    pub(super) aliases_on: bool,
    /// Total length of the alias texts, capped at [`MAX_ALIAS_BYTES`].
    pub(super) alias_bytes: usize,
    /// How flags are written, from the `FLAG` line.
    pub(super) mode: FlagMode,
    /// Whether a flag has been read: a later `FLAG` line can't change how the earlier ones read.
    pub(super) flag_read: bool,
}

impl Rules {
    /// A `FLAG` line. One that changes the mode after a flag was read is refused: Hunspell loads
    /// such a file, reading the earlier flags in the earlier mode, but no real dictionary does
    /// this, and refusing it is safe where half-reading it would not be.
    pub(super) fn set_mode(&mut self, name: &str) -> Result<(), String> {
        let mode = FlagMode::from_name(name).ok_or_else(|| format!("FLAG {name} is not supported"))?;
        if self.flag_read && mode != self.mode {
            return Err(format!("FLAG {name} after flags were read"));
        }
        self.mode = mode;
        Ok(())
    }

    /// Add a pair to the `ICONV` (`output` false) or `OCONV` table: at most [`MAX_CONV_PAIRS`]
    /// pairs of at most [`MAX_CONV_BYTES`] each side. A pair with nothing to convert is skipped.
    pub(super) fn push_conv(&mut self, output: bool, from: &str, to: &str) -> Result<(), String> {
        let (name, table) = if output { ("OCONV", &mut self.oconv) } else { ("ICONV", &mut self.iconv) };
        if from.len() > MAX_CONV_BYTES || to.len() > MAX_CONV_BYTES {
            return Err(format!("{name} pair longer than {MAX_CONV_BYTES} bytes"));
        }
        if table.len() >= MAX_CONV_PAIRS {
            return Err(format!("{name} table larger than {MAX_CONV_PAIRS} pairs"));
        }
        if !from.is_empty() {
            table.push((from.to_string(), to.to_string()));
        }
        Ok(())
    }

    /// The one flag of a rule header or a single-flag directive.
    pub(super) fn one_flag(&mut self, field: &str) -> Result<FlagKey, String> {
        self.flag_read = true;
        self.mode.parse_one(field)
    }

    /// Flags as written in a `.dic` entry or a continuation class: the flag characters
    /// themselves, or with an `AF` table the alias its leading digits number, as Hunspell reads
    /// them (`2x` is alias 2). A field without digits or an undefined alias reads as no flags:
    /// the stem is still a word, the rule still applies, without the flags (Hunspell warns).
    fn flags<'a>(&'a self, written: &'a str) -> &'a str {
        if !self.aliases_on {
            return written;
        }
        self.alias_index(written).and_then(|i| self.aliases.get(i)).map_or("", String::as_str)
    }

    /// With an `AF` table, the index in `aliases` of the alias `written` numbers, if defined.
    pub(super) fn alias_index(&self, written: &str) -> Option<usize> {
        if !self.aliases_on {
            return None;
        }
        let digits = written.bytes().take_while(u8::is_ascii_digit).count();
        let i = written.get(..digits)?.parse::<usize>().ok()?.checked_sub(1)?;
        (i < self.aliases.len()).then_some(i)
    }

    /// `w` without the `IGNORE` characters.
    pub(super) fn strip_ignored(&self, w: &str) -> String {
        if self.ignore.is_empty() { w.to_string() } else { w.chars().filter(|c| !self.ignore.contains(c)).collect() }
    }
}

/// The fields of an `.aff`/`.dic` line: Hunspell separates them with spaces and tabs only, so a
/// flag byte read as U+0085 or U+00A0 (see [`crate::hunspell::decode`]) stays a flag.
pub(super) fn split_fields(line: &str) -> impl Iterator<Item = &str> {
    line.split([' ', '\t']).filter(|f| !f.is_empty())
}

/// A rule as parsed from text, before flags get ids: `(flag, cross, strip, add, cont, cond)`.
pub(super) type RawRule = (FlagKey, bool, String, String, String, String);

/// Most distinct `TRY` characters kept; a hostile `.aff` could otherwise make every suggestion
/// round cost time and memory proportional to the file size.
const MAX_TRY_CHARS: usize = 128;

/// Keeps the first [`MAX_TRY_CHARS`] distinct characters of a `TRY` value, in order.
pub(super) fn cap_try(s: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in s.chars() {
        if n >= MAX_TRY_CHARS {
            break;
        }
        if !out.contains(c) {
            out.push(c);
            n += 1;
        }
    }
    out
}

/// Most `REP` entries kept; every suggestion round tries each one at every place it matches.
pub(super) const MAX_REP_ENTRIES: usize = 4096;

/// Adds a `REP` entry (`_` stands for a space) unless the table is full, `from` is empty or longer
/// than any word a suggester takes (it could never match), or `to` is longer than a dictionary word.
pub(super) fn push_rep(rep: &mut Vec<(String, String)>, from: &str, to: &str) {
    // Letters, not bytes: a 40-letter word in a four-byte script is 160 bytes long.
    let fits = from.chars().nth(crate::affix_suggest::MAX_LETTERS).is_none();
    if rep.len() < MAX_REP_ENTRIES && !from.is_empty() && fits && to.len() <= MAX_WORD_BYTES {
        rep.push((from.replace('_', " "), to.replace('_', " ")));
    }
}

/// `form`, unless it is empty: no word is, and `FULLSTRIP` can strip a stem down to nothing.
pub(super) fn non_empty(form: String) -> Option<String> {
    Some(form).filter(|f| !f.is_empty())
}

/// `0` means empty in Hunspell's strip and append fields.
pub(super) fn zero_empty(s: &str) -> String {
    if s == "0" { String::new() } else { s.to_string() }
}

/// Read Hunspell `.aff` text (already decoded to UTF-8). `english`: ignore the
/// flag-based compounding directives (see [`compound::is_directive`]).
pub(super) fn parse_aff(aff: &str, english: bool) -> Result<Rules, String> {
    let mut r = Rules::default();
    let mut cross: HashMap<(bool, FlagKey), bool> = HashMap::new();
    let mut map_header = false;
    let mut rep_header = false;
    let mut af_header = false;
    let mut iconv_header = false;
    let mut compound_header = false;
    let mut oconv_header = false;
    for (n, line) in aff.lines().enumerate() {
        let f: Vec<&str> = split_fields(line).collect();
        let Some(&dir) = f.first() else { continue };
        if dir.starts_with('#') {
            continue;
        }
        let at = |k: usize| f.get(k).copied().unwrap_or("");
        match dir {
            "SET" | "NAME" | "VERSION" | "HOME" | "WORDCHARS" | "MAXNGRAMSUGS" | "MAXDIFF" | "ONLYMAXDIFF" | "SUGSWITHDOTS" | "WARN"
            | "FORBIDWARN" => {}
            // Morphological aliases, and a flag that only matters for morphological generation:
            // the words it marks are correct.
            "AM" | "LEMMA_PRESENT" => {}
            "SUBSTANDARD" => r.substandard = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            // The first `AF`, `ICONV` or `OCONV` line is the table's count, as in Hunspell, even with a
            // trailing comment (`AF 1559 # aliases`).
            "AF" => {
                if !af_header {
                    af_header = true;
                    // A count below 1 rejects the table too: flags stay literal (`AF 0`).
                    r.aliases_on = at(1).parse::<usize>().is_ok_and(|n| n > 0);
                } else if r.aliases_on {
                    r.alias_bytes = r.alias_bytes.saturating_add(at(1).len());
                    if r.aliases.len() >= MAX_ALIASES || r.alias_bytes > MAX_ALIAS_BYTES {
                        return Err(format!("line {}: AF table over {MAX_ALIASES} aliases or {MAX_ALIAS_BYTES} bytes", n + 1));
                    }
                    r.aliases.push(at(1).to_string());
                }
            }
            "ICONV" | "OCONV" => {
                let header = if dir == "OCONV" { &mut oconv_header } else { &mut iconv_header };
                if !*header && at(1).parse::<usize>().is_ok() {
                    *header = true;
                } else if f.len() >= 3 {
                    r.push_conv(dir == "OCONV", at(1), at(2)).map_err(|e| format!("line {}: {e}", n + 1))?;
                }
            }
            "IGNORE" => r.ignore = at(1).chars().collect(),
            "KEEPCASE" => r.keepcase = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            // The first `COMPOUNDRULE` line is the table's count, as in Hunspell.
            "COMPOUNDRULE" => {
                if !compound_header && at(1).parse::<usize>().is_ok() {
                    compound_header = true;
                } else {
                    if r.compound_rules.len() >= MAX_COMPOUND_RULES {
                        return Err(format!("line {}: more than {MAX_COMPOUND_RULES} COMPOUNDRULE patterns", n + 1));
                    }
                    r.compound_rules.push(at(1).to_string());
                }
            }
            "ONLYINCOMPOUND" => {
                r.only_in_compound = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?);
            }
            "COMPOUNDMIN" => {
                let min = at(1).parse::<usize>().map_err(|_| format!("line {}: COMPOUNDMIN needs a number", n + 1))?;
                r.compound_min = Some(min.clamp(1, MAX_COMPOUND_BYTES));
            }
            "NEEDAFFIX" => r.need_affix = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            "CIRCUMFIX" => r.circumfix = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            "FULLSTRIP" => r.full_strip = true,
            "CHECKSHARPS" => r.check_sharps = true,
            "FLAG" => r.set_mode(at(1)).map_err(|e| format!("line {}: {e}", n + 1))?,
            "FORBIDDENWORD" => r.forbidden = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            "NOSUGGEST" => r.no_suggest = Some(r.one_flag(at(1)).map_err(|e| format!("line {}: {dir} needs one flag: {e}", n + 1))?),
            "TRY" => r.try_chars = cap_try(at(1)),
            "KEY" => r.key = at(1).to_string(),
            "MAP" => {
                if !map_header && at(1).parse::<usize>().is_ok() {
                    map_header = true;
                } else if !at(1).contains('(') {
                    r.map.push(at(1).chars().collect());
                }
            }
            "REP" => {
                if !rep_header && at(1).parse::<usize>().is_ok() {
                    rep_header = true;
                } else if f.len() >= 3 {
                    push_rep(&mut r.rep, at(1), at(2));
                }
            }
            "PFX" | "SFX" => {
                let suffix = dir == "SFX";
                let flag = r.one_flag(at(1)).map_err(|e| format!("line {}: {e}", n + 1))?;
                // Only a header ignores trailing fields; a rule condition beginning with # is literal.
                let is_header = f.len() >= 4 && matches!(at(2), "Y" | "N") && at(3).parse::<usize>().is_ok() && !cross.contains_key(&(suffix, flag));
                if is_header {
                    cross.insert((suffix, flag), at(2) == "Y");
                    continue;
                }
                if f.len() < 4 {
                    return Err(format!("line {}: short affix rule", n + 1));
                }
                // `able/B`: the appended text and its continuation classes.
                let (add, cont) = match at(3).split_once('/') {
                    Some((a, c)) => (a, c),
                    None => (at(3), ""),
                };
                let cond = if f.len() >= 5 { at(4) } else { "." };
                parse_cond(cond).map_err(|e| format!("line {}: {e}", n + 1))?;
                let c = cross.get(&(suffix, flag)).copied().unwrap_or(false);
                let rule: RawRule = (flag, c, zero_empty(at(2)), zero_empty(add), cont.to_string(), cond.to_string());
                if suffix {
                    r.suffixes.push(rule);
                } else {
                    r.prefixes.push(rule);
                }
            }
            // English dictionaries compound only ordinals, by `COMPOUNDRULE` (read above): their
            // flag-based compounding directives don't change which English words are right.
            d if english && compound::is_directive(d) => {}
            d if UNSUPPORTED.contains(&d) => return Err(format!("line {}: {d} is not supported", n + 1)),
            _ => {
                let mut cpd = std::mem::take(&mut r.cpd);
                let read = cpd.read(&f, &mut |x| r.one_flag(x));
                r.cpd = cpd;
                read.map_err(|e| format!("line {}: {e}", n + 1))?;
            }
        }
    }
    // Aliases may be declared after the rules that use them; resolve once all are read.
    // Hunspell removes `IGNORE` characters from the appended text only: a stripped text that
    // holds one never matches a stem (stems lose them too), so the rule never applies.
    let resolve = |rules: &mut Vec<RawRule>, r: &Rules| {
        for rule in rules.iter_mut() {
            rule.4 = r.flags(&rule.4).to_string();
            rule.3 = r.strip_ignored(&rule.3);
        }
    };
    let (mut pfx, mut sfx) = (std::mem::take(&mut r.prefixes), std::mem::take(&mut r.suffixes));
    resolve(&mut pfx, &r);
    resolve(&mut sfx, &r);
    (r.prefixes, r.suffixes) = (pfx, sfx);
    Ok(r)
}
