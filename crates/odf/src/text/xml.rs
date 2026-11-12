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

/// Stream XML events (empty elements become start + end). Malformed XML ends the stream.
pub(super) fn events(xml: &[u8], mut f: impl FnMut(Ev)) {
    events_at(xml, |ev, _| f(ev));
}

/// [`events`], each with the byte range of `xml` its markup takes (an empty element's start
/// and end both take the whole element).
pub(super) fn events_at(xml: &[u8], mut f: impl FnMut(Ev, (usize, usize))) {
    // Names and attributes as written; no limit but the input's own size (each part is at most a
    // package entry), and at most 128 attributes kept per element.
    let limits =
        craft_xml::Limits { max_input: usize::MAX, max_depth: usize::MAX, max_nodes: usize::MAX, max_attrs: usize::MAX, max_attrs_total: usize::MAX };
    let opts = craft_xml::Options::default().with_limits(limits);
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
