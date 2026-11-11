//! Front matter: optional `key: value` lines between `---` fences at the top of a file.

/// A file's front matter and its body (trimmed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed<'a> {
    pub fields: Vec<(&'a str, &'a str)>,
    pub body: &'a str,
}

impl Parsed<'_> {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}

/// Splits `text` into front matter and body. `allowed` lists the keys a file may use.
pub fn parse<'a>(text: &'a str, allowed: &[&str]) -> Result<Parsed<'a>, String> {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        return Ok(Parsed { fields: Vec::new(), body: text.trim() });
    };
    let mut fields = Vec::new();
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let line = line.trim_end();
        if line == "---" {
            let body = rest.get(offset..).unwrap_or_default().trim();
            return Ok(Parsed { fields, body });
        }
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once(':').ok_or_else(|| format!("front matter line {line:?} isn't `key: value`"))?;
        let key = key.trim();
        if !allowed.contains(&key) {
            return Err(format!("unknown front matter key `{key}`; allowed: {}", allowed.join(", ")));
        }
        if fields.iter().any(|(k, _)| *k == key) {
            return Err(format!("front matter key `{key}` appears twice"));
        }
        fields.push((key, value.trim()));
    }
    Err("front matter has no closing `---`".into())
}
