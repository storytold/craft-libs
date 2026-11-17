//! A small JSON-with-comments editor that changes only the bytes it must.
//!
//! The text is tokenized (comments and trailing commas tolerated), parsed into a tree that keeps
//! every value's byte span, and edits are applied by splicing new text into the original. Comments,
//! formatting and key order elsewhere in the file are untouched. In strict mode comments and
//! trailing commas are rejected (for clients whose own parser is strict JSON).

use serde_json::{Map, Value};

/// Deepest nesting accepted (hostile files must not blow the stack).
const MAX_DEPTH: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Str,
    Scalar,
}

#[derive(Debug, Clone, Copy)]
struct Token {
    kind: Tok,
    start: usize,
    end: usize,
}

/// A parsed value with its span in the source.
#[derive(Debug, Clone)]
pub struct Node {
    pub start: usize,
    pub end: usize,
    pub kind: NodeKind,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Object { members: Vec<Member>, trailing_comma: bool },
    Array(Vec<Node>),
    Scalar(Value),
}

#[derive(Debug, Clone)]
pub struct Member {
    pub key: String,
    pub key_start: usize,
    pub value: Node,
}

/// A parsed document: the original text plus the tree.
#[derive(Debug, Clone)]
pub struct Doc {
    pub text: String,
    /// `None` for an empty (or whitespace/comment-only) file.
    pub root: Option<Node>,
    tokens: Vec<Token>,
}

