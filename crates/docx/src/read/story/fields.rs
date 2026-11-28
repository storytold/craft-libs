use super::{CharProps, FieldState, InlineObject, Item, MAX_FIELD_DEPTH, PB, Phase, Reader, StoryCtx};
use craft_wordmodel::link::hyperlink_target;

/// Enclosed characters (`eq \o\ac(○,字)`), with their style from the code's font sizes: Word
/// enlarges the symbol (bigger than the text, which keeps the field's size) or shrinks the text
/// (smaller than the symbol).
fn enclosure(f: &FieldState) -> Option<craft_wordmodel::eq::Enclosure> {
    use craft_wordmodel::eq::{EncloseShape, EncloseStyle, Enclosure};
    let mut e = Enclosure::parse(&f.instr)?;
    let size_at = |at: usize| f.sizes.iter().rev().find(|(start, _)| *start <= at).and_then(|(_, s)| *s);
    let sym = f.instr.char_indices().find(|(_, c)| EncloseShape::from_symbol(&c.to_string()) == Some(e.shape)).map(|(i, _)| i)?;
    let txt = e.text.chars().next().and_then(|c| f.instr.get(sym..)?.find(c).map(|i| i + sym)).unwrap_or(sym);
    let base = f.props.size;
    e.style = match (size_at(sym), size_at(txt)) {
        (Some(s), Some(t)) if s > t + 0.01 && base.is_none_or(|b| t >= b - 0.01) => EncloseStyle::Enlarge,
        (Some(s), None) if base.is_none_or(|b| s > b + 0.01) => EncloseStyle::Enlarge,
        _ => EncloseStyle::Shrink,
    };
    Some(e)
}

/// Fields kept as ranges: those whose result another program writes, with formatting and often
/// several paragraphs (TOCs and citation managers' `ADDIN` fields).
fn is_range_instr(instr: &str) -> bool {
    crate::read::is_toc(instr) || instr.trim_start().get(..5).is_some_and(|k| k.eq_ignore_ascii_case("ADDIN"))
}

