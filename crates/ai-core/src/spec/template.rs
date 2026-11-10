//! The only substitutions a spec may make: `{prompt}`, `{model}` and `{job_id}`.
//!
//! The structure of a request always comes from the spec. A substitution replaces text inside a
//! string value of a JSON template, so a prompt can never add a field or change a key, and the
//! inserted text is not read again for more substitutions. `{{` and `}}` write a literal brace.

use serde_json::Value;

/// A value a template may name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Prompt,
    Model,
    JobId,
}

impl Slot {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "prompt" => Some(Self::Prompt),
            "model" => Some(Self::Model),
            "job_id" => Some(Self::JobId),
            _ => None,
        }
    }
}

/// The values for one call. A slot left `None` cannot be used by that call.
#[derive(Clone, Copy, Debug, Default)]
pub struct Values<'a> {
    pub prompt: Option<&'a str>,
    pub model: Option<&'a str>,
    pub job_id: Option<&'a str>,
}

impl Values<'_> {
    fn get(&self, slot: Slot) -> Option<&str> {
        match slot {
            Slot::Prompt => self.prompt,
            Slot::Model => self.model,
            Slot::JobId => self.job_id,
        }
    }
}

enum Piece {
    Text(String),
    Slot(Slot, String),
}

/// Splits `text` into literal text and slots, or says why its braces are wrong.
fn pieces(text: &str) -> Result<Vec<Piece>, String> {
    let mut out = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(['{', '}']) {
        literal.push_str(&rest[..i]);
        let here = &rest[i..];
        let after = &rest[i + 1..];
        if here.starts_with("{{") || here.starts_with("}}") {
            literal.push_str(&here[..1]);
            rest = &after[1..];
        } else if here.starts_with('}') {
            return Err("a lone } (write }} for a literal brace)".into());
        } else {
            let end = after.find('}').ok_or("a { that is never closed (write {{ for a literal brace)")?;
            let name = &after[..end];
            let slot = Slot::parse(name).ok_or_else(|| format!("unsupported: placeholder {{{name}}}"))?;
            if !literal.is_empty() {
                out.push(Piece::Text(std::mem::take(&mut literal)));
            }
            out.push(Piece::Slot(slot, name.to_owned()));
            rest = &after[end + 1..];
        }
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        out.push(Piece::Text(literal));
    }
    Ok(out)
}

/// The slots `text` uses, or why its braces are wrong.
pub fn slots(text: &str) -> Result<Vec<Slot>, String> {
    Ok(pieces(text)?
        .into_iter()
        .filter_map(|p| match p {
            Piece::Slot(slot, _) => Some(slot),
            Piece::Text(_) => None,
        })
        .collect())
}

/// Every slot used by a string value anywhere in `template`. Object keys must be literal.
pub fn value_slots(template: &Value) -> Result<Vec<Slot>, String> {
    let mut out = Vec::new();
    match template {
        Value::String(s) => out.extend(slots(s)?),
        Value::Array(items) => {
            for item in items {
                out.extend(value_slots(item)?);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if key.contains(['{', '}']) {
                    return Err(format!("a key cannot hold a brace ({key:?})"));
                }
                out.extend(value_slots(item)?);
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Renders one string. Fails past [`MAX_RENDERED_BYTES`](crate::spec::MAX_RENDERED_BYTES).
pub fn render_text(text: &str, values: &Values<'_>) -> Result<String, String> {
    let mut left = crate::spec::MAX_RENDERED_BYTES;
    render_text_within(text, values, &mut left)
}

/// Renders `text`, taking its length from `left`; a template that repeats a slot cannot outgrow it.
fn render_text_within(text: &str, values: &Values<'_>, left: &mut usize) -> Result<String, String> {
    let mut out = String::new();
    for piece in pieces(text)? {
        let add = match &piece {
            Piece::Text(t) => t.as_str(),
            Piece::Slot(slot, name) => values.get(*slot).ok_or_else(|| format!("{{{name}}} is not available in this call"))?,
        };
        *left = left.checked_sub(add.len()).ok_or_else(|| format!("a rendered template is at most {} bytes", crate::spec::MAX_RENDERED_BYTES))?;
        out.push_str(add);
    }
    Ok(out)
}

/// Renders every string value of `template`. Keys are never substituted. All strings together
/// share one [`MAX_RENDERED_BYTES`](crate::spec::MAX_RENDERED_BYTES) budget.
pub fn render_value(template: &Value, values: &Values<'_>) -> Result<Value, String> {
    let mut left = crate::spec::MAX_RENDERED_BYTES;
    render_value_within(template, values, &mut left)
}

fn render_value_within(template: &Value, values: &Values<'_>, left: &mut usize) -> Result<Value, String> {
    Ok(match template {
        Value::String(s) => Value::String(render_text_within(s, values, left)?),
        Value::Array(items) => Value::Array(items.iter().map(|i| render_value_within(i, values, left)).collect::<Result<_, _>>()?),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| Ok((k.clone(), render_value_within(v, values, left)?))).collect::<Result<_, String>>()?)
        }
        other => other.clone(),
    })
}