fn tokenize(text: &str, strict: bool) -> Result<Vec<Token>, String> {
    let b = text.as_bytes();
    let mut i = 0usize;
    if text.starts_with('\u{feff}') {
        if strict {
            return Err("byte-order mark is not allowed".into());
        }
        i = 3;
    }
    let mut out = Vec::new();
    while let Some(&c) = b.get(i) {
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                if strict {
                    return Err(format!("comment at byte {i} is not allowed in strict JSON"));
                }
                while let Some(&c) = b.get(i) {
                    if matches!(c, b'\r' | b'\n') {
                        break;
                    }
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                if strict {
                    return Err(format!("comment at byte {i} is not allowed in strict JSON"));
                }
                let start = i;
                i += 2;
                loop {
                    match b.get(i) {
                        None => return Err(format!("unterminated comment starting at byte {start}")),
                        Some(b'*') if b.get(i + 1) == Some(&b'/') => {
                            i += 2;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                let kind = match c {
                    b'{' => Tok::LBrace,
                    b'}' => Tok::RBrace,
                    b'[' => Tok::LBracket,
                    b']' => Tok::RBracket,
                    b':' => Tok::Colon,
                    _ => Tok::Comma,
                };
                out.push(Token { kind, start: i, end: i + 1 });
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                loop {
                    match b.get(i) {
                        None => return Err(format!("unterminated string starting at byte {start}")),
                        Some(b'\\') => i += 2,
                        Some(b'"') => {
                            i += 1;
                            break;
                        }
                        Some(b'\n') => return Err(format!("line break inside string at byte {i}")),
                        Some(_) => i += 1,
                    }
                }
                out.push(Token { kind: Tok::Str, start, end: i });
            }
            c if c == b'-' || c.is_ascii_alphanumeric() || c == b'.' || c == b'+' => {
                let start = i;
                while let Some(&c) = b.get(i) {
                    if c == b'-' || c.is_ascii_alphanumeric() || c == b'.' || c == b'+' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(Token { kind: Tok::Scalar, start, end: i });
            }
            _ => return Err(format!("unexpected character at byte {i}")),
        }
    }
    Ok(out)
}

struct Parser<'a> {
    text: &'a str,
    toks: &'a [Token],
    pos: usize,
    strict: bool,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Token> {
        self.toks.get(self.pos).copied()
    }

    fn next(&mut self) -> Result<Token, String> {
        let t = self.peek().ok_or_else(|| "unexpected end of file".to_string())?;
        self.pos += 1;
        Ok(t)
    }

    fn slice(&self, t: Token) -> &str {
        self.text.get(t.start..t.end).unwrap_or_default()
    }

    fn value(&mut self, depth: usize) -> Result<Node, String> {
        if depth > MAX_DEPTH {
            return Err("nested too deeply".into());
        }
        let t = self.next()?;
        match t.kind {
            Tok::LBrace => {
                let mut members = Vec::new();
                let mut trailing_comma = false;
                loop {
                    let n = self.peek().ok_or("unterminated object")?;
                    if n.kind == Tok::RBrace {
                        self.pos += 1;
                        if trailing_comma && self.strict {
                            return Err(format!("trailing comma before byte {}", n.start));
                        }
                        return Ok(Node { start: t.start, end: n.end, kind: NodeKind::Object { members, trailing_comma } });
                    }
                    if !members.is_empty() && !trailing_comma {
                        return Err(format!("expected , or }} at byte {}", n.start));
                    }
                    let k = self.next()?;
                    if k.kind != Tok::Str {
                        return Err(format!("expected a key at byte {}", k.start));
                    }
                    let key: String = serde_json::from_str(self.slice(k)).map_err(|e| format!("bad key at byte {}: {e}", k.start))?;
                    let c = self.next()?;
                    if c.kind != Tok::Colon {
                        return Err(format!("expected : at byte {}", c.start));
                    }
                    let value = self.value(depth + 1)?;
                    members.push(Member { key, key_start: k.start, value });
                    trailing_comma = false;
                    if self.peek().is_some_and(|t| t.kind == Tok::Comma) {
                        self.pos += 1;
                        trailing_comma = true;
                    }
                }
            }
            Tok::LBracket => {
                let mut items = Vec::new();
                let mut comma = false;
                loop {
                    let n = self.peek().ok_or("unterminated array")?;
                    if n.kind == Tok::RBracket {
                        self.pos += 1;
                        if comma && self.strict {
                            return Err(format!("trailing comma before byte {}", n.start));
                        }
                        return Ok(Node { start: t.start, end: n.end, kind: NodeKind::Array(items) });
                    }
                    if !items.is_empty() && !comma {
                        return Err(format!("expected , or ] at byte {}", n.start));
                    }
                    items.push(self.value(depth + 1)?);
                    comma = false;
                    if self.peek().is_some_and(|t| t.kind == Tok::Comma) {
                        self.pos += 1;
                        comma = true;
                    }
                }
            }
            Tok::Str | Tok::Scalar => {
                let v: Value = serde_json::from_str(self.slice(t)).map_err(|_| format!("bad value at byte {}", t.start))?;
                Ok(Node { start: t.start, end: t.end, kind: NodeKind::Scalar(v) })
            }
            _ => Err(format!("unexpected token at byte {}", t.start)),
        }
    }
}

impl Node {
    pub fn to_value(&self) -> Value {
        match &self.kind {
            NodeKind::Scalar(v) => v.clone(),
            NodeKind::Array(items) => Value::Array(items.iter().map(Node::to_value).collect()),
            NodeKind::Object { members, .. } => {
                let mut m = Map::new();
                for mem in members {
                    m.insert(mem.key.clone(), mem.value.to_value());
                }
                Value::Object(m)
            }
        }
    }

    fn member(&self, key: &str) -> Option<&Member> {
        match &self.kind {
            NodeKind::Object { members, .. } => members.iter().rev().find(|m| m.key == key),
            _ => None,
        }
    }
}

/// Parse `text`. `strict` rejects comments, trailing commas and a BOM.
pub fn parse(text: &str, strict: bool) -> Result<Doc, String> {
    let tokens = tokenize(text, strict)?;
    if tokens.is_empty() {
        return Ok(Doc { text: text.to_string(), root: None, tokens });
    }
    let mut p = Parser { text, toks: &tokens, pos: 0, strict };
    let root = p.value(0)?;
    if p.pos != tokens.len() {
        return Err("extra content after the top-level value".into());
    }
    if !matches!(root.kind, NodeKind::Object { .. }) {
        return Err("the top level must be an object".into());
    }
    Ok(Doc { text: text.to_string(), root: Some(root), tokens })
}

impl Doc {
    pub fn to_value(&self) -> Value {
        self.root.as_ref().map(Node::to_value).unwrap_or_else(|| Value::Object(Map::new()))
    }

    /// The value at `path` (object keys only).
    pub fn get(&self, path: &[&str]) -> Option<Value> {
        let mut node = self.root.as_ref()?;
        for k in path {
            node = &node.member(k)?.value;
        }
        Some(node.to_value())
    }

    fn line_start(&self, pos: usize) -> usize {
        self.text.get(..pos).and_then(|s| s.rfind(['\r', '\n'])).map(|i| i + 1).unwrap_or(0)
    }

    fn indent_at(&self, pos: usize) -> String {
        let ls = self.line_start(pos);
        self.text.get(ls..).unwrap_or_default().chars().take_while(|c| *c == ' ' || *c == '\t').collect()
    }

    /// True when only whitespace precedes `pos` on its line.
    fn first_on_line(&self, pos: usize) -> bool {
        let ls = self.line_start(pos);
        self.text.get(ls..pos).is_some_and(|s| s.trim().is_empty())
    }

    /// Set `path` to `value`, creating intermediate objects. Returns the new text.
    pub fn set(&self, path: &[&str], value: &Value) -> Result<String, String> {
        let Some((last, parents)) = path.split_last() else { return Err("empty path".into()) };
        let Some(root) = &self.root else {
            // No object yet: retain comments, whitespace and an optional BOM before the new one.
            let mut v = value.clone();
            for k in path.iter().rev() {
                let mut m = Map::new();
                m.insert((*k).to_string(), v);
                v = Value::Object(m);
            }
            let mut s = self.text.clone();
            let eol = self.eol();
            if !s.is_empty() && !s.ends_with(['\r', '\n']) {
                s.push_str(eol);
            }
            s.push_str(&render(&v, "", eol));
            s.push_str(eol);
            return Ok(s);
        };
        // Walk down as far as the path exists.
        let mut node = root;
        let mut depth = 0;
        for k in parents {
            match node.member(k) {
                Some(m) if matches!(m.value.kind, NodeKind::Object { .. }) => {
                    node = &m.value;
                    depth += 1;
                }
                Some(_) => return Err(format!("\"{k}\" exists but is not an object")),
                None => break,
            }
        }
        if depth == parents.len()
            && let Some(m) = node.member(last)
        {
            // Replace the existing value in place.
            let indent = self.indent_at(m.key_start);
            let rendered = render(value, &indent, self.eol());
            return Ok(splice(&self.text, &[(m.value.start, m.value.end, rendered)]));
        }
        // Insert a new member (holding the rest of the path) into `node`.
        let rest = path.get(depth..).unwrap_or_default();
        let Some((first, tail)) = rest.split_first() else { return Err("empty path".into()) };
        let mut v = value.clone();
        for k in tail.iter().rev() {
            let mut m = Map::new();
            m.insert((*k).to_string(), v);
            v = Value::Object(m);
        }
        self.insert_member(node, first, &v)
    }

    /// Preserve CRLF and CR-only documents; otherwise use LF.
    fn eol(&self) -> &'static str {
        if self.text.contains("\r\n") {
            "\r\n"
        } else if self.text.contains('\r') && !self.text.contains('\n') {
            "\r"
        } else {
            "\n"
        }
    }

    /// Add `key: value` as the last member of `obj`, indented like the members already there.
    fn insert_member(&self, obj: &Node, key: &str, value: &Value) -> Result<String, String> {
        let NodeKind::Object { members, trailing_comma } = &obj.kind else { return Err("not an object".into()) };
        let close = obj.end.saturating_sub(1);
        let close_indent = self.indent_at(obj.start);
        let indent = match members.first() {
            Some(m) if self.first_on_line(m.key_start) => self.indent_at(m.key_start),
            _ => format!("{close_indent}  "),
        };
        let eol = self.eol();
        let key_json = serde_json::to_string(key).map_err(|e| e.to_string())?;
        // An object whose members all end in a comma gets a new member that does too.
        let comma = if *trailing_comma { "," } else { "" };
        let member = format!("{indent}{key_json}: {}{comma}", render(value, &indent, eol));
        let mut edits = Vec::new();
        if let Some(last) = members.last()
            && !trailing_comma
        {
            edits.push((last.value.end, last.value.end, ",".to_string()));
        }
        if self.first_on_line(close) && !members.is_empty() {
            let ls = self.line_start(close);
            edits.push((ls, ls, format!("{member}{eol}")));
        } else {
            // `{}` or `{ "a": 1 }` on one line: open it up.
            let inner_end = close;
            let before = self.text.get(..inner_end).unwrap_or_default();
            let trimmed_end = before.trim_end().len().max(obj.start + 1);
            edits.push((trimmed_end, inner_end, format!("{eol}{member}{eol}{close_indent}")));
        }
        Ok(splice(&self.text, &edits))
    }

    /// Remove every member at `path`. Returns the new text, or `None` if it was not there.
    pub fn remove(&self, path: &[&str]) -> Option<String> {
        let (last, parents) = path.split_last()?;
        let mut node = self.root.as_ref()?;
        for k in parents {
            node = &node.member(k)?.value;
        }
        let NodeKind::Object { members, .. } = &node.kind else { return None };
        let mut edits = Vec::new();
        // Remove members separately so comments between duplicates survive.
        // A final run's leading comma is a separate edit for the same reason.
        for run in members.split(|m| m.key != *last) {
            let Some(first) = run.first() else { continue };
            let before_idx = self.tokens.partition_point(|t| t.end <= first.key_start);
            let before = before_idx.checked_sub(1).and_then(|i| self.tokens.get(i)).copied().filter(|t| t.kind == Tok::Comma);
            for member in run {
                let after_idx = self.tokens.partition_point(|t| t.start < member.value.end);
                let after = self.tokens.get(after_idx).copied().filter(|t| t.kind == Tok::Comma);
                let with_comma = after.is_none_or(|c| self.text.get(member.value.end..c.start).is_some_and(|gap| gap.trim().is_empty()));
                let member_end = if with_comma { after.map(|t| t.end).unwrap_or(member.value.end) } else { member.value.end };
                let rest = self.text.get(member_end..).unwrap_or_default();
                let line_rest = rest.split(['\r', '\n']).next().unwrap_or_default();
                let own_lines = self.first_on_line(member.key_start) && line_rest.trim().is_empty();
                let (start, end) = if own_lines {
                    let line_end = member_end + line_rest.len();
                    let eol_len = if self.text.get(line_end..).is_some_and(|s| s.starts_with("\r\n")) { 2 } else { usize::from(rest.len() > line_rest.len()) };
                    (self.line_start(member.key_start), line_end + eol_len)
                } else {
                    (member.key_start, member_end)
                };
                edits.push((start, end, String::new()));
                if !with_comma && let Some(c) = after {
                    edits.push((c.start, c.end, String::new()));
                }
                // The last member had no comma after it, so the one before it goes: leaving it
                // would add a trailing comma the file didn't have, which plain JSON readers refuse.
                if after.is_none()
                    && let Some(c) = before
                {
                    edits.push((c.start, c.end, String::new()));
                }
            }
        }
        (!edits.is_empty()).then(|| splice(&self.text, &edits))
    }
}

/// Pretty-print `value` with continuation lines indented by `indent`.
fn render(value: &Value, indent: &str, eol: &str) -> String {
    let s = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into());
    let mut out = String::new();
    for (i, line) in s.lines().enumerate() {
        if i > 0 {
            out.push_str(eol);
            out.push_str(indent);
        }
        out.push_str(line);
    }
    out
}