impl Reader<'_> {
    // ---- fields ----

    pub(super) fn begin_field(&mut self, sc: &mut StoryCtx, props: CharProps, instr: String, locked: bool, dirty: bool, form: String) {
        if sc.fields.len() >= MAX_FIELD_DEPTH || sc.ignored_begins > 0 {
            sc.ignored_begins += 1;
            return;
        }
        // Fields nested deeper than the cap are ignored (their content flows to the enclosing field).
        sc.fields.push(FieldState {
            instr,
            phase: Phase::Instr,
            spilled: false,
            buf: Vec::new(),
            props,
            locked,
            dirty,
            range: false,
            sizes: Vec::new(),
            form,
        });
    }

    /// Whether the field below the innermost one is still collecting its code (a field nested
    /// in another's code contributes text to that code, never content).
    fn parent_in_instr(sc: &StoryCtx) -> bool {
        let n = sc.fields.len();
        n >= 2 && sc.fields.get(n - 2).is_some_and(|p| !p.spilled && !p.range && p.phase == Phase::Instr)
    }

    /// `separate`: the code is complete; a range field starts its result here.
    pub(super) fn separate_field(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        if sc.ignored_begins > 0 {
            return;
        }
        let nested_in_instr = Self::parent_in_instr(sc);
        let Some(f) = sc.fields.last_mut() else { return };
        if f.range {
            return;
        }
        f.phase = Phase::Result;
        if !f.spilled && !nested_in_instr && is_range_instr(&f.instr) {
            f.range = true;
            let (obj, props) = (InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, f.props.clone());
            self.emit_obj(sc, pb, obj, &props);
        }
    }

    pub(super) fn end_field(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        if sc.ignored_begins > 0 {
            sc.ignored_begins -= 1;
            return;
        }
        let nested_in_instr = Self::parent_in_instr(sc);
        let Some(f) = sc.fields.pop() else { return };
        if f.range {
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        if f.spilled {
            return;
        }
        if f.phase == Phase::Instr && !nested_in_instr && is_range_instr(&f.instr) {
            // A range field with no result yet.
            self.emit_obj(sc, pb, InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, &f.props);
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        // A `HYPERLINK` field is a link, as a `w:hyperlink` is: its result stays ordinary content
        // carrying the link, so a tab in it reaches its tab stop and the `PAGEREF` in a table of
        // contents entry (Word for the web writes them this way) stays a field of its own (#2174).
        if !nested_in_instr && let Some(target) = hyperlink_target(&f.instr) {
            let mut before = None;
            for (it, mut p) in f.buf {
                match &it {
                    // A content control's start keeps its own formatting and its end takes that
                    // of the piece before it, as they read back: a link is written inside a
                    // control, never round its start.
                    Item::Obj(o) if matches!(**o, InlineObject::ControlStart { .. }) => {}
                    Item::Obj(o) if matches!(**o, InlineObject::ControlEnd) => p.link = before,
                    _ => {
                        p.link.get_or_insert_with(|| target.clone());
                    }
                }
                before = p.link.clone();
                match it {
                    Item::Text(t) => self.emit_text(sc, pb, &t, &p),
                    Item::Obj(o) => self.emit_obj(sc, pb, *o, &p),
                }
            }
            return;
        }
        // A cached result holding a picture, chart or shape (`INCLUDEPICTURE`'s embedded
        // picture) can't be a text result: keep the field as a range around it so the picture
        // stays in the document and is saved again. The linked source is never read.
        // A tracked insertion or deletion inside the result (one the field itself isn't wrapped
        // in) keeps the field a range too: a cached text result has one revision at most, so
        // each piece stays an ordinary run with its own `w:ins` / `w:del` (#1037).
        let tracked_inside = f.buf.iter().any(|(_, p)| (p.ins, p.del) != (f.props.ins, f.props.del));
        // So does a content control holding text or an object: a text result can't keep the
        // control round its content. Markers alone (empty controls among them) never do.
        let mut control_depth = 0_usize;
        let controlled_result = f.form.is_empty()
            && f.buf.iter().any(|(item, _)| match item {
                Item::Obj(o) if matches!(**o, InlineObject::ControlStart { .. }) => {
                    control_depth += 1;
                    false
                }
                Item::Obj(o) if matches!(**o, InlineObject::ControlEnd) => {
                    control_depth = control_depth.saturating_sub(1);
                    false
                }
                Item::Text(text) => control_depth > 0 && !text.is_empty(),
                Item::Obj(o) => control_depth > 0 && !o.is_marker(),
            });
        if !nested_in_instr && (tracked_inside || controlled_result || f.buf.iter().any(|(it, _)| matches!(it, Item::Obj(o) if o.frame().is_some())))
        {
            self.emit_obj(sc, pb, InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, &f.props);
            for (it, p) in f.buf {
                match it {
                    Item::Text(t) => self.emit_text(sc, pb, &t, &p),
                    Item::Obj(o) => self.emit_obj(sc, pb, *o, &p),
                }
            }
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        let mut result = String::new();
        for (it, _) in &f.buf {
            match it {
                Item::Text(t) => result.push_str(t),
                Item::Obj(o) => result.push_str(o.plain_text()),
            }
        }
        let (instr, result) = match enclosure(&f) {
            // Word writes no result for enclosed characters: the text is their plain text.
            Some(e) => (e.instr(), if result.is_empty() { e.text } else { result }),
            None => (f.instr.trim().to_string(), result),
        };
        // The visible result owns the field's formatting; empty results keep the begin's.
        let props = f
            .buf
            .iter()
            .find_map(|(it, p)| match it {
                Item::Text(t) if !t.is_empty() => Some(p),
                Item::Obj(o) if !o.plain_text().is_empty() => Some(p),
                _ => None,
            })
            .unwrap_or(&f.props)
            .clone();
        let obj = InlineObject::Field { instr, result, locked: f.locked, dirty: f.dirty, code: false, form: f.form };
        if let Some(parent) = sc.fields.last_mut()
            && !parent.spilled
            && parent.phase == Phase::Instr
        {
            parent.instr.push_str(obj.plain_text());
            return;
        }
        self.emit_obj(sc, pb, obj, &props);
        // A text result can't hold markers where they were in it: they follow the field, in
        // order, so each keeps its partner and its place against the text outside the field.
        for (item, props) in f.buf {
            if let Item::Obj(o) = item
                && o.is_marker()
                && !matches!(*o, InlineObject::FieldStart { .. } | InlineObject::FieldEnd)
            {
                self.emit_obj(sc, pb, *o, &props);
            }
        }
    }

    /// At a paragraph end, open fields can't keep buffering: emit them (empty result) and
    /// let the rest of their result flow as ordinary text.
    pub(super) fn spill_fields(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        for f in sc.fields.iter_mut() {
            if f.spilled || f.range {
                continue;
            }
            f.spilled = true;
            let form = std::mem::take(&mut f.form);
            pb.push_obj(
                InlineObject::Field { instr: f.instr.trim().to_string(), result: String::new(), locked: f.locked, dirty: f.dirty, code: false, form },
                &f.props,
            );
            for (it, p) in f.buf.drain(..) {
                match it {
                    Item::Text(t) => pb.push_text(&t, &p),
                    Item::Obj(o) => pb.push_obj(*o, &p),
                }
            }
        }
    }
}
