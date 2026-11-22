use super::drawing::{DrawingInfo, MemberInfo, nv_props, shape_name, spin_attrs, write_fill, xfrm_spin_only};
use super::field::{FieldData, field_run};
use super::section::comment_reference_style;
use super::{Anchor, CharProps, Fill, Float, InlineObject, MAX_DEPTH, NoteKind, PartRels, ShapeKind, TextColor, W, Wrap, Writer, emu, n, rpr, rt};

impl Writer<'_> {
    /// Write an inline object (`Bookmark`, `Comment`, `Equation`, `Field`, `Shape`, etc.).
    pub(super) fn object(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, depth: usize) {
        match o {
            // Written by `para_content` (inline) or `blocks` (around paragraphs).
            InlineObject::ControlStart { .. } | InlineObject::ControlEnd => {}
            InlineObject::BookmarkStart { name } => {
                let id = self.bookmark_id(name);
                w.empty("w:bookmarkStart", &[("w:id", &id), ("w:name", name)]);
            }
            InlineObject::BookmarkEnd { name } => {
                let id = self.bookmark_id(name);
                w.empty("w:bookmarkEnd", &[("w:id", &id)]);
            }
            InlineObject::PermStart { id, group, editor } => {
                let id = id.to_string();
                let mut a = vec![("w:id", id.as_str())];
                if !group.is_empty() {
                    a.push(("w:edGrp", group.as_str()));
                }
                if !editor.is_empty() {
                    a.push(("w:ed", editor.as_str()));
                }
                w.empty("w:permStart", &a);
            }
            InlineObject::PermEnd { id } => w.empty("w:permEnd", &[("w:id", &id.to_string())]),
            InlineObject::CommentStart { id } => {
                if self.doc.comments.contains_key(id) {
                    w.empty("w:commentRangeStart", &[("w:id", &id.to_string())]);
                }
            }
            InlineObject::CommentEnd { id } => {
                if self.doc.comments.contains_key(id) {
                    let s = id.to_string();
                    w.empty("w:commentRangeEnd", &[("w:id", &s)]);
                    w.open("w:r", &[]);
                    w.open("w:rPr", &[]);
                    w.val("w:rStyle", comment_reference_style(self.doc));
                    w.close("w:rPr");
                    w.empty("w:commentReference", &[("w:id", &s)]);
                    w.close("w:r");
                }
            }
            InlineObject::Equation { linear, display, math } => {
                // Automatic equation numbers become text: Word numbers nothing by itself.
                let color = match props.color {
                    Some(TextColor::Rgb(c)) => Some(c),
                    _ => None,
                };
                self.rev_open(w, props);
                match crate::write::math::resolve_numbers(math, *display, &mut self.eq_number) {
                    Some(m) => crate::write::math::write_equation(w, linear, *display, &m, color),
                    None => crate::write::math::write_equation(w, linear, *display, math, color),
                }
                self.rev_close(w, props);
            }
            InlineObject::Field { instr, result, locked, dirty, form, .. } => {
                self.field(w, FieldData { instr, result, locked: *locked, dirty: *dirty, form }, props)
            }
            InlineObject::FieldStart { instr, locked, dirty } => {
                self.rev_open(w, props);
                self.field_start(w, instr, *locked, *dirty, "", props);
                self.rev_close(w, props);
            }
            InlineObject::FieldEnd => {
                self.rev_open(w, props);
                field_run(w, props, &|w| w.empty("w:fldChar", &[("w:fldCharType", "end")]));
                self.rev_close(w, props);
            }
            InlineObject::NoteRef { kind, id, custom } => {
                let foot = *kind == NoteKind::Footnote;
                if self.current_note == Some((foot, *id)) {
                    // The note's own number is the w:footnoteRef / w:endnoteRef mark, written once
                    // per note: here for its first reference to itself, never as a reference.
                    if let Some(mark) = self.pending_mark.take() {
                        let mut p = props.clone();
                        if p.style.is_none() && p.vert_align.is_none() {
                            p.vert_align = Some(craft_wordmodel::props::VertAlign::Superscript);
                        }
                        self.rev_open(w, props);
                        w.open("w:r", &[]);
                        self.rpr(w, &p);
                        // A custom mark is plain text; the placeholder would turn it into a number.
                        if custom.is_empty() {
                            w.empty(mark, &[]);
                        } else {
                            w.leaf("w:t", &[("xml:space", "preserve")], custom);
                        }
                        w.close("w:r");
                        self.rev_close(w, props);
                    }
                    return;
                }
                let nid = self.note_id(*id, foot);
                let mut p = props.clone();
                if p.style.is_none() && p.vert_align.is_none() {
                    p.vert_align = Some(craft_wordmodel::props::VertAlign::Superscript);
                }
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, &p);
                let tag = if foot { "w:footnoteReference" } else { "w:endnoteReference" };
                if custom.is_empty() {
                    w.empty(tag, &[("w:id", &nid)]);
                } else {
                    w.empty(tag, &[("w:customMarkFollows", "1"), ("w:id", &nid)]);
                    w.leaf("w:t", &[("xml:space", "preserve")], custom);
                }
                w.close("w:r");
                self.rev_close(w, props);
            }
            // Editable charts and SmartArt keep main's model writer; opaque graphics and native
            // ink keep their original source and related parts.
            InlineObject::Graphic { .. } => self.write_graphic(w, o, props, rels, depth),
            InlineObject::Image { .. } => self.write_image(w, o, props, rels, depth),
            InlineObject::Shape { extra, float, .. } if extra.hr.is_some() && float.wrap == Wrap::Inline && !float.hidden => {
                self.write_hr(w, o, props)
            }
            InlineObject::Shape { .. } => self.write_shape(w, o, props, rels, depth),
            InlineObject::Group { .. } => self.write_group(w, o, props, rels, depth),
            InlineObject::Ruby { base, ruby, align, size, raise, base_size, lang, props: ruby_props } => {
                // ECMA-376 §17.3.3.25: a run holding w:ruby (properties, ruby text, base text).
                let t = if self.is_del(props) { "w:delText" } else { "w:t" };
                let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
                let hps = |v: f32, min: f32| n(craft_wordmodel::geom::to_half_points(v.clamp(min, 1584.0)));
                let size = fin(*size, 5.0).clamp(1.0, 1584.0);
                let base_size = if base_size.is_finite() && *base_size > 0.0 { *base_size } else { props.size.unwrap_or(10.5) };
                self.rev_open(w, props);
                w.open("w:r", &[]);
                rpr(w, props);
                w.open("w:ruby", &[]);
                w.open("w:rubyPr", &[]);
                w.val("w:rubyAlign", align.ooxml());
                w.val("w:hps", &hps(size, 1.0));
                w.val("w:hpsRaise", &hps(fin(*raise, 0.0), 0.0));
                w.val("w:hpsBaseText", &hps(fin(base_size, 10.5), 1.0));
                w.val("w:lid", if lang.is_empty() { "ja-JP" } else { lang });
                w.close("w:rubyPr");
                let mut rt_props = props.clone().overlaid(ruby_props);
                rt_props.size = Some(size);
                for (tag, p, text) in [("w:rt", &rt_props, ruby), ("w:rubyBase", props, base)] {
                    w.open(tag, &[]);
                    w.open("w:r", &[]);
                    rpr(w, p);
                    w.leaf(t, &[("xml:space", "preserve")], text);
                    w.close("w:r");
                    w.close(tag);
                }
                w.close("w:ruby");
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::PositionalTab { align, relative_to, leader } => {
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, props);
                w.empty(
                    "w:ptab",
                    &[("w:relativeTo", relative_to.ooxml()), ("w:alignment", align.ooxml()), ("w:leader", crate::write::props::tab_leader(*leader))],
                );
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Opaque { format, xml, text } => {
                if format == "docx" && crate::xml::parse(xml.as_bytes()).is_ok() {
                    w.raw(xml);
                } else if !text.is_empty() {
                    self.rev_open(w, props);
                    w.open("w:r", &[]);
                    self.rpr(w, props);
                    w.leaf("w:t", &[("xml:space", "preserve")], text);
                    w.close("w:r");
                    self.rev_close(w, props);
                }
            }
        }
    }

    fn write_graphic(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, _depth: usize) {
        let InlineObject::Graphic { w: gw, h: gh, alt, float, graphic } = o else { return };
        let ink = graphic.kind == craft_wordmodel::graphic::GraphicKind::Ink;
        let root = match (graphic.chart.as_deref(), graphic.smart_art.as_deref(), graphic.source.as_deref()) {
            (Some(spec), _, _) => crate::xml::parse(self.chart_graphic(spec, rels).as_bytes()).ok(),
            (None, Some(spec), _) => crate::xml::parse(self.smart_art_graphic(spec, *gw, *gh, rels).as_bytes()).ok(),
            // Ink in a group member's form (no drawing of its own) can't stand alone: left
            // out before its parts are queued, so no orphan part or relationship is written.
            (None, None, Some(src)) if ink && !standalone_ink(src) => {
                log::warn!("docx: group-member ink outside its group can't be written; it is left out");
                None
            }
            (None, None, Some(src)) => self.embedded_tree(src, rels, ink.then_some((*gw, *gh, float, alt.as_str()))),
            (None, None, None) => None,
        };
        let Some(mut root) = root else { return };
        let direct_ink = ink && root.name == "w:contentPart";
        if ink && !direct_ink {
            Self::ink_transform(&mut root, (graphic.w, graphic.h), (*gw, *gh), float, 0);
        }
        // Native ink, or an Office 2016 chart (`cx:chart`) with its fallback picture.
        let alternate = (root.name == "mc:AlternateContent").then_some(&root);
        let fallback = alternate.and_then(|e| e.child("mc:Fallback"));
        let choice = alternate.and_then(|e| e.child("mc:Choice"));
        let native = choice.and_then(|e| e.find("a:graphic"));
        if alternate.is_some() && native.is_none() {
            return;
        }
        // A chart's picture gets the chart's current frame too; ink's VML preview is resized in place.
        let picture = if ink { None } else { fallback.and_then(|e| e.find("a:graphic")) };
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        if direct_ink {
            w.raw(&root.to_xml());
        } else {
            if fallback.is_some() {
                let requires = if ink { "wpi" } else { choice.and_then(|c| c.attr("Requires")).unwrap_or_default() };
                w.open("mc:AlternateContent", &[]);
                w.open("mc:Choice", &[("Requires", requires)]);
            }
            let docpr = self.next_docpr();
            let link = Self::drawing_link(props, rels);
            let name = match graphic.kind {
                craft_wordmodel::graphic::GraphicKind::Chart => format!("Chart {docpr}"),
                craft_wordmodel::graphic::GraphicKind::Diagram => format!("Diagram {docpr}"),
                craft_wordmodel::graphic::GraphicKind::Ink => format!("Ink {docpr}"),
            };
            let z = self.z;
            self.graphic_frame(
                w,
                DrawingInfo { float, size: (*gw, *gh), docpr: &docpr, name: &name, alt, link: link.as_deref() },
                native.unwrap_or(&root),
            );
            if let Some(fallback) = fallback {
                w.close("mc:Choice");
                match picture {
                    Some(picture) => {
                        // Both branches are one object: same id and stacking order.
                        self.z = z;
                        w.open("mc:Fallback", &[]);
                        self.graphic_frame(
                            w,
                            DrawingInfo { float, size: (*gw, *gh), docpr: &docpr, name: &name, alt, link: link.as_deref() },
                            picture,
                        );
                        w.close("mc:Fallback");
                    }
                    None => w.raw(&fallback.to_xml()),
                }
                w.close("mc:AlternateContent");
            }
        }
        w.close("w:r");
        self.rev_close(w, props);
    }

    fn write_image(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, _depth: usize) {
        let InlineObject::Image { media, w: iw, h: ih, alt, float, ole, .. } = o else { return };
        // An OLE object read from a file: the object itself, at its current size.
        if let Some(src) = ole.as_deref().filter(|e| !e.is_odf_object())
            && let Some(obj) = self.embedded_xml(src, rels, Some((*iw, *ih, float, alt.as_str())))
        {
            self.rev_open(w, props);
            w.open("w:r", &[]);
            self.rpr(w, props);
            w.raw(&obj);
            w.close("w:r");
            self.rev_close(w, props);
            return;
        }
        let Some(file) = self.media_files.get(media).cloned() else { return };
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        w.open("w:drawing", &[]);
        let docpr = self.next_docpr();
        let link = Self::drawing_link(props, rels);
        let name = format!("Picture {docpr}");
        self.drawing_open(w, DrawingInfo { float, size: (*iw, *ih), docpr: &docpr, name: &name, alt, link: link.as_deref() });
        w.empty("wp:cNvGraphicFramePr", &[]);
        w.open("a:graphic", &[]);
        w.open("a:graphicData", &[("uri", "http://schemas.openxmlformats.org/drawingml/2006/picture")]);
        self.pic(w, o, &file, MemberInfo { id: None, off: (0.0, 0.0), link: None }, rels);
        w.close("a:graphicData");
        w.close("a:graphic");
        w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
        w.close("w:drawing");
        w.close("w:r");
        self.rev_close(w, props);
    }

    /// A horizontal rule goes back as the VML rule it was read from: DrawingML has none, and a
    /// fixed-size shape would lose its line-wide width.
    fn write_hr(&mut self, w: &mut W, o: &InlineObject, props: &CharProps) {
        let InlineObject::Shape { w: sw, h: sh, fill, stroke, stroke_width, extra, float, .. } = o else { return };
        let Some(hr) = extra.hr else { return };
        let align = match hr.align {
            craft_wordmodel::props::Align::Center => "center",
            craft_wordmodel::props::Align::Right => "right",
            _ => "left",
        };
        let width = if hr.pct > 0.0 { "0".to_string() } else { format!("{:.2}pt", sw.max(0.0)) };
        let mut style = format!("width:{width};height:{:.2}pt", sh.max(0.0));
        let spin = float.spin();
        if spin.deg != 0.0 {
            style.push_str(&format!(";rotation:{}", spin.deg));
        }
        match (spin.flip_h, spin.flip_v) {
            (true, true) => style.push_str(";flip:x y"),
            (true, false) => style.push_str(";flip:x"),
            (false, true) => style.push_str(";flip:y"),
            (false, false) => {}
        }
        let pct = format!("{}", (hr.pct * 10.0).round().max(0.0) as u32);
        let fill_hex = fill.as_ref().map(|f| format!("#{}", f.main_color().hex()));
        let stroke_hex = stroke.map(|c| format!("#{}", c.hex()));
        let weight = format!("{stroke_width:.2}pt");
        let mut attrs: Vec<(&str, &str)> = vec![
            ("xmlns:v", "urn:schemas-microsoft-com:vml"),
            ("xmlns:o", "urn:schemas-microsoft-com:office:office"),
            ("style", &style),
            ("o:hrpct", &pct),
            ("o:hralign", align),
            ("o:hr", "t"),
        ];
        if hr.standard {
            attrs.push(("o:hrstd", "t"));
        }
        if let Some(f) = &fill_hex {
            attrs.push(("fillcolor", f));
        } else {
            attrs.push(("filled", "f"));
        }
        match &stroke_hex {
            Some(s) => {
                attrs.push(("strokecolor", s));
                attrs.push(("strokeweight", &weight));
            }
            None => attrs.push(("stroked", "f")),
        }
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        w.open("w:pict", &[]);
        // A see-through fill keeps its opacity, which `fillcolor` can't say.
        match fill.as_ref() {
            Some(craft_wordmodel::fill::Fill::Solid { transparency, .. }) if *transparency > 0.0 => {
                let opacity = format!("{:.4}", (1.0 - transparency / 100.0).clamp(0.0, 1.0));
                w.open("v:rect", &attrs);
                w.empty("v:fill", &[("opacity", &opacity)]);
                w.close("v:rect");
            }
            _ => w.empty("v:rect", &attrs),
        }
        w.close("w:pict");
        w.close("w:r");
        self.rev_close(w, props);
    }

    fn write_shape(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, depth: usize) {
        let InlineObject::Shape { kind, w: sw, h: sh, float, effects, freeform, extra, story, .. } = o else { return };
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        // A Word 2010 shape (`wps`) with a VML form goes in `mc:AlternateContent`, as Word writes
        // it: a reader that doesn't know `wps` (Word 2007, the Office Compatibility Pack) takes the
        // VML fallback instead of refusing the file. A shape without one stays bare: guarded with
        // no fallback, that reader would open the file without the shape and drop it on its next
        // save, which is worse than refusing the file.
        // VML currently has no chain IDs or mso-next-textbox links. Keep every member of a
        // linked story bare, including its head: an unlinked fallback would silently lose
        // overflow text in older readers. Unlinked boxes still get the compatibility fallback.
        let linked = story.is_some_and(|id| self.txbx_links.contains_key(&id));
        let fallback = freeform.is_none() && vml_tag(*kind).is_some() && !linked;
        if fallback {
            w.open("mc:AlternateContent", &[]);
            w.open("mc:Choice", &[("Requires", "wps")]);
        }
        w.open("w:drawing", &[]);
        let docpr = self.shape_docpr(extra.id);
        let link = Self::drawing_link(props, rels);
        // Ink is told apart by its name (and its pen), which reading looks for.
        let name = shape_name(*kind, freeform.as_deref(), &docpr);
        // The effect extent leaves room for the shadow and glow.
        let mut float = float.clone();
        for (e, fx) in float.effect.iter_mut().zip(effects.extent()) {
            *e = craft_wordmodel::geom::finite(*e).max(fx);
        }
        // Tight and Through wrapping need a polygon: the shape's outline unless it has one.
        if matches!(float.wrap, Wrap::Tight | Wrap::Through) && float.wrap_polygon.is_none() {
            float.wrap_polygon = craft_wordmodel::wrap::shape_outline(*kind, *sw, *sh, extra.corner)
                .and_then(|pts| craft_wordmodel::wrap::WrapPolygon::from_unit(false, &pts))
                .map(std::sync::Arc::new);
        }
        let float = &float;
        self.drawing_open(w, DrawingInfo { float, size: (*sw, *sh), docpr: &docpr, name: &name, alt: o.alt_text(), link: link.as_deref() });
        // This shape's stacking order, as its anchor gave it: floating objects in its text box
        // write anchors of their own.
        let z = self.anchor_z;
        w.empty("wp:cNvGraphicFramePr", &[]);
        w.open("a:graphic", &[]);
        w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingShape")]);
        let content = self.wsp(w, o, MemberInfo { id: None, off: (0.0, 0.0), link: None }, rels, depth);
        w.close("a:graphicData");
        w.close("a:graphic");
        w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
        w.close("w:drawing");
        if fallback {
            w.close("mc:Choice");
            // The same text again: the fallback is a copy of the choice, not more of the story.
            let content = content.and_then(|r| w.s.get(r)).map(str::to_owned);
            w.open("mc:Fallback", &[]);
            self.vml_shape(w, o, float, z, content.as_deref());
            w.close("mc:Fallback");
            w.close("mc:AlternateContent");
        }
        w.close("w:r");
        self.rev_close(w, props);
    }

    /// A shape as VML (`w:pict`), for readers older than Word 2010 shapes: its frame, place,
    /// wrapping, fill, outline and text box. Only for a kind [`vml_tag`] names.
    fn vml_shape(&self, w: &mut W, o: &InlineObject, float: &Float, z: u32, content: Option<&str>) {
        let InlineObject::Shape { kind, w: sw, h: sh, fill, stroke, stroke_width, story, extra, .. } = o else { return };
        let Some(tag) = vml_tag(*kind) else { return };
        let pt = |v: f32| format!("{:.2}pt", craft_wordmodel::geom::finite(v));
        let mut style = Vec::new();
        if float.wrap != Wrap::Inline {
            style.push("position:absolute".to_string());
            style.push(format!("margin-left:{}", pt(float.x)));
            style.push(format!("margin-top:{}", pt(float.y)));
        }
        style.push(format!("width:{}", pt(sw.max(0.0))));
        style.push(format!("height:{}", pt(sh.max(0.0))));
        let spin = float.spin();
        if spin.deg != 0.0 {
            style.push(format!("rotation:{}", spin.deg));
        }
        match (spin.flip_h, spin.flip_v) {
            (true, true) => style.push("flip:x y".into()),
            (true, false) => style.push("flip:x".into()),
            (false, true) => style.push("flip:y".into()),
            (false, false) => {}
        }
        if float.wrap != Wrap::Inline {
            // The stacking order the choice was given, as a signed 32-bit `z-index`. Behind the text
            // is negative, shifted down rather than negated, so a higher `z` still stacks higher.
            let z = i64::from(z.min(i32::MAX as u32));
            let z = if float.wrap == Wrap::BehindText { z - (i64::from(i32::MAX) + 1) } else { z };
            style.push(format!("z-index:{z}"));
            // The same clearances from the text as the choice's `distL`/`distR`/`distT`/`distB`.
            let d = pt(float.dist.clamp(0.0, 1584.0));
            style.push(format!("mso-wrap-distance-left:{d}"));
            style.push(format!("mso-wrap-distance-right:{d}"));
            style.push(format!("mso-wrap-distance-top:{}", pt(float.dist_top.clamp(0.0, 1584.0))));
            style.push(format!("mso-wrap-distance-bottom:{}", pt(float.dist_bottom.clamp(0.0, 1584.0))));
            if let Some(a) = float.h_align {
                style.push(format!("mso-position-horizontal:{}", a.ooxml(true)));
            }
            style.push(format!("mso-position-horizontal-relative:{}", vml_rel(float.h_rel, true)));
            if let Some(a) = float.v_align {
                style.push(format!("mso-position-vertical:{}", a.ooxml(false)));
            }
            style.push(format!("mso-position-vertical-relative:{}", vml_rel(float.v_rel, false)));
        }
        let body = (*story).and_then(|id| self.doc.parts.get(&id)).map(|p| p.body).unwrap_or_default();
        style.push(
            match body.anchor {
                craft_wordmodel::props::VAlign::Top => "v-text-anchor:top",
                craft_wordmodel::props::VAlign::Center => "v-text-anchor:middle",
                craft_wordmodel::props::VAlign::Bottom => "v-text-anchor:bottom",
            }
            .into(),
        );
        if float.hidden {
            style.push("visibility:hidden".into());
        }
        let style = style.join(";");
        let mut attrs: Vec<(&str, String)> = vec![("style", style)];
        if *kind == ShapeKind::RoundedRectangle {
            // The same fraction of the shorter side: VML's in 1/65536ths, DrawingML's in 1/100000ths.
            let c = match extra.corner {
                Some(c) => c.min(craft_wordmodel::connector::MAX_CORNER) as f32 / 100_000.0,
                None => craft_wordmodel::connector::DEFAULT_CORNER,
            };
            attrs.push(("arcsize", format!("{}f", (c * 65_536.0).round() as u32)));
        }
        if float.wrap != Wrap::Inline {
            attrs.push(("o:allowincell", (if float.outside_cell { "f" } else { "t" }).into()));
        }
        let opacity = match fill.as_ref() {
            Some(f) => {
                attrs.push(("fillcolor", format!("#{}", f.main_color().hex())));
                match f {
                    Fill::Solid { transparency, .. } if *transparency > 0.0 => Some(format!("{:.4}", (1.0 - transparency / 100.0).clamp(0.0, 1.0))),
                    _ => None,
                }
            }
            None => {
                attrs.push(("filled", "f".into()));
                None
            }
        };
        match stroke {
            Some(c) => {
                attrs.push(("strokecolor", format!("#{}", c.hex())));
                attrs.push(("strokeweight", pt(stroke_width.clamp(0.0, 100.0))));
            }
            None => attrs.push(("stroked", "f".into())),
        }
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.open("w:pict", &[]);
        w.open(tag, &attrs);
        if let Some(opacity) = &opacity {
            w.empty("v:fill", &[("opacity", opacity)]);
        }
        if let Some(content) = content {
            let inset = body.inset.map(|i| i.map(|v| pt(v.min(craft_wordmodel::MAX_TEXT_BOX_INSET) as f32 / 12_700.0)).join(","));
            let mut tb: Vec<(&str, &str)> = Vec::new();
            if let Some(inset) = &inset {
                tb.push(("inset", inset));
            }
            // Text direction as the VML reader takes it (stacked letters have no VML form).
            let mut tb_style = match body.vert {
                craft_wordmodel::props::TextVert::Vert => vec!["layout-flow:vertical"],
                craft_wordmodel::props::TextVert::Vert270 => vec!["layout-flow:vertical", "mso-layout-flow-alt:bottom-to-top"],
                craft_wordmodel::props::TextVert::Horz | craft_wordmodel::props::TextVert::Stacked => Vec::new(),
            };
            if body.auto_fit {
                tb_style.push("mso-fit-shape-to-text:t");
            }
            let tb_style = tb_style.join(";");
            if !tb_style.is_empty() {
                tb.push(("style", &tb_style));
            }
            w.open("v:textbox", &tb);
            w.raw(content);
            w.close("v:textbox");
        }
        let wrap = match float.wrap {
            Wrap::Square => Some("square"),
            Wrap::Tight => Some("tight"),
            Wrap::Through => Some("through"),
            Wrap::TopAndBottom => Some("topAndBottom"),
            Wrap::Inline | Wrap::BehindText | Wrap::InFrontOfText => None,
        };
        if let Some(wrap) = wrap {
            let side = float.wrap_text.ooxml();
            let attrs: &[(&str, &str)] = if float.wrap_text.is_both() { &[("type", wrap)] } else { &[("type", wrap), ("side", side)] };
            w.empty("w10:wrap", attrs);
        }
        w.close(tag);
        w.close("w:pict");
    }

    fn write_group(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, depth: usize) {
        let InlineObject::Group { w: gw, h: gh, float, ch_w, ch_h, children, canvas } = o else { return };
        self.rev_open(w, props);
        w.open("w:r", &[]);
        self.rpr(w, props);
        w.open("w:drawing", &[]);
        let docpr = self.next_docpr();
        let link = Self::drawing_link(props, rels);
        let name = format!("{} {docpr}", if canvas.is_some() { "Canvas" } else { "Group" });
        self.drawing_open(w, DrawingInfo { float, size: (*gw, *gh), docpr: &docpr, name: &name, alt: o.alt_text(), link: link.as_deref() });
        w.empty("wp:cNvGraphicFramePr", &[]);
        w.open("a:graphic", &[]);
        let outer = match canvas {
            Some(c) => {
                // A Drawing Canvas: its background (`wpc:bg`) and outline (`wpc:whole`), then
                // its members, at offsets in the canvas's own (unscaled) space.
                w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas")]);
                w.open("wpc:wpc", &[]);
                w.open("wpc:bg", &[]);
                write_fill(w, c.fill.as_ref());
                w.close("wpc:bg");
                w.open("wpc:whole", &[]);
                match c.stroke {
                    Some(_) => w.open("a:ln", &[("w", &emu(c.stroke_width.clamp(0.0, 100.0)))]),
                    None => w.open("a:ln", &[]),
                }
                write_fill(w, c.stroke.map(Fill::solid).as_ref());
                w.close("a:ln");
                w.close("wpc:whole");
                "wpc:wpc"
            }
            None => {
                w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup")]);
                // ECMA-376 §20.1.7.6 (`a:xfrm` of a group): the members' space `a:chOff`/`a:chExt`
                // maps onto the group's `a:off`/`a:ext`.
                w.open("wpg:wgp", &[]);
                w.empty("wpg:cNvGrpSpPr", &[]);
                w.open("wpg:grpSpPr", &[]);
                w.open("a:xfrm", &spin_attrs(float.spin()).iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
                w.empty("a:off", &[("x", "0"), ("y", "0")]);
                w.empty("a:ext", &[("cx", &emu(gw.max(0.0))), ("cy", &emu(gh.max(0.0)))]);
                w.empty("a:chOff", &[("x", "0"), ("y", "0")]);
                w.empty("a:chExt", &[("cx", &emu(ch_w.max(0.0))), ("cy", &emu(ch_h.max(0.0)))]);
                w.close("a:xfrm");
                w.close("wpg:grpSpPr");
                "wpg:wgp"
            }
        };
        let mut remaining = craft_wordmodel::para::MAX_GROUP_CHILDREN;
        self.write_group_members(w, children, rels, depth, &mut remaining);
        if canvas.is_some() && float.spin() != craft_wordmodel::geom::Spin::default() {
            // The canvas schema has no transform of its own: keep its turn in our
            // extension (Word draws it unturned, as it can't turn a canvas).
            w.open("wpc:extLst", &[]);
            w.open("a:ext", &[("uri", crate::CANVAS_SPIN_EXT)]);
            xfrm_spin_only(w, float.spin());
            w.close("a:ext");
            w.close("wpc:extLst");
        }
        w.close(outer);
        w.close("a:graphicData");
        w.close("a:graphic");
        w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
        w.close("w:drawing");
        w.close("w:r");
        self.rev_close(w, props);
    }
    /// Members share one finite budget across the whole recursive group, not one per level.
    fn write_group_members(
        &mut self,
        w: &mut W,
        children: &[craft_wordmodel::para::GroupChild],
        rels: &mut PartRels,
        depth: usize,
        remaining: &mut usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in children {
            if *remaining == 0 {
                break;
            }
            *remaining -= 1;
            let at = (c.x, c.y);
            match &c.obj {
                InlineObject::Image { media, .. } => {
                    if let Some(file) = self.media_files.get(media).cloned() {
                        let id = self.next_docpr();
                        let link = member_link(c.link.as_deref(), rels);
                        self.pic(w, &c.obj, &file, MemberInfo { id: Some(&id), off: at, link: link.as_deref() }, rels);
                    }
                }
                InlineObject::Shape { extra, .. } => {
                    let id = self.shape_docpr(extra.id);
                    let link = member_link(c.link.as_deref(), rels);
                    self.wsp(w, &c.obj, MemberInfo { id: Some(&id), off: at, link: link.as_deref() }, rels, depth);
                }
                InlineObject::Group { .. } if depth < MAX_DEPTH => {
                    let id = self.next_docpr();
                    let link = member_link(c.link.as_deref(), rels);
                    self.write_inner_group(w, &c.obj, MemberInfo { id: Some(&id), off: at, link: link.as_deref() }, rels, depth + 1, remaining);
                }
                // Native ink kept from the file, with its fallback, at the member's place.
                InlineObject::Graphic { w: iw, h: ih, float, graphic, .. } if graphic.kind == craft_wordmodel::graphic::GraphicKind::Ink => {
                    if let Some(src) = graphic.source.as_deref()
                        && let Some(mut root) = self.embedded_tree(src, rels, None)
                    {
                        Self::place_member(&mut root, [c.x, c.y, *iw, *ih], float, 0);
                        w.raw(&root.to_xml());
                    }
                }
                _ => {}
            }
        }
    }

    /// A nested group is an inner `wpg:grpSp`, never another run or drawing anchor.
    fn write_inner_group(&mut self, w: &mut W, o: &InlineObject, member: MemberInfo<'_>, rels: &mut PartRels, depth: usize, remaining: &mut usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let InlineObject::Group { w: gw, h: gh, float, ch_w, ch_h, children, .. } = o else { return };
        let id = member.id.unwrap_or("0");
        let name = format!("Group {id}");
        w.open("wpg:grpSp", &[]);
        nv_props(w, "wpg:cNvPr", (id, &name), o.alt_text(), &float.alt, float.hidden, member.link);
        w.empty("wpg:cNvGrpSpPr", &[]);
        w.open("wpg:grpSpPr", &[]);
        w.open("a:xfrm", &spin_attrs(float.spin()).iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
        w.empty("a:off", &[("x", &emu(member.off.0)), ("y", &emu(member.off.1))]);
        w.empty("a:ext", &[("cx", &emu(gw.max(0.0))), ("cy", &emu(gh.max(0.0)))]);
        w.empty("a:chOff", &[("x", "0"), ("y", "0")]);
        w.empty("a:chExt", &[("cx", &emu(ch_w.max(0.0))), ("cy", &emu(ch_h.max(0.0)))]);
        w.close("a:xfrm");
        w.close("wpg:grpSpPr");
        self.write_group_members(w, children, rels, depth, remaining);
        w.close("wpg:grpSp");
    }
}

/// Group members have no enclosing run, so even a literal `#name` needs its own
/// external hyperlink relationship. Top-level drawings retain `drawing_link`'s policy.
fn member_link(target: Option<&str>, rels: &mut PartRels) -> Option<String> {
    target.filter(|url| !url.is_empty()).map(|url| rels.add(rt::HYPERLINK, url, true))
}

/// The VML element that draws a shape kind, for the kinds VML has one for.
fn vml_tag(kind: ShapeKind) -> Option<&'static str> {
    match kind {
        ShapeKind::Rectangle | ShapeKind::TextBox => Some("v:rect"),
        ShapeKind::RoundedRectangle => Some("v:roundrect"),
        ShapeKind::Ellipse => Some("v:oval"),
        _ => None,
    }
}