/// Apply non-overlapping edits `(start, end, replacement)`.
fn splice(text: &str, edits: &[(usize, usize, String)]) -> String {
    let mut sorted: Vec<&(usize, usize, String)> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.0, e.1));
    let mut out = String::with_capacity(text.len() + 256);
    let mut pos = 0;
    for (s, e, r) in sorted {
        if *s < pos {
            continue;
        }
        out.push_str(text.get(pos..*s).unwrap_or_default());
        out.push_str(r);
        pos = *e;
    }
    out.push_str(text.get(pos..).unwrap_or_default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: &str = r#"// user settings
{
  /* theme */
  "theme": "dark", // trailing note
  "mcpServers": {
    "other": { "command": "x" }, // keep me
  },
}
"#;

    #[test]
    fn set_keeps_crlf_line_endings() {
        for src in ["{\r\n  \"a\": 1\r\n}\r\n", "{}\r\n"] {
            let d = parse(src, true).unwrap();
            let out = d.set(&["z"], &json!({"k": [1]})).unwrap();
            assert_eq!(out.matches('\n').count(), out.matches("\r\n").count(), "mixed line endings: {out:?}");
            assert_eq!(parse(&out, true).unwrap().get(&["z", "k"]), Some(json!([1])));
        }
    }

    #[test]
    fn a_carriage_return_ends_a_line_comment() {
        let text = "// editor settings\r{\r  \"keep\": 1, // explanation\r  \"port\": 2\r}\r";
        let doc = parse(text, false).unwrap();
        assert_eq!(doc.get(&["keep"]), Some(json!(1)));
        let updated = doc.set(&["port"], &json!(3)).unwrap();
        assert_eq!(updated, text.replace('2', "3"));
        assert_eq!(parse(&updated, false).unwrap().get(&["port"]), Some(json!(3)));
        assert!(parse(text, true).is_err());
    }

    #[test]
    fn parses_comments_and_trailing_commas() {
        let d = parse(FIXTURE, false).unwrap();
        assert_eq!(d.get(&["mcpServers", "other", "command"]), Some(json!("x")));
        assert!(parse(FIXTURE, true).is_err());
        assert!(parse("{\"a\": 1,}", true).is_err());
        assert!(parse("{\"a\": 1}", true).is_ok());
        assert!(parse("[1]", false).is_err());
        assert!(parse("{", false).is_err());
        assert!(parse("", false).unwrap().root.is_none());
    }

    #[test]
    fn insert_keeps_comments_and_reparses() {
        let d = parse(FIXTURE, false).unwrap();
        let new = d.set(&["mcpServers", "craft-anything"], &json!({"command": "/x/craft-anything-cli", "args": ["mcp"]})).unwrap();
        assert!(new.contains("// keep me") && new.contains("/* theme */") && new.contains("// user settings"));
        let d2 = parse(&new, false).unwrap();
        assert_eq!(d2.get(&["mcpServers", "craft-anything", "args"]), Some(json!(["mcp"])));
        assert_eq!(d2.get(&["mcpServers", "other", "command"]), Some(json!("x")));
        // replace in place
        let new2 = d2.set(&["mcpServers", "craft-anything"], &json!({"command": "y"})).unwrap();
        let d3 = parse(&new2, false).unwrap();
        assert_eq!(d3.get(&["mcpServers", "craft-anything"]), Some(json!({"command": "y"})));
        // remove restores the original exactly
        assert_eq!(d3.remove(&["mcpServers", "craft-anything"]).unwrap(), FIXTURE);
    }

    #[test]
    fn creates_missing_tables_and_handles_empty() {
        let d = parse("{}", true).unwrap();
        let new = d.set(&["mcp", "servers", "k"], &json!(1)).unwrap();
        assert_eq!(parse(&new, true).unwrap().get(&["mcp", "servers", "k"]), Some(json!(1)));
        let d = parse("{\n  \"a\": 1\n}\n", true).unwrap();
        let new = d.set(&["s", "k"], &json!(true)).unwrap();
        assert_eq!(new, "{\n  \"a\": 1,\n  \"s\": {\n    \"k\": true\n  }\n}\n");
        let empty = parse("  \n", false).unwrap().set(&["s", "k"], &json!(2)).unwrap();
        assert_eq!(parse(&empty, true).unwrap().get(&["s", "k"]), Some(json!(2)));
        let one_line = parse(r#"{"a": {"b": 1}}"#, true).unwrap().set(&["a", "c"], &json!(2)).unwrap();
        assert_eq!(parse(&one_line, true).unwrap().to_value(), json!({"a": {"b": 1, "c": 2}}));
    }

    #[test]
    fn remove_first_middle_last() {
        let text = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}\n";
        let d = parse(text, true).unwrap();
        for (k, rest) in [("a", json!({"b":2,"c":3})), ("b", json!({"a":1,"c":3})), ("c", json!({"a":1,"b":2}))] {
            let t = d.remove(&[k]).unwrap();
            assert_eq!(parse(&t, true).unwrap().to_value(), rest, "{t}");
        }
        assert!(d.remove(&["zz"]).is_none());
        let single = parse("{\"a\": 1}", true).unwrap().remove(&["a"]).unwrap();
        assert_eq!(parse(&single, true).unwrap().to_value(), json!({}));
    }

    #[test]
    fn hostile_input_never_panics() {
        for s in ["{\"a\":", "{\"a\" 1}", "{,}", "\"\\", "/*", "{\"a\":[1,,2]}", "{\"a\":1}}", &"[".repeat(1000), "\u{feff}{}"] {
            let _ = parse(s, false);
            let _ = parse(s, true);
        }
        assert!(parse("\u{feff}{}", false).is_ok());
    }
}
