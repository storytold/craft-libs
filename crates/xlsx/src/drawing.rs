//! A worksheet's drawing part (`xdr:wsDr`, ECMA-376 Part 1, 20.5): the anchors that place
//! shapes, pictures, charts and groups on the grid.
//!
//! An anchor's placement is typed ([`Placement`]); the object it places stays the tree the file
//! has (`xdr:sp`, `xdr:pic`, `xdr:graphicFrame`, `xdr:grpSp`, `xdr:cxnSp`), so it is written back
//! as it was. [`Anchor`] says what the object is and which relationships it uses, and gives its
//! `a:xfrm`; its fill, line and text are read with `craft_drawingml::read` (`fill_of`, `line`,
//! `text_body`), which match by local name. Charts themselves are separate parts: see
//! [`crate::Package::chart`].

use craft_drawingml::Xfrm;
use craft_xml::El;

/// A cell corner plus an offset into the cell, in EMU (`xdr:from`, `xdr:to`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marker {
    pub col: u32,
    pub col_off: i64,
    pub row: u32,
    pub row_off: i64,
}

impl Marker {
    fn read(e: &El) -> Marker {
        let n = |k: &str| e.child(format!("xdr:{k}").as_str()).map(|c| c.text().trim().to_string()).unwrap_or_default();
        Marker {
            col: craft_xml::value::to_u32(&n("col")).unwrap_or(0),
            col_off: craft_xml::value::to_i64_lossy(&n("colOff")).unwrap_or(0),
            row: craft_xml::value::to_u32(&n("row")).unwrap_or(0),
            row_off: craft_xml::value::to_i64_lossy(&n("rowOff")).unwrap_or(0),
        }
    }

    fn to_el(self, name: &str) -> El {
        let leaf = |k: &str, v: String| El::new(format!("xdr:{k}")).with_text(v);
        El::new(name)
            .with_child(leaf("col", self.col.to_string()))
            .with_child(leaf("colOff", self.col_off.to_string()))
            .with_child(leaf("row", self.row.to_string()))
            .with_child(leaf("rowOff", self.row_off.to_string()))
    }
}

/// Where an anchor puts its object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// `xdr:twoCellAnchor`: from one cell corner to another. `edit_as` is `twoCell`, `oneCell`
    /// or `absolute`: how the object moves and sizes with the cells.
    TwoCell { from: Marker, to: Marker, edit_as: Option<String> },
    /// `xdr:oneCellAnchor`: from a cell corner, with a size in EMU.
    OneCell { from: Marker, cx: i64, cy: i64 },
    /// `xdr:absoluteAnchor`: a position and size in EMU.
    Absolute { x: i64, y: i64, cx: i64, cy: i64 },
}

/// What an anchor places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Shape,
    Picture,
    /// A graphic frame holding a chart (`c:chart`).
    Chart,
    /// A graphic frame holding something else (a SmartArt diagram, a table).
    GraphicFrame,
    Group,
    Connector,
    /// `xdr:contentPart` (ink) or anything else.
    Other,
}

/// One anchor of a drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub placement: Placement,
    /// The object, as written.
    pub object: El,
    /// `xdr:clientData` (`fLocksWithSheet`, `fPrintsWithSheet`).
    pub client_data: Option<El>,
}

fn ext(e: Option<&El>) -> (i64, i64) {
    let v = |k: &str| e.and_then(|e| e.attr(k)).and_then(craft_xml::value::to_i64_lossy).unwrap_or(0);
    (v("cx"), v("cy"))
}

impl Anchor {
    fn read(a: &El) -> Option<Anchor> {
        let placement = match a.name.as_str() {
            "xdr:twoCellAnchor" => Placement::TwoCell {
                from: Marker::read(a.child("xdr:from")?),
                to: a.child("xdr:to").map(Marker::read).unwrap_or_default(),
                edit_as: a.attr("editAs").map(str::to_string),
            },
            "xdr:oneCellAnchor" => {
                let (cx, cy) = ext(a.child("xdr:ext"));
                Placement::OneCell { from: Marker::read(a.child("xdr:from")?), cx, cy }
            }
            "xdr:absoluteAnchor" => {
                let pos = a.child("xdr:pos");
                let v = |k: &str| pos.and_then(|p| p.attr(k)).and_then(craft_xml::value::to_i64_lossy).unwrap_or(0);
                let (cx, cy) = ext(a.child("xdr:ext"));
                Placement::Absolute { x: v("x"), y: v("y"), cx, cy }
            }
            _ => return None,
        };
        let object = a.elements().find(|c| !matches!(c.name.as_str(), "xdr:from" | "xdr:to" | "xdr:ext" | "xdr:pos" | "xdr:clientData"))?.clone();
        Some(Anchor { placement, object, client_data: a.child("xdr:clientData").cloned() })
    }

