/// Length → points.
pub(super) fn length(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|x| x.is_finite());
    if let Some(n) = v.strip_suffix("pt") {
        num(n)
    } else if let Some(n) = v.strip_suffix("in") {
        num(n).map(|x| x * 72.0)
    } else if let Some(n) = v.strip_suffix("cm") {
        num(n).map(|x| x * 72.0 / 2.54)
    } else if let Some(n) = v.strip_suffix("mm") {
        num(n).map(|x| x * 72.0 / 25.4)
    } else if let Some(n) = v.strip_suffix("pc") {
        num(n).map(|x| x * 12.0)
    } else if let Some(n) = v.strip_suffix("px") {
        num(n).map(|x| x * 0.75)
    } else {
        num(&v)
    }
}

pub(super) type Attrs = Vec<(String, String)>;

pub(super) fn get<'a>(a: &'a Attrs, k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

pub(super) fn local(q: &str) -> &str {
    q.rsplit(':').next().unwrap_or(q)
}

pub(super) enum Ev {
    Start(String, Attrs),
    End(String),
    Text(String),
}

/// No limit but the input's own size (each part is at most a package entry).
const LIMITS: craft_xml::Limits = craft_xml::Limits {
    max_input: usize::MAX,
    max_depth: usize::MAX,
    max_nodes: usize::MAX,
    max_elements: usize::MAX,
    max_attrs: usize::MAX,
    max_attrs_total: usize::MAX,
};

/// Stream XML events (empty elements become start + end). Malformed XML ends the stream.
pub(super) fn events(xml: &[u8], mut f: impl FnMut(Ev)) {
    events_at(xml, |ev, _| f(ev));
}

/// [`events`], each with the byte range of `xml` its markup takes (an empty element's start
/// and end both take the whole element).
pub(super) fn events_at(xml: &[u8], mut f: impl FnMut(Ev, (usize, usize))) {
    // Names and attributes as written, and at most 128 attributes kept per element.
    let opts = craft_xml::Options::default().with_limits(LIMITS);
    let mut events = opts.events(xml);
    // Malformed XML ends the stream.
    while let Ok(Some(ev)) = events.next() {
        let span = events.span();
        let at = (span.start, span.end);
        match ev {
            craft_xml::Event::Start { name, mut attrs } => {
                attrs.truncate(128);
                f(Ev::Start(name, attrs), at)
            }
            craft_xml::Event::End { name } => f(Ev::End(name), at),
            craft_xml::Event::Text(t) | craft_xml::Event::CData(t) => f(Ev::Text(t), at),
            _ => {}
        }
    }
}

/// Most events, and bytes, [`holds_words`] looks at: an element still open past them counts as
/// holding words (an empty one LibreOffice writes is a handful of events).
const LOOK_EVENTS: usize = 64;
const LOOK_BYTES: usize = 4096;

/// Whether the element whose start tag ends at `from` in `xml` holds any words before its end,
/// read as [`events`] reads them: text, CDATA or a reference that isn't only whitespace (comments
/// and processing instructions hold none). Any end at its own depth ends it, as in [`events`].
/// `true` when that can't be told within [`LOOK_EVENTS`] events and [`LOOK_BYTES`] bytes, or
/// the markup is malformed, so nothing in it is taken for empty unread.
pub(super) fn holds_words(xml: &[u8], from: usize) -> bool {
    let Some(rest) = xml.get(from..from.saturating_add(LOOK_BYTES).min(xml.len())) else { return true };
    // The reader skips an end tag with nothing open, so the contents are read inside a stand-in
    // start tag, which the element's own end (any end at its depth) closes.
    const OPEN: &[u8] = b"<_>";
    let mut wrapped = Vec::with_capacity(OPEN.len().saturating_add(rest.len()));
    wrapped.extend_from_slice(OPEN);
    wrapped.extend_from_slice(rest);
    // An unknown entity is an error rather than left out, so that it counts as words.
    let opts = craft_xml::Options { unknown_entity: craft_xml::UnknownEntity::Error, ..craft_xml::Options::default().with_limits(LIMITS) };
    let mut r = opts.events(&wrapped);
    if !matches!(r.next(), Ok(Some(craft_xml::Event::Start { .. }))) {
        return true;
    }
    let mut depth = 0usize;
    let mut looked = 0usize;
    // An empty element's start and end are one piece of markup (one span): counted once.
    let mut last = 0..0;
    while looked < LOOK_EVENTS {
        let ev = match r.next() {
            Ok(Some(ev)) => ev,
            Ok(None) | Err(_) => return true,
        };
        let span = r.span();
        if span != last {
            looked = looked.saturating_add(1);
        }
        let markup = wrapped.get(span.clone()).unwrap_or_default();
        last = span;
        match ev {
            craft_xml::Event::Start { .. } => depth = depth.saturating_add(1),
            craft_xml::Event::End { .. } if depth == 0 => return false,
            craft_xml::Event::End { .. } => depth = depth.saturating_sub(1),
            // A reference, resolved: words whatever it stands for.
            craft_xml::Event::Text(_) if markup.starts_with(b"&") => return true,
            craft_xml::Event::Text(t) | craft_xml::Event::CData(t) if t.bytes().any(|b| !b.is_ascii_whitespace()) => return true,
            _ => {}
        }
    }
    true
}
