//! WordCraft's compact byte format: `to_bytes` and `from_bytes`.

use super::*;

impl AffixDict {
    /// Serialize to WordCraft's compact format: raw-deflate of `WCAFF2`, a `FLAG` line when flags
    /// aren't one character each, the comments, the tables and rules as text lines, a `WORDS`
    /// line, then one line per stem, front-coded against the previous one (a byte with the shared
    /// prefix length in bytes, the rest of the word, `/` and its flags if any). Flags are written
    /// as the `.aff` wrote them, so one-character data is the same as before `FLAG` lines existed.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let mut out = String::new();
        out.push_str(MAGIC);
        out.push('\n');
        if let Some(name) = self.flag_mode.name() {
            out.push_str(&format!("FLAG {name}\n"));
        }
        for c in &self.comments {
            out.push_str("# ");
            out.push_str(c.trim());
            out.push('\n');
        }
        if !self.try_chars.is_empty() {
            out.push_str(&format!("TRY {}\n", self.try_chars));
        }
        if !self.key.is_empty() {
            out.push_str(&format!("KEY {}\n", self.key));
        }
        for m in &self.map {
            out.push_str(&format!("MAP {}\n", m.iter().collect::<String>()));
        }
        for (a, b) in &self.rep {
            out.push_str(&format!("REP {} {}\n", a.replace(' ', "_"), b.replace(' ', "_")));
        }
        // In one-character mode a directive flag that no stem or rule uses isn't written, as
        // before flag ids existed, so those dictionaries' compact data stays byte for byte the same.
        let used = |flag: u16| {
            self.flags.iter().chain(self.cpd_sets.values()).chain(self.homonyms.values().flatten()).any(|&id| self.set(id).contains(&flag))
                || self.prefixes.iter().chain(&self.suffixes).any(|r| r.flag == flag || self.set(r.cont).contains(&flag))
        };
        if self.full_strip {
            out.push_str("FULLSTRIP\n");
        }
        if self.check_sharps {
            out.push_str("CHECKSHARPS\n");
        }
        for (directive, flag) in [
            ("FORBIDDENWORD", self.forbidden),
            ("NOSUGGEST", self.no_suggest),
            ("SUBSTANDARD", self.substandard),
            ("KEEPCASE", self.keepcase),
            ("NEEDAFFIX", self.need_affix),
            ("CIRCUMFIX", self.circumfix),
        ] {
            if let Some(flag) = flag.filter(|&f| self.flag_mode != FlagMode::Char || used(f)) {
                out.push_str(directive);
                out.push(' ');
                self.write_flags(&[flag], &mut out);
                out.push('\n');
            }
        }
        for (name, table) in [("ICONV", &self.iconv), ("OCONV", &self.oconv)] {
            for (a, b) in &table.pairs {
                out.push_str(&format!("{name} {a} {b}\n"));
            }
        }
        if !self.ignore.is_empty() {
            out.push_str(&format!("IGNORE {}\n", self.ignore.iter().collect::<String>()));
        }
        // ONLYINCOMPOUND goes after COMPOUNDMIN, as before flag ids, so one-character data stays
        // byte for byte the same; without compound rules it is still written (its stems are no
        // words on their own).
        let only_in_compound = self.only_in_compound.filter(|&f| self.flag_mode != FlagMode::Char || used(f));
        let write_only_in_compound = |out: &mut String| {
            if let Some(flag) = only_in_compound {
                out.push_str("ONLYINCOMPOUND ");
                self.write_flags(&[flag], out);
                out.push('\n');
            }
        };
        if self.compound_text.is_empty() && self.cpd.is_none() {
            write_only_in_compound(&mut out);
        } else {
            out.push_str(&format!("COMPOUNDMIN {}\n", self.compound_min));
            write_only_in_compound(&mut out);
            for p in &self.compound_text {
                out.push_str(&format!("COMPOUNDRULE {p}\n"));
            }
        }
        if let Some(c) = &self.cpd {
            c.write(&mut out, &|k, out: &mut String| self.flag_mode.write(std::iter::once(k), out));
        }
        let z = |s: &str| if s.is_empty() { "0".to_string() } else { s.to_string() };
        for (kind, rules) in [("PFX", &self.prefixes), ("SFX", &self.suffixes)] {
            for r in rules {
                let y = if r.cross { "Y" } else { "N" };
                // Continuation classes ride on the appended field, as in the `.aff` source.
                let mut add = z(&r.add);
                let cont = self.set(r.cont);
                if !cont.is_empty() {
                    add.push('/');
                    self.write_flags(cont, &mut add);
                }
                let mut flag = String::new();
                self.write_flags(&[r.flag], &mut flag);
                out.push_str(&format!("{kind} {flag} {y} {} {} {}\n", z(&r.strip), add, r.cond_text));
            }
        }
        out.push_str("WORDS\n");
        let mut raw = out.into_bytes();
        let mut prev: &[u8] = &[];
        for i in 0..self.len() {
            let Some(w) = self.word(i) else { continue };
            let b = w.as_bytes();
            let p = prev.iter().zip(b).take_while(|(x, y)| x == y).count().min(255);
            raw.push(p as u8);
            raw.extend_from_slice(b.get(p..).unwrap_or_default());
            // The homonyms kept apart each on a line, or their flags merged; then the
            // ONLYINCOMPOUND homonyms' flags. The same stem again is merged back on reading.
            let at = u32::try_from(i).ok();
            let main = self.flags.get(i).copied().unwrap_or_default();
            let apart = at.and_then(|i| self.homonyms.get(&i)).map_or(std::slice::from_ref(&main), |h| &h[..]);
            let cpd = at.and_then(|i| self.cpd_sets.get(&i));
            for (k, &id) in apart.iter().chain(cpd).enumerate() {
                if k > 0 {
                    raw.push(b.len().min(255) as u8);
                    raw.extend_from_slice(b.get(b.len().min(255)..).unwrap_or_default());
                }
                let set = self.set(id);
                if !set.is_empty() {
                    let mut flags = String::from("/");
                    self.write_flags(set, &mut flags);
                    raw.extend_from_slice(flags.as_bytes());
                }
                raw.push(b'\n');
            }
            prev = b;
        }
        // What [`AffixDict::from_bytes`] would refuse to inflate is refused here.
        if raw.len() > MAX_INFLATED {
            return Err(format!("affix dictionary: {} bytes uncompressed, over the {MAX_INFLATED} the reader takes", raw.len()));
        }
        Ok(miniz_oxide::deflate::compress_to_vec(&raw, 10))
    }

    /// Read data written by [`AffixDict::to_bytes`].
    pub fn from_bytes(data: &[u8]) -> Result<AffixDict, String> {
        let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_INFLATED)
            .map_err(|e| format!("affix dictionary: inflate failed: {:?}", e.status))?;
        let words_at = raw.windows(7).position(|w| w == b"\nWORDS\n").ok_or("affix dictionary: no WORDS section")?;
        let head = std::str::from_utf8(raw.get(..words_at).unwrap_or_default()).map_err(|_| "affix dictionary: bad utf-8 in the rules")?;
        let mut lines = head.lines();
        if lines.next() != Some(MAGIC) {
            return Err("affix dictionary: bad header".into());
        }
        let mut rules = Rules::default();
        let mut comments = Vec::new();
        for line in lines {
            if let Some(c) = line.strip_prefix("# ") {
                comments.push(c.to_string());
                continue;
            }
            let f: Vec<&str> = split_fields(line).collect();
            let at = |k: usize| f.get(k).copied().unwrap_or("");
            match at(0) {
                "FLAG" => rules.set_mode(at(1)).map_err(|e| format!("affix dictionary: {e}"))?,
                "TRY" => rules.try_chars = cap_try(at(1)),
                "KEY" => rules.key = at(1).to_string(),
                "FULLSTRIP" => rules.full_strip = true,
                "CHECKSHARPS" => rules.check_sharps = true,
                directive @ ("FORBIDDENWORD" | "NOSUGGEST" | "SUBSTANDARD" | "KEEPCASE" | "ONLYINCOMPOUND" | "NEEDAFFIX" | "CIRCUMFIX") => {
                    let c = rules.one_flag(at(1)).map_err(|e| format!("affix dictionary: bad {directive} in `{line}`: {e}"))?;
                    match directive {
                        "FORBIDDENWORD" => rules.forbidden = Some(c),
                        "NOSUGGEST" => rules.no_suggest = Some(c),
                        "SUBSTANDARD" => rules.substandard = Some(c),
                        "KEEPCASE" => rules.keepcase = Some(c),
                        "ONLYINCOMPOUND" => rules.only_in_compound = Some(c),
                        "NEEDAFFIX" => rules.need_affix = Some(c),
                        _ => rules.circumfix = Some(c),
                    }
                }
                conv @ ("ICONV" | "OCONV") if f.len() == 3 => {
                    rules.push_conv(conv == "OCONV", at(1), at(2)).map_err(|e| format!("affix dictionary: {e}"))?;
                }
                "IGNORE" => rules.ignore = at(1).chars().collect(),
                "COMPOUNDMIN" => {
                    let min = at(1).parse::<usize>().map_err(|_| format!("affix dictionary: bad `{line}`"))?;
                    rules.compound_min = Some(min.clamp(1, MAX_COMPOUND_BYTES));
                }
                "COMPOUNDRULE" => {
                    if rules.compound_rules.len() >= MAX_COMPOUND_RULES {
                        return Err("affix dictionary: too many COMPOUNDRULE patterns".into());
                    }
                    rules.compound_rules.push(at(1).to_string());
                }
                d if compound::is_directive(d) => {
                    let mut cpd = std::mem::take(&mut rules.cpd);
                    let read = cpd.read(&f, &mut |x| rules.one_flag(x));
                    rules.cpd = cpd;
                    read.map_err(|e| format!("affix dictionary: bad `{line}`: {e}"))?;
                }
                "MAP" => rules.map.push(at(1).chars().collect()),
                "REP" => push_rep(&mut rules.rep, at(1), at(2)),
                kind @ ("PFX" | "SFX") => {
                    let flag = rules.one_flag(at(1)).map_err(|e| format!("affix dictionary: bad flag in `{line}`: {e}"))?;
                    if f.len() != 6 {
                        return Err(format!("affix dictionary: bad rule `{line}`"));
                    }
                    // Continuation classes ride on the appended field, as in the `.aff` source.
                    let (add, cont) = match at(4).split_once('/') {
                        Some((a, c)) => (zero_empty(a), c.to_string()),
                        None => (zero_empty(at(4)), String::new()),
                    };
                    let rule: RawRule = (flag, at(2) == "Y", zero_empty(at(3)), add, cont, at(5).to_string());
                    if kind == "SFX" {
                        rules.suffixes.push(rule);
                    } else {
                        rules.prefixes.push(rule);
                    }
                }
                "" => {}
                other => return Err(format!("affix dictionary: unknown line `{other}`")),
            }
        }
        let body = raw.get(words_at + 7..).unwrap_or_default();
        let mut stems: Vec<(String, String)> = Vec::new();
        let mut flag_text = FlagText::new(MAX_STEM_FLAGS);
        let mut cur: Vec<u8> = Vec::new();
        let mut i = 0;
        while let Some(&p) = body.get(i) {
            let rest = body.get(i + 1..).unwrap_or_default();
            let len = rest.iter().position(|&b| b == b'\n').ok_or("affix dictionary: truncated")?;
            let line = rest.get(..len).unwrap_or_default();
            i += len + 2;
            let p = p as usize;
            if p > cur.len() {
                return Err("affix dictionary: bad prefix".into());
            }
            let (tail, flags) = match line.iter().position(|&b| b == b'/') {
                Some(k) => (line.get(..k).unwrap_or_default(), line.get(k + 1..).unwrap_or_default()),
                None => (line, &[][..]),
            };
            cur.truncate(p);
            cur.extend_from_slice(tail);
            let word = std::str::from_utf8(&cur).map_err(|_| "affix dictionary: bad utf-8 in a stem")?;
            let flags = std::str::from_utf8(flags).map_err(|_| "affix dictionary: bad utf-8 in flags")?;
            if word.is_empty() || word.len() > MAX_WORD_BYTES {
                return Err("affix dictionary: bad stem".into());
            }
            flag_text.add(flags).map_err(|e| format!("affix dictionary: {e}"))?;
            stems.push((word.to_string(), flags.to_string()));
        }
        AffixDict::build(rules, stems, comments, MAX_FLAGS_PER_STEM, MAX_STEM_FLAGS)
    }
}