    pub fn to_el(&self) -> El {
        let mut e = match &self.placement {
            Placement::TwoCell { from, to, edit_as } => {
                let mut e = El::new("xdr:twoCellAnchor");
                if let Some(v) = edit_as {
                    e.set_attr("editAs", v);
                }
                e.with_child(from.to_el("xdr:from")).with_child(to.to_el("xdr:to"))
            }
            Placement::OneCell { from, cx, cy } => {
                El::new("xdr:oneCellAnchor").with_child(from.to_el("xdr:from")).with_child(El::new("xdr:ext").with_attr("cx", cx).with_attr("cy", cy))
            }
            Placement::Absolute { x, y, cx, cy } => El::new("xdr:absoluteAnchor")
                .with_child(El::new("xdr:pos").with_attr("x", x).with_attr("y", y))
                .with_child(El::new("xdr:ext").with_attr("cx", cx).with_attr("cy", cy)),
        };
        e.push(self.object.clone());
        e.push(self.client_data.clone().unwrap_or_else(|| El::new("xdr:clientData")));
        e
    }

    pub fn kind(&self) -> ObjectKind {
        match self.object.name.as_str() {
            "xdr:sp" => ObjectKind::Shape,
            "xdr:pic" => ObjectKind::Picture,
            "xdr:graphicFrame" if self.chart_rel().is_some() => ObjectKind::Chart,
            "xdr:graphicFrame" => ObjectKind::GraphicFrame,
            "xdr:grpSp" => ObjectKind::Group,
            "xdr:cxnSp" => ObjectKind::Connector,
            _ => ObjectKind::Other,
        }
    }

    /// The object's non-visual properties (`xdr:cNvPr`): `id`, `name`, `descr`, `hidden`.
    pub fn props(&self) -> Option<&El> {
        self.object.elements().find(|c| c.name.starts_with("xdr:nv")).and_then(|nv| nv.child("xdr:cNvPr"))
    }

    pub fn name(&self) -> Option<&str> {
        self.props().and_then(|p| p.attr("name"))
    }

    /// The relationship id of the chart part a graphic frame shows (`c:chart/@r:id`, or a
    /// `cx:chart` of an Office 2016 chart).
    pub fn chart_rel(&self) -> Option<&str> {
        let data = self.object.path(&["a:graphic", "a:graphicData"])?;
        data.child("c:chart").or_else(|| data.child("cx:chart")).and_then(|c| c.attr("r:id"))
    }

    /// The relationship id of a picture's image (`a:blip/@r:embed`, or `r:link` for a linked one).
    pub fn picture_rel(&self) -> Option<&str> {
        let blip = self.object.path(&["xdr:blipFill", "a:blip"])?;
        blip.attr("r:embed").or_else(|| blip.attr("r:link"))
    }

    /// The object's `a:xfrm` (`xdr:xfrm` for a graphic frame), in points, with the child space
    /// of a group.
    pub fn xfrm(&self) -> Option<(Xfrm, Option<Xfrm>)> {
        let x = match self.object.name.as_str() {
            "xdr:graphicFrame" => self.object.child("xdr:xfrm"),
            "xdr:grpSp" => self.object.path(&["xdr:grpSpPr", "a:xfrm"]),
            _ => self.object.path(&["xdr:spPr", "a:xfrm"]),
        }?;
        Some(craft_drawingml::read::xfrm(x))
    }

