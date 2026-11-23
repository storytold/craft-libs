//! Compatibility stylesheet subset for simple SVG importers, extracted from EffectCraft.
//!
//! Supports compound tag/class/id selectors and comma lists. Unsupported selectors/at-rules
//! are skipped; malformed trailing rules keep the accepted prefix. This deliberately is not
//! the richer `paths` cascade or a full CSS parser. Property values are retained as text, so
//! caller colour precision and geometry policy are unaffected. `!important` is stripped as in
//! the source importer; it does not change precedence here.
//!
//! Call `parse` for each style element, then `StyleSheet::index` before matching. Discard a
//! sheet after a parse error. Matching charges the caller's aggregate budget before applying
//! any declarations. The caller must stop using results when its budget callback refuses.
use std::collections::HashMap;

/// Aggregate stylesheet byte bound before copying or allocating declaration strings.
pub const MAX_CSS_BYTES: usize = 64 * 1024 * 1024;

/// A stylesheet exceeds its byte, selector or declaration limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}

/// The most selectors (each entry of a comma list counts) the `<style>` sheets of a document may
/// hold (#2140).
pub const MAX_CSS_SELECTORS: usize = 20_000;
/// The most declarations the `<style>` sheets of a document may hold, each block counted once.
pub const MAX_CSS_DECLARATIONS: usize = 50_000;

/// The most matching work (candidate rules checked plus declarations applied) the elements of a
/// document may cost, `<use>` copies included: a bound on elements x rules x declarations.
pub const MAX_CSS_WORK: usize = 50_000_000;

/// A CSS rule: selector (type, class, id; `*`) with its specificity and declaration block.
#[derive(Clone, Debug)]
struct Rule {
    tag: Option<String>,
    classes: Vec<String>,
    id: Option<String>,
    specificity: u32,
    order: usize,
    /// Index into [`StyleSheet::blocks`]: every selector of a comma list shares one block.
    block: usize,
}

/// The rules of a document's `<style>` sheets, indexed by the part of the selector an element
/// must have (id, else first class, else type) so an element only checks rules that can match.
#[derive(Default, Debug)]
pub struct StyleSheet {
    /// Distinct selector rules in source order.
    rules: Vec<Rule>,
    /// Aggregate input bytes across all parsed style elements.
    bytes: usize,
    /// Declaration blocks; comma-list selectors share one block.
    blocks: Vec<Vec<(String, String)>>,
    /// Declarations over all blocks.
    decls: usize,
    /// Rules keyed by their id selector.
    by_id: HashMap<String, Vec<usize>>,
    /// Rules keyed by their first class selector.
    by_class: HashMap<String, Vec<usize>>,
    /// Rules keyed by their type selector.
    by_tag: HashMap<String, Vec<usize>>,
    /// Rules with no id, class or type to index on.
    universal: Vec<usize>,
}

impl StyleSheet {
    /// Build the id, class, tag and universal lookup buckets from `rules`.
    pub fn index(&mut self) {
        self.by_id.clear();
        self.by_class.clear();
        self.by_tag.clear();
        self.universal.clear();
        for (i, r) in self.rules.iter().enumerate() {
            let bucket = if let Some(id) = &r.id {
                self.by_id.entry(id.clone()).or_default()
            } else if let Some(c) = r.classes.first() {
                self.by_class.entry(c.clone()).or_default()
            } else if let Some(t) = &r.tag {
                self.by_tag.entry(t.clone()).or_default()
            } else {
                &mut self.universal
            };
            bucket.push(i);
        }
    }
}

/// Iterate a caller-bounded inline declaration list, preserving the compatibility grammar.
pub fn declarations(s: &str) -> impl Iterator<Item = (String, String)> + '_ {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let v = v.trim().trim_end_matches("!important").trim();
            Some((k.trim().to_ascii_lowercase(), v.to_string()))
        })
        .filter(|(k, _)| !k.is_empty())
}