/// A floating object's anchor as VML names it (`mso-position-horizontal-relative`, `-vertical-`).
fn vml_rel(a: Anchor, horiz: bool) -> &'static str {
    match (a, horiz) {
        (Anchor::Margin, _) => "margin",
        (Anchor::Page, _) => "page",
        (Anchor::InsideMargin, _) => "inner-margin-area",
        (Anchor::OutsideMargin, _) => "outer-margin-area",
        (Anchor::LeftMargin, true) => "left-margin-area",
        (Anchor::RightMargin, true) => "right-margin-area",
        (Anchor::Character, true) => "char",
        (Anchor::TopMargin, false) => "top-margin-area",
        (Anchor::BottomMargin, false) => "bottom-margin-area",
        (Anchor::Line, false) => "line",
        _ => "text",
    }
}

pub(super) fn ink_name(tool: craft_wordmodel::freeform::InkTool) -> &'static str {
    match tool {
        craft_wordmodel::freeform::InkTool::Pen => "Pen",
        craft_wordmodel::freeform::InkTool::Pencil => "Pencil",
        craft_wordmodel::freeform::InkTool::Highlighter => "Highlighter",
    }
}

/// Is kept ink `src` in a form the drawing-level writer writes: its `a:graphic`, a direct run-level
/// `w:contentPart`, or an `mc:AlternateContent` whose choice holds a drawing (not a group member's
/// bare `w14:contentPart`, nor one in `mc:AlternateContent` with a `pic:pic` fallback)?
fn standalone_ink(src: &craft_wordmodel::graphic::Embedded) -> bool {
    match crate::xml::parse(src.xml.as_bytes()) {
        Ok(root) => match root.name.as_str() {
            "a:graphic" | "w:contentPart" => true,
            "mc:AlternateContent" => root.child("mc:Choice").and_then(|c| c.child("w:drawing")).is_some_and(|d| d.find("a:graphic").is_some()),
            _ => false,
        },
        Err(_) => false,
    }
}

#[cfg(test)]
mod standalone_ink_tests {
    use super::standalone_ink;
    use craft_wordmodel::graphic::Embedded;

    fn emb(xml: &str) -> Embedded {
        Embedded { xml: xml.into(), ..Default::default() }
    }

    /// Review: an alternate whose choice has a drawing but no `a:graphic` is not writable as ink.
    #[test]
    fn alternate_content_needs_a_graphic() {
        let ns = r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
        let with =
            format!("<mc:AlternateContent {ns}><mc:Choice Requires=\"w14\"><w:drawing><a:graphic/></w:drawing></mc:Choice></mc:AlternateContent>");
        let without = format!("<mc:AlternateContent {ns}><mc:Choice Requires=\"w14\"><w:drawing/></mc:Choice></mc:AlternateContent>");
        assert!(standalone_ink(&emb(&with)));
        assert!(!standalone_ink(&emb(&without)));
    }
}