    /// A chart in a graphic frame, its part reached through relationship `rel_id`.
    pub fn chart(placement: Placement, id: u32, name: &str, rel_id: &str) -> Anchor {
        let frame = El::new("xdr:graphicFrame")
            .with_attr("macro", "")
            .with_child(
                El::new("xdr:nvGraphicFramePr")
                    .with_child(El::new("xdr:cNvPr").with_attr("id", id).with_attr("name", name))
                    .with_child(El::new("xdr:cNvGraphicFramePr")),
            )
            .with_child(
                El::new("xdr:xfrm")
                    .with_child(El::new("a:off").with_attr("x", 0).with_attr("y", 0))
                    .with_child(El::new("a:ext").with_attr("cx", 0).with_attr("cy", 0)),
            )
            .with_child(
                El::new("a:graphic").with_child(
                    El::new("a:graphicData")
                        .with_attr("uri", "http://schemas.openxmlformats.org/drawingml/2006/chart")
                        .with_child(El::new("c:chart").with_attr("r:id", rel_id)),
                ),
            );
        Anchor { placement, object: frame, client_data: None }
    }

    /// A picture of the image reached through relationship `rel_id`, stretched over its anchor.
    pub fn picture(placement: Placement, id: u32, name: &str, descr: &str, rel_id: &str) -> Anchor {
        let mut props = El::new("xdr:cNvPr").with_attr("id", id).with_attr("name", name);
        if !descr.is_empty() {
            props.set_attr("descr", descr);
        }
        let (cx, cy) = match &placement {
            Placement::OneCell { cx, cy, .. } | Placement::Absolute { cx, cy, .. } => (*cx, *cy),
            Placement::TwoCell { .. } => (0, 0),
        };
        let pic = El::new("xdr:pic")
            .with_child(
                El::new("xdr:nvPicPr")
                    .with_child(props)
                    .with_child(El::new("xdr:cNvPicPr").with_child(El::new("a:picLocks").with_attr("noChangeAspect", 1))),
            )
            .with_child(
                El::new("xdr:blipFill")
                    .with_child(El::new("a:blip").with_attr("r:embed", rel_id))
                    .with_child(El::new("a:stretch").with_child(El::new("a:fillRect"))),
            )
            .with_child(
                El::new("xdr:spPr")
                    .with_child(
                        El::new("a:xfrm")
                            .with_child(El::new("a:off").with_attr("x", 0).with_attr("y", 0))
                            .with_child(El::new("a:ext").with_attr("cx", cx).with_attr("cy", cy)),
                    )
                    .with_child(El::new("a:prstGeom").with_attr("prst", "rect").with_child(El::new("a:avLst"))),
            );
        Anchor { placement, object: pic, client_data: None }
    }
}

/// One child of the drawing: an anchor, or what the file has around anchors (an
/// `mc:AlternateContent` holding Office 2010 or 2016 objects).
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Anchor(Anchor),
    Other(El),
}

/// A drawing part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    pub items: Vec<Item>,
    /// The root's attributes (`mc:Ignorable`), as written.
    pub attrs: Vec<(String, String)>,
}

impl Drawing {
    /// A part read with [`crate::ns::CANONICAL`]. An anchor that does not read (no `xdr:from`, no
    /// object) is kept as written.
    pub fn read(root: &El) -> Drawing {
        let items = root
            .elements()
            .map(|e| match Anchor::read(e) {
                Some(a) => Item::Anchor(a),
                None => Item::Other(e.clone()),
            })
            .collect();
        Drawing { items, attrs: root.attrs.clone() }
    }

    /// The anchors outside `mc:AlternateContent` items (see [`Drawing::alternate_anchors`]).
    pub fn anchors(&self) -> Vec<&Anchor> {
        self.items.iter().filter_map(|i| if let Item::Anchor(a) = i { Some(a) } else { None }).collect()
    }

    /// The anchors an `mc:AlternateContent` item holds, read as [`Drawing::read`] reads them:
    /// from its first choice, or its fallback when it has no choice.
    pub fn alternate_anchors(item: &El) -> Vec<Anchor> {
        let branch = item.child("mc:Choice").or_else(|| item.child("mc:Fallback"));
        branch.map(|b| b.elements().filter_map(Anchor::read).collect()).unwrap_or_default()
    }