/// Escape `s` as a CSS string delimited by `quote`.
///
/// Escapes backslashes, the delimiter quote, control characters, and `<` / `>` (which could end
/// a `<style>` element) using CSS hex escapes.
pub fn quote_css_string(s: &str, quote: char) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() || matches!(c, '<' | '>') => {
                let _ = write!(out, "\\{:x} ", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

fn strip_comments(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Splits one simple selector (`tag.class#id`) into its tag, classes and id.
fn parse_compound(one: &str) -> (Option<String>, Vec<String>, Option<String>) {
    let mut tag = None;
    let mut classes = vec![];
    let mut id = None;
    let mut cur = String::new();
    let mut kind = 't';
    let flush = |kind: char, cur: &mut String, tag: &mut Option<String>, classes: &mut Vec<String>, id: &mut Option<String>| {
        if !cur.is_empty() {
            match kind {
                '.' => classes.push(std::mem::take(cur)),
                '#' => *id = Some(std::mem::take(cur)),
                _ => {
                    if cur != "*" {
                        *tag = Some(std::mem::take(cur));
                    }
                    cur.clear();
                }
            }
        }
    };
    for c in one.chars() {
        if c == '.' || c == '#' {
            flush(kind, &mut cur, &mut tag, &mut classes, &mut id);
            kind = c;
        } else {
            cur.push(c);
        }
    }
    flush(kind, &mut cur, &mut tag, &mut classes, &mut id);
    (tag, classes, id)
}

/// Add the rules of one `<style>` sheet. Errs once the sheets exceed [`MAX_CSS_SELECTORS`] or
/// [`MAX_CSS_DECLARATIONS`] (#2140).
pub fn parse(css: &str, sheet: &mut StyleSheet) -> Result<(), Error> {
    sheet.bytes = sheet.bytes.saturating_add(css.len());
    if sheet.bytes > MAX_CSS_BYTES {
        return Err(Error("stylesheet exceeds the byte limit".to_owned()));
    }
    let css = strip_comments(css);
    let mut rest = css.as_str();
    while let Some(open) = rest.find('{') {
        let sel = rest[..open].trim();
        if sel.starts_with('@') {
            // Skip the whole at-rule block, matching nested braces by depth.
            let mut depth = 0usize;
            let mut end = None;
            for (i, c) in rest[open..].char_indices() {
                match c {
                    '{' => depth = depth.saturating_add(1),
                    '}' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            end = Some(open + i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(end) = end else { break };
            rest = &rest[end..];
            continue;
        }
        let Some(close) = rest[open..].find('}') else { break };
        let body = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        let mut decls: Vec<_> = declarations(body).take(MAX_CSS_DECLARATIONS + 1).collect();
        sheet.decls = sheet.decls.saturating_add(decls.len());
        if sheet.decls > MAX_CSS_DECLARATIONS {
            return Err(Error(format!("its <style> sheets hold more than {MAX_CSS_DECLARATIONS} declarations")));
        }
        // Stored with the rule's first accepted selector: a rule of unsupported selectors only
        // (`a b{}`, repeated) adds no block.
        let mut block = None;
        for one in sel.split(',') {
            let one = one.trim();
            // Only simple selectors (no combinators / pseudo-classes / attributes).
            if one.is_empty() || one.contains([' ', '>', '+', '~', ':', '[']) {
                continue;
            }
            if sheet.rules.len() >= MAX_CSS_SELECTORS {
                return Err(Error(format!("its <style> sheets hold more than {MAX_CSS_SELECTORS} selectors")));
            }
            let block = *block.get_or_insert_with(|| {
                let i = sheet.blocks.len();
                sheet.blocks.push(std::mem::take(&mut decls));
                i
            });
            let (tag, classes, id) = parse_compound(one);
            let specificity = (id.is_some() as u32 * 100)
                .saturating_add(u32::try_from(classes.len()).unwrap_or(u32::MAX).saturating_mul(10))
                .saturating_add(tag.is_some() as u32);
            let order = sheet.rules.len();
            sheet.rules.push(Rule { tag, classes, id, specificity, order, block });
        }
    }
    Ok(())
}

impl StyleSheet {
    /// Apply the matching `<style>` rules to `out`, by specificity then source order.
    pub fn apply(&self, tag: &str, id: Option<&str>, class: &str, out: &mut HashMap<String, String>, mut charge: impl FnMut(usize) -> bool) {
        let css = self;
        if css.rules.is_empty() {
            return;
        }
        // Splitting and sorting the class list is work too: charge its length first.
        if !charge(class.len()) {
            return;
        }
        let mut classes: Vec<&str> = class.split_whitespace().collect();
        classes.sort_unstable();
        classes.dedup();
        // Each rule sits in exactly one bucket and the classes are distinct: no rule twice.
        let mut buckets: Vec<&Vec<usize>> = vec![&css.universal];
        buckets.extend(id.and_then(|i| css.by_id.get(i)));
        buckets.extend(classes.iter().filter_map(|c| css.by_class.get(*c)));
        buckets.extend(css.by_tag.get(tag));
        if !charge(buckets.iter().map(|b| b.len()).fold(0, usize::saturating_add)) {
            return;
        }
        let candidates = || buckets.iter().flat_map(|b| b.iter()).filter_map(|&i| css.rules.get(i));
        // A selector like `.z.z.z…` checks each of its classes: charge them too.
        if !charge(candidates().map(|r| r.classes.len()).fold(0, usize::saturating_add)) {
            return;
        }
        let mut matching: Vec<&Rule> = candidates()
            .filter(|r| {
                r.tag.as_deref().is_none_or(|t| t == tag)
                    && r.id.as_deref().is_none_or(|i| Some(i) == id)
                    && r.classes.iter().all(|c| classes.binary_search(&c.as_str()).is_ok())
            })
            .collect();
        matching.sort_by_key(|r| (r.specificity, r.order));
        let blocks: Vec<&Vec<(String, String)>> = matching.iter().filter_map(|r| css.blocks.get(r.block)).collect();
        if !charge(blocks.iter().map(|b| b.len()).fold(0, usize::saturating_add)) {
            return;
        }
        for (k, v) in blocks.into_iter().flatten() {
            out.insert(k.clone(), v.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_comma_list_stores_its_declarations_once() {
        let sels = vec!["rect"; 1000].join(",");
        let decls: String = (0..50).map(|i| format!("p{i}:v;")).collect();
        let mut sheet = StyleSheet::default();
        parse(&format!("{sels}{{{decls}}}"), &mut sheet).unwrap();
        assert_eq!(sheet.rules.len(), 1000);
        assert_eq!(sheet.blocks.len(), 1);
        assert_eq!(sheet.decls, 50);
    }

    #[test]
    fn rules_of_unsupported_selectors_store_no_blocks() {
        let mut sheet = StyleSheet::default();
        parse(&"a b{}".repeat(100_000), &mut sheet).unwrap();
        parse("a b{fill:red} a>b, rect{fill:blue} c:hover, d{fill:red}", &mut sheet).unwrap();
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.blocks.len(), 2);
        assert!(sheet.rules.iter().all(|r| sheet.blocks.get(r.block).is_some_and(|b| b.len() == 1)));
    }

    #[test]
    fn at_rule_blocks_are_skipped_by_brace_depth() {
        let mut sheet = StyleSheet::default();
        parse("@media print { .a{fill:#ff0000} .b{fill:#0000ff} } .c{fill:red}", &mut sheet).unwrap();
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].classes, vec!["c".to_string()]);
        let mut sheet = StyleSheet::default();
        parse("@media x { @supports y { .c{} } .d{} } .e{fill:red}", &mut sheet).unwrap();
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].classes, vec!["e".to_string()]);
    }
    #[test]
    fn specificity_order_and_duplicate_classes_match_once() {
        let mut sheet = StyleSheet::default();
        parse("*{fill:black} .a{fill:red} rect.a{fill:green} #x{fill:blue} #x{stroke:white}", &mut sheet).unwrap();
        sheet.index();
        // Rebuilding buckets is idempotent.
        sheet.index();
        let mut result = HashMap::new();
        let mut work = 0usize;
        sheet.apply("rect", Some("x"), "a a", &mut result, |n| {
            work += n;
            true
        });
        assert_eq!(result.get("fill").map(String::as_str), Some("blue"));
        assert_eq!(result.get("stroke").map(String::as_str), Some("white"));
        assert_eq!(work, 3 + 5 + 2 + 5);
    }

    #[test]
    fn refused_matching_budget_applies_nothing() {
        let mut sheet = StyleSheet::default();
        parse(".a{fill:red}", &mut sheet).unwrap();
        sheet.index();
        for refuse_at in 0..4 {
            let mut result = HashMap::new();
            let mut stage = 0;
            sheet.apply("rect", None, "a", &mut result, |_| {
                let allowed = stage != refuse_at;
                stage += 1;
                allowed
            });
            assert!(result.is_empty());
        }
    }

    #[test]
    fn stylesheet_limits_and_malformed_prefix_are_preserved() {
        let mut sheet = StyleSheet::default();
        parse("rect{fill:red} broken{", &mut sheet).unwrap();
        assert_eq!(sheet.rules.len(), 1);
        let mut sheet = StyleSheet::default();
        assert!(parse(&format!("{}{{fill:red}}", vec!["rect"; MAX_CSS_SELECTORS + 1].join(",")), &mut sheet).is_err());
        let mut sheet = StyleSheet::default();
        assert!(parse(&format!("rect{{{}}}", "fill:red;".repeat(MAX_CSS_DECLARATIONS + 1)), &mut sheet).is_err());
        let mut sheet = StyleSheet { bytes: MAX_CSS_BYTES, ..Default::default() };
        assert!(parse("x", &mut sheet).is_err());
    }

    #[test]
    fn css_string_quoting_escapes_delimiters_controls_and_tags() {
        assert_eq!(quote_css_string("a\"b\\c\n<d>", '"'), "\"a\\\"b\\\\c\\a \\3c d\\3e \"");
        assert_eq!(quote_css_string("Arial's", '\''), "'Arial\\'s'");
    }
}