    pub fn to_xml(&self) -> Vec<u8> {
        let mut root = El::new("xdr:wsDr");
        root.attrs = self.attrs.clone();
        for i in &self.items {
            root.push(match i {
                Item::Anchor(a) => a.to_el(),
                Item::Other(e) => e.clone(),
            });
        }
        crate::ns::declare(&mut root);
        // The drawing's own namespace has a prefix; the main SpreadsheetML one is not used.
        root.attrs.retain(|(k, _)| k != "xmlns");
        let mut w = craft_xml::Writer::document();
        w.tree(&root);
        w.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWING: &str = r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:twoCellAnchor editAs="oneCell"><xdr:from><xdr:col>1</xdr:col><xdr:colOff>9525</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>9</xdr:row><xdr:rowOff>190500</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"><a:extLst><a:ext uri="{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}"><a16:creationId xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main" id="{1}"/></a:ext></a:extLst></xdr:cNvPr><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>
<xdr:oneCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="12700" cy="25400"/><xdr:sp><xdr:nvSpPr><xdr:cNvPr id="3" name="Box"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr><a:xfrm rot="5400000"><a:off x="12700" y="0"/><a:ext cx="12700" cy="25400"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></xdr:spPr></xdr:sp><xdr:clientData fLocksWithSheet="0"/></xdr:oneCellAnchor>
<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:cx1="http://schemas.microsoft.com/office/drawing/2015/9/8/chartex" Requires="cx1"><xdr:absoluteAnchor><xdr:pos x="1" y="2"/><xdr:ext cx="3" cy="4"/><xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="4" name="Waterfall"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/drawing/2014/chartex"><cx:chart xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId2"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:absoluteAnchor></mc:Choice><mc:Fallback/></mc:AlternateContent></xdr:wsDr>"#;

    fn read(xml: &str) -> El {
        craft_xml::Options::canonical(&crate::ns::CANONICAL).parse(xml.as_bytes()).unwrap_or_default()
    }

    #[test]
    fn anchors_read_and_write_back() {
        let d = Drawing::read(&read(DRAWING));
        let anchors = d.anchors();
        assert_eq!(anchors.len(), 2);
        let chart = anchors[0];
        assert_eq!((chart.kind(), chart.name(), chart.chart_rel()), (ObjectKind::Chart, Some("Chart 1"), Some("rId1")));
        assert_eq!(
            chart.placement,
            Placement::TwoCell {
                from: Marker { col: 1, col_off: 9525, row: 2, row_off: 0 },
                to: Marker { col: 4, col_off: 0, row: 9, row_off: 190500 },
                edit_as: Some("oneCell".into())
            }
        );
        let shape = anchors[1];
        assert_eq!(
            (shape.kind(), shape.placement.clone()),
            (ObjectKind::Shape, Placement::OneCell { from: Marker::default(), cx: 12700, cy: 25400 })
        );
        let (x, _) = shape.xfrm().unwrap_or_default();
        assert_eq!((x.x, x.w, x.h, x.rot), (1.0, 1.0, 2.0, 90.0));
        let mut src = craft_drawingml::part::NoSource;
        let fill = craft_drawingml::read::fill_of(&mut src, shape.object.child("xdr:spPr").unwrap_or(&El::default()));
        assert!(fill.is_some());
        let Item::Other(alt) = &d.items[2] else { panic!("{:?}", d.items[2]) };
        let ex = Drawing::alternate_anchors(alt);
        assert_eq!(
            (ex[0].kind(), ex[0].chart_rel(), ex[0].placement.clone()),
            (ObjectKind::Chart, Some("rId2"), Placement::Absolute { x: 1, y: 2, cx: 3, cy: 4 })
        );

        let out = d.to_xml();
        let again = Drawing::read(&read(std::str::from_utf8(&out).unwrap_or_default()));
        assert_eq!(again, d);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("a16:creationId") && text.contains(r#"Requires="cx1""#) && !text.contains(r#" xmlns="#), "{text}");
    }

    #[test]
    fn new_objects() {
        let at = Placement::OneCell { from: Marker { col: 2, ..Marker::default() }, cx: 100, cy: 50 };
        let d = Drawing {
            items: vec![Item::Anchor(Anchor::chart(at.clone(), 2, "Chart", "rId7")), Item::Anchor(Anchor::picture(at, 3, "Pic", "logo", "rId8"))],
            ..Drawing::default()
        };
        let again = Drawing::read(&read(std::str::from_utf8(&d.to_xml()).unwrap_or_default()));
        let a = again.anchors();
        assert_eq!((a[0].kind(), a[0].chart_rel()), (ObjectKind::Chart, Some("rId7")));
        assert_eq!((a[1].kind(), a[1].picture_rel(), a[1].props().and_then(|p| p.attr("descr"))), (ObjectKind::Picture, Some("rId8"), Some("logo")));
    }
}
