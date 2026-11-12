//! Writing the model: what `Drawing::tags` writes reads back as what was given.

use std::collections::HashSet;

use craft_dxf::model::text::{MText, MTextColumns};
use craft_dxf::model::write::{BlockDef, Drawing, Format, entity_tags, fixed};
use craft_dxf::model::{
    Attrib, Color, Common, DashElement, Dimension, Document, Entity, EntityKind, Gradient, Hatch, HatchEdge, HatchLoop, HatchPath, Helix, Image,
    Insert, Layer, Leader, Linetype, Lineweight, LwPolyline, LwVertex, MLine, MLineVertex, PatternLine, Polyline, Spline, Text, TextStyle,
    Transparency, Vertex, Viewport, records,
};
use craft_dxf::{Tag, parse, write_ascii_as};

fn common(layer: &str) -> Common {
    Common {
        handle: None,
        owner: None,
        layer: layer.into(),
        linetype: None,
        color: Color::ByLayer,
        lineweight: Lineweight::ByLayer,
        ltscale: 1.0,
        visible: true,
        paper_space: false,
        transparency: Transparency::ByLayer,
        thickness: 0.0,
        extrusion: [0.0, 0.0, 1.0],
        plot_style: None,
    }
}

fn text(value: &str) -> Text {
    Text {
        insert: [1.0, 2.0, 0.0],
        align: Some([3.0, 2.0, 0.0]),
        height: 2.5,
        value: value.into(),
        rotation: 30.0,
        width_factor: 0.8,
        oblique: 15.0,
        style: "Notes".into(),
        generation: 2,
        halign: 1,
        valign: 2,
    }
}

fn mtext(columns: Option<MTextColumns>) -> MText {
    MText {
        insert: [1.0, 2.0, 0.0],
        height: 2.5,
        width: 40.0,
        attach: 5,
        drawing_direction: 1,
        direction: None,
        rotation: 0.0,
        style: "Notes".into(),
        contents: format!("tab\there\nnext ^ line {}", "long ".repeat(120)),
        line_spacing: 1.2,
        line_spacing_exact: true,
        background: Some((1, Color::Index(4), 1.25)),
        columns,
    }
}

fn hatch(fit: bool) -> Hatch {
    let spline = HatchEdge::Spline {
        degree: 1,
        rational: true,
        periodic: false,
        knots: vec![0.0, 0.0, 1.0, 1.0],
        control: vec![[0.0, 3.0], [0.0, 0.0]],
        weights: vec![1.0, 2.0],
        fit: if fit { vec![[0.0, 3.0], [0.0, 0.0]] } else { Vec::new() },
        start_tangent: fit.then_some([0.0, -1.0]),
        end_tangent: None,
    };
    let edges = vec![
        HatchEdge::Line { start: [0.0, 0.0], end: [4.0, 0.0] },
        HatchEdge::Arc { center: [4.0, 1.0], radius: 1.0, start_angle: 270.0, end_angle: 360.0, ccw: true },
        HatchEdge::Ellipse { center: [2.0, 2.0], major: [2.0, 0.0], ratio: 0.5, start_angle: 0.0, end_angle: 180.0, ccw: false },
        spline,
    ];
    Hatch {
        pattern: "ANSI31".into(),
        solid: false,
        associative: true,
        style: 1,
        pattern_type: 1,
        angle: 45.0,
        scale: 2.0,
        double: false,
        elevation: 2.5,
        loops: vec![
            HatchLoop { flags: 1, path: HatchPath::Edges(edges), sources: vec![0x91, 0x92] },
            HatchLoop {
                flags: 2,
                path: HatchPath::Polyline { closed: true, vertices: vec![[1.0, 1.0, 0.5], [2.0, 1.0, 0.0], [2.0, 2.0, 0.0]] },
                sources: vec![],
            },
        ],
        pattern_lines: vec![
            PatternLine { angle: 45.0, base: [0.0, 0.0], offset: [-1.4, 1.4], dashes: vec![] },
            PatternLine { angle: 135.0, base: [1.0, 2.0], offset: [1.0, 1.0], dashes: vec![3.0, -1.5] },
        ],
        seeds: vec![[1.0, 1.0]],
        gradient: None,
    }
}

/// One of every kind the writer has, with values that are not the defaults. `full` adds what
/// only the newest versions hold.
fn sample(full: bool) -> Vec<Entity> {
    let e = |kind: EntityKind| Entity { common: common("Walls"), kind };
    let p = |x: f64| [x, x + 1.0, x + 2.0];
    let polyline =
        |flags: i64, vertices: Vec<Vertex>| Polyline { flags, elevation: 0.0, start_width: 0.0, end_width: 0.0, m: 0, n: 0, smooth: 0, vertices };
    let vertex = |x: f64, flags: i64, face: [i64; 4]| Vertex { location: [x, 1.0, 0.0], bulge: 0.0, start_width: 0.0, end_width: 0.0, flags, face };
    let attrib = Attrib {
        tag: "A".into(),
        text: text("one"),
        prompt: String::new(),
        flags: 1,
        lock_position: true,
        multiline: full.then(|| MText { background: None, ..mtext(None) }),
    };
    let attdef = Attrib { prompt: "Enter\ta".into(), flags: 12, multiline: None, ..attrib.clone() };
    let dim = |flags: i64| Dimension {
        flags,
        block: Some("*D1".into()),
        style: "ISO-25".into(),
        defpt: p(1.0),
        text_mid: p(2.0),
        insertion: [0.0; 3],
        p13: [0.0; 3],
        p14: [0.0; 3],
        p15: [0.0; 3],
        p16: [0.0; 3],
        text: "<> ^ m".into(),
        measurement: Some(12.5),
        rotation: 0.0,
        oblique: 0.0,
        text_rotation: 45.0,
        horizontal: 10.0,
        leader_length: 0.0,
    };
    let image = Image {
        insert: p(1.0),
        u: [0.5, 0.0, 0.0],
        v: [0.0, 0.5, 0.0],
        size: [640.0, 480.0],
        image_def: Some(0x77),
        display: 7,
        clipping: true,
        brightness: 60,
        contrast: 40,
        fade: 10,
        clip_type: 2,
        clip: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
        clip_inverted: full,
    };
    let mut styled = e(EntityKind::Line { start: p(1.0), end: p(4.0) });
    styled.common = Common {
        handle: Some(0x500),
        linetype: Some("DASHED".into()),
        color: Color::Rgb([255, 127, 0]),
        lineweight: Lineweight::Hundredths(50),
        ltscale: 2.5,
        visible: false,
        paper_space: true,
        transparency: Transparency::Alpha(128),
        thickness: 1.5,
        extrusion: [0.0, 0.0, -1.0],
        plot_style: Some(fixed::PLOT_NORMAL),
        ..common("Walls")
    };
    let columns = MTextColumns {
        dynamic: true,
        count: 3,
        width: 30.0,
        gutter: 5.0,
        height: 40.0,
        auto_height: false,
        heights: vec![10.0, 40.0, 20.0],
        flow_reversed: true,
    };
    let fixed_columns =
        MTextColumns { dynamic: false, count: 2, width: 30.0, gutter: 5.0, height: 80.0, auto_height: false, heights: vec![], flow_reversed: false };
    let gradient =
        Gradient { name: "LINEAR".into(), angle: 0.5, shift: 1.0, single: false, tint: 0.25, colors: vec![Color::Rgb([0, 0, 255]), Color::Index(2)] };
    vec![
        styled,
        e(EntityKind::Point { location: p(1.0), angle: 30.0 }),
        e(EntityKind::Circle { center: p(1.0), radius: 3.0 }),
        e(EntityKind::Arc { center: p(1.0), radius: 3.0, start_angle: 10.0, end_angle: 200.0 }),
        e(EntityKind::Ellipse { center: p(1.0), major: [2.0, 0.0, 0.0], ratio: 0.5, start: 0.25, end: 3.0 }),
        e(EntityKind::LwPolyline(LwPolyline {
            vertices: vec![
                LwVertex { x: 0.0, y: 0.0, bulge: 1.0, start_width: 0.0, end_width: 0.0 },
                LwVertex { x: 4.0, y: 0.0, bulge: 0.0, start_width: 0.1, end_width: 0.2 },
                LwVertex { x: 4.0, y: 3.0, ..LwVertex::default() },
            ],
            closed: true,
            plinegen: true,
            const_width: 0.5,
            elevation: 2.0,
        })),
        e(EntityKind::Polyline(Polyline {
            start_width: 0.5,
            end_width: 0.25,
            smooth: 6,
            ..polyline(1 | 4, vec![Vertex { bulge: 0.5, start_width: 0.1, end_width: 0.2, ..vertex(0.0, 16, [0; 4]) }, vertex(1.0, 0, [0; 4])])
        })),
        e(EntityKind::Polyline(polyline(8, vec![vertex(0.0, 32, [0; 4]), vertex(1.0, 32, [0; 4])]))),
        e(EntityKind::Polyline(Polyline {
            m: 3,
            n: 1,
            ..polyline(
                64,
                vec![
                    vertex(0.0, 192, [0; 4]),
                    vertex(1.0, 192, [0; 4]),
                    vertex(2.0, 192, [0; 4]),
                    Vertex { location: [0.0; 3], ..vertex(0.0, 128, [1, 2, -3, 0]) },
                ],
            )
        })),
        e(EntityKind::Spline(Spline {
            degree: 2,
            flags: 8,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control: vec![p(0.0), p(1.0), p(2.0)],
            weights: vec![1.0, 2.0, 1.0],
            fit: vec![p(0.0), p(2.0)],
            start_tangent: Some([1.0, 0.0, 0.0]),
            end_tangent: Some([0.0, 1.0, 0.0]),
            fit_tolerance: 0.01,
        })),
        e(EntityKind::Helix(Helix {
            spline: Spline {
                degree: 3,
                flags: 0,
                knots: vec![0.0, 1.0],
                control: vec![p(0.0), p(1.0)],
                weights: vec![],
                fit: vec![],
                start_tangent: None,
                end_tangent: None,
                fit_tolerance: 0.0,
            },
            axis_base: p(1.0),
            start: p(2.0),
            axis: [0.0, 0.0, 1.0],
            radius: 2.0,
            turns: 3.0,
            turn_height: 1.5,
            ccw: false,
            constrain: 1,
        })),
        e(EntityKind::MLine(MLine {
            style: "WALL".into(),
            style_handle: Some(0x41),
            scale: 2.0,
            justification: 1,
            flags: 1 | 4,
            elements: 2,
            start: p(0.0),
            vertices: vec![
                MLineVertex {
                    location: p(0.0),
                    direction: [1.0, 0.0, 0.0],
                    miter: [0.0, 1.0, 0.0],
                    element_params: vec![vec![0.5, 1.0, 2.0], vec![-0.5]],
                    fill_params: vec![vec![], vec![]],
                },
                MLineVertex {
                    location: p(4.0),
                    direction: [1.0, 0.0, 0.0],
                    miter: [0.0, 1.0, 0.0],
                    element_params: vec![vec![0.5], vec![-0.5]],
                    fill_params: vec![vec![], vec![]],
                },
            ],
        })),
        e(EntityKind::Ray { base: p(1.0), direction: [0.0, 1.0, 0.0] }),
        e(EntityKind::XLine { base: p(1.0), direction: [1.0, 0.0, 0.0] }),
        e(EntityKind::Text(text("10^2\tm"))),
        e(EntityKind::MText(mtext(None))),
        e(EntityKind::MText(MText { direction: Some([0.0, 1.0, 0.0]), background: None, ..mtext(Some(columns)) })),
        e(EntityKind::MText(MText { background: None, ..mtext(Some(fixed_columns)) })),
        e(EntityKind::AttDef(attdef)),
        e(EntityKind::Insert(Insert {
            block: "DOOR".into(),
            insert: p(1.0),
            scale: [2.0, 1.0, -1.0],
            rotation: 45.0,
            columns: 3,
            rows: 2,
            column_spacing: 5.0,
            row_spacing: 6.0,
            attribs: vec![(Common { color: Color::Index(2), handle: Some(0x501), ..common("Tags") }, attrib)],
        })),
        e(EntityKind::Insert(Insert {
            block: "DOOR".into(),
            insert: p(1.0),
            scale: [1.0; 3],
            rotation: 0.0,
            columns: 1,
            rows: 1,
            column_spacing: 0.0,
            row_spacing: 0.0,
            attribs: vec![],
        })),
        e(EntityKind::Dimension(Dimension { p13: p(3.0), p14: p(4.0), rotation: 30.0, oblique: 60.0, ..dim(128) })),
        e(EntityKind::Dimension(Dimension { p13: p(3.0), p14: p(4.0), ..dim(1) })),
        e(EntityKind::Dimension(Dimension { p13: p(3.0), p14: p(4.0), p15: p(5.0), p16: p(6.0), ..dim(2) })),
        e(EntityKind::Dimension(Dimension { p15: p(5.0), leader_length: 2.0, ..dim(3) })),
        e(EntityKind::Dimension(Dimension { p15: p(5.0), leader_length: 2.0, ..dim(4) })),
        e(EntityKind::Dimension(Dimension { p13: p(3.0), p14: p(4.0), p15: p(5.0), ..dim(5) })),
        e(EntityKind::Dimension(Dimension { p13: p(3.0), p14: p(4.0), ..dim(6 | 64) })),
        e(EntityKind::Leader(Leader {
            vertices: vec![p(1.0), [4.0, 5.0, 0.0], p(6.0)],
            arrow: false,
            spline: true,
            style: "Arrows".into(),
            annotation_type: 0,
            annotation: Some(0x4F),
        })),
        e(EntityKind::Solid { corners: [p(0.0), p(1.0), p(2.0), p(3.0)] }),
        e(EntityKind::Trace { corners: [p(0.0), p(1.0), p(2.0), p(3.0)] }),
        e(EntityKind::Face3d { corners: [p(0.0), p(1.0), p(2.0), p(3.0)], hidden_edges: 5 }),
        e(EntityKind::Hatch(Hatch { gradient: None, ..hatch(full) })),
        e(EntityKind::Hatch(Hatch {
            pattern: "SOLID".into(),
            solid: true,
            angle: 0.0,
            scale: 1.0,
            pattern_lines: vec![],
            gradient: Some(gradient),
            ..hatch(full)
        })),
        e(EntityKind::Viewport(Viewport {
            center: p(5.0),
            width: 8.0,
            height: 6.0,
            id: 2,
            status: 16384,
            view_center: [1.0, 2.0],
            view_direction: [0.0, 0.0, 1.0],
            view_target: p(9.0),
            view_height: 30.0,
            twist: 90.0,
            clip: Some(0xA1),
            frozen_layers: vec![0xB1, 0xB2],
        })),
        e(EntityKind::Image(image.clone())),
        e(EntityKind::Wipeout(Image { image_def: None, ..image })),
        e(EntityKind::Tolerance { insert: p(1.0), direction: [0.0, 1.0, 0.0], style: "Tol".into(), text: "{\\Fgdt;j}%%v0.1".into() }),
    ]
}

/// The file as text and back: what a reader of the written file sees.
fn through_text(tags: &[Tag], format: Format) -> Vec<Tag> {
    parse(write_ascii_as(tags, format.utf8()).as_bytes()).expect("the written file parses")
}

fn read_back(tags: &[Tag]) -> Vec<Entity> {
    Document::read(tags).entities.iter().map(Entity::read).collect()
}

/// What the writer decides: the handle of an entity that had none, and every owner.
fn settled(mut e: Entity, given: &Entity) -> Entity {
    if given.common.handle.is_none() {
        assert!(e.common.handle.is_some(), "every entity gets a handle");
        e.common.handle = None;
    }
    e.common.owner = None;
    if let (EntityKind::Insert(i), EntityKind::Insert(g)) = (&mut e.kind, &given.kind) {
        for ((c, _), (gc, _)) in i.attribs.iter_mut().zip(&g.attribs) {
            c.owner = None;
            c.handle = gc.handle.and(c.handle);
        }
    }
    e
}

#[test]
fn every_entity_reads_back_as_it_was_given() {
    for (format, full) in [(Format::R2010, false), (Format::R2018, true)] {
        let given = sample(full);
        let (tags, skipped) = Drawing { format, entities: given.clone(), ..Drawing::default() }.tags();
        assert_eq!(skipped, 0);
        let back = read_back(&through_text(&tags, format));
        assert_eq!(back.len(), given.len());
        for (b, g) in back.into_iter().zip(&given) {
            let mut g = g.clone();
            // An R2010 file has no fit data in a hatch's spline edge, and keeps a multiline
            // attribute as its one line.
            if !full && let EntityKind::Hatch(h) = &mut g.kind {
                assert!(matches!(&h.loops[0].path, HatchPath::Edges(e) if matches!(&e[3], HatchEdge::Spline { fit, .. } if fit.is_empty())));
                h.clone_from(&Hatch {
                    gradient: h.gradient.clone(),
                    pattern: h.pattern.clone(),
                    solid: h.solid,
                    angle: h.angle,
                    scale: h.scale,
                    pattern_lines: h.pattern_lines.clone(),
                    ..hatch(false)
                });
            }
            assert_eq!(settled(b, &g), g, "{format:?}");
        }
    }
}

#[test]
fn owners_are_the_space_the_entity_is_in() {
    let (tags, _) = Drawing { entities: sample(true), ..Drawing::default() }.tags();
    let back = read_back(&tags);
    assert_eq!(back[0].common.owner, Some(fixed::PAPER_RECORD), "a paper space entity");
    assert_eq!(back[1].common.owner, Some(fixed::MODEL_RECORD));
    let EntityKind::Insert(i) = &back[19].kind else { panic!("an insert") };
    assert_eq!(i.attribs[0].0.owner, back[19].common.handle, "an attribute is owned by its insert");
    assert_eq!((back[0].common.handle, i.attribs[0].0.handle), (Some(0x500), Some(0x501)), "given handles are kept");
}

#[test]
fn what_a_version_does_not_have_is_left_out() {
    let given = sample(false);
    // R12: no handles, owners or subclass markers, and no lightweight polyline.
    let (tags, _) = Drawing { format: Format::R12, entities: given.clone(), ..Drawing::default() }.tags();
    assert!(!tags.iter().any(|t| matches!(t.code, 5 | 100 | 105 | 370 | 420 | 440)), "R12 has none of these groups");
    let d = Document::read(&tags);
    assert_eq!(d.header.acadver().as_deref(), Some("AC1009"));
    assert!(d.objects.is_empty() && d.classes.is_empty() && d.layouts().is_empty());
    let back = read_back(&tags);
    let EntityKind::Polyline(p) = &back[5].kind else { panic!("the lightweight polyline as a POLYLINE") };
    assert!(p.closed() && p.flags & 128 != 0);
    assert_eq!((p.vertices.len(), p.vertices[0].bulge, p.vertices[1].end_width, p.start_width), (3, 1.0, 0.2, 0.5));
    assert_eq!(p.vertices[2].location, [4.0, 3.0, 2.0], "the elevation is each vertex's z");
    // The colour's nearest index stands in for the true colour.
    assert_eq!(back[0].common.color, Color::Index(30));
    assert_eq!((back[0].common.lineweight, back[0].common.transparency), (Lineweight::ByLayer, Transparency::ByLayer));
    // R13 has handles but still no LWPOLYLINE; R14 has it. R2000 has lineweights, R2004 colour.
    let kinds = |format| {
        let (tags, _) = Drawing { format, entities: given.clone(), ..Drawing::default() }.tags();
        read_back(&tags)
    };
    assert!(matches!(kinds(Format::R13)[5].kind, EntityKind::Polyline(_)));
    assert!(matches!(kinds(Format::R14)[5].kind, EntityKind::LwPolyline(_)));
    assert_eq!(kinds(Format::R14)[0].common.lineweight, Lineweight::ByLayer);
    let r2000 = &kinds(Format::R2000)[0].common;
    assert_eq!((r2000.lineweight, r2000.color, r2000.transparency), (Lineweight::Hundredths(50), Color::Index(30), Transparency::ByLayer));
    let r2004 = &kinds(Format::R2004)[0].common;
    assert_eq!((r2004.color, r2004.transparency), (Color::Rgb([255, 127, 0]), Transparency::Alpha(128)));
    for f in Format::ALL {
        let (tags, _) = Drawing { format: f, ..Drawing::default() }.tags();
        assert_eq!(Document::read(&through_text(&tags, f)).header.acadver().as_deref(), Some(f.acadver()));
    }
}

fn tables() -> (Vec<Layer>, Vec<Linetype>, Vec<TextStyle>) {
    let layer = Layer {
        handle: Some(0x200),
        name: "Walls".into(),
        color: Color::Rgb([255, 0, 255]),
        on: false,
        frozen: true,
        vp_freeze_new: true,
        locked: true,
        plot: false,
        linetype: "GAS".into(),
        lineweight: Lineweight::Hundredths(35),
        plot_style: Some(fixed::PLOT_NORMAL),
        transparency: Transparency::Alpha(127),
        description: Some("outer walls".into()),
    };
    let plain = Layer {
        handle: None,
        name: "Plain".into(),
        color: Color::Index(3),
        on: true,
        frozen: false,
        vp_freeze_new: false,
        locked: false,
        plot: true,
        linetype: "Continuous".into(),
        lineweight: Lineweight::Default,
        transparency: Transparency::ByLayer,
        description: None,
        ..layer.clone()
    };
    let text_element = DashElement {
        length: -0.2,
        absolute: true,
        text: Some("GAS".into()),
        shape: None,
        style: Some(0x201),
        scale: 0.1,
        rotation: 15.0,
        offset: [-0.1, -0.05],
    };
    let dash = |length: f64| DashElement { length, scale: 1.0, ..DashElement::default() };
    let gas = Linetype {
        handle: Some(0x202),
        name: "GAS".into(),
        description: "Gas line".into(),
        length: 0.95,
        pattern: vec![dash(0.5), text_element, dash(-0.25)],
    };
    let style = TextStyle {
        handle: Some(0x201),
        name: "Notes".into(),
        font_file: "arial.ttf".into(),
        big_font: String::new(),
        typeface: Some("Arial".into()),
        height: 2.5,
        width_factor: 0.8,
        oblique: 15.0,
        shape_file: false,
        vertical: true,
        backwards: true,
        upside_down: true,
        annotative: false,
    };
    (vec![layer, plain], vec![gas], vec![style])
}

#[test]
fn tables_read_back_as_they_were_given() {
    let (layers, linetypes, styles) = tables();
    let source = parse(b"0\nDIMSTYLE\n105\n203\n2\nISO-25\n70\n0\n41\n2.5\n278\n44\n69\n2\n70\n5\n").expect("parses");
    let dim_styles = Document { tables: records(&source), ..Document::default() }.dim_styles();
    let drawing = Drawing {
        layers: layers.clone(),
        linetypes: linetypes.clone(),
        styles: styles.clone(),
        dim_styles,
        insunits: 4,
        metric: false,
        ..Drawing::default()
    };
    let (tags, _) = drawing.tags();
    let tags = through_text(&tags, Format::R2018);
    let d = Document::read(&tags);
    let back = d.layers();
    assert_eq!(back[0].name, "0");
    assert_eq!(back[1], layers[0]);
    assert!(back[2].handle.is_some_and(|h| h >= fixed::FIRST_FREE));
    assert_eq!(Layer { handle: None, ..back[2].clone() }, Layer { plot_style: Some(fixed::PLOT_NORMAL), ..layers[1].clone() });
    let lt = d.linetypes();
    assert_eq!(lt.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["ByBlock", "ByLayer", "Continuous", "GAS"]);
    assert_eq!(lt[3], linetypes[0]);
    let st = d.text_styles();
    assert_eq!((st[0].name.as_str(), st[0].font_file.as_str()), ("Standard", "txt"));
    assert_eq!(st[1], styles[0]);
    let ds = d.dim_styles();
    assert_eq!(ds.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Standard", "ISO-25"]);
    assert_eq!(ds[1].handle, Some(0x203));
    assert_eq!(
        ds[1].vars.iter().map(|(n, t)| (*n, t.str())).collect::<Vec<_>>(),
        [("DIMASZ", "2.5".into()), ("DIMDSEP", "44".into()), ("DIMTFILL", "2".into()), ("DIMTFILLCLR", "5".to_string())]
    );
    assert_eq!((d.header.insunits(), d.header.measurement()), (4, Some(0)));
    assert_eq!(d.app_ids()[0].name, "ACAD");
    assert_eq!(d.vports()[0].name, "*Active");
}

#[test]
fn the_file_around_the_content() {
    let line = |layer: &str| Entity { common: common(layer), kind: EntityKind::Line { start: [0.0; 3], end: [1.0; 3] } };
    let door = BlockDef {
        name: "DOOR".into(),
        base: [1.0, 2.0, 3.0],
        description: "A door".into(),
        entities: vec![line("0"), Entity { common: common("0"), kind: EntityKind::Other }],
    };
    let raw = vec![Tag::s(0, "MULTILEADER"), Tag::s(5, "900"), Tag::s(8, "0")];
    let drawing = Drawing {
        blocks: vec![door],
        // Handles in the reserved range, and one given twice, are replaced.
        entities: vec![
            Entity { common: Common { handle: Some(0xA), ..common("0") }, ..line("0") },
            Entity { common: Common { handle: Some(0x400), ..common("0") }, ..line("0") },
            Entity { common: Common { handle: Some(0x400), ..common("0") }, ..line("0") },
        ],
        raw_entities: raw,
        extents: [0.0, 0.0, 100.0, 50.0],
        header: vec![
            ("$LTSCALE".into(), vec![Tag::f(40, 2.0)]),
            ("$INSUNITS".into(), vec![Tag::i(70, 6)]),
            ("$ACADVER".into(), vec![Tag::s(1, "nonsense")]),
            ("$HANDSEED".into(), vec![Tag::s(5, "1")]),
        ],
        root_entries: vec![("ACAD_IMAGE_DICT".into(), 0x901)],
        raw_objects: vec![Tag::s(0, "DICTIONARY"), Tag::s(5, "901"), Tag::s(330, "A")],
        raw_classes: vec![Tag::s(0, "CLASS"), Tag::s(1, "IMAGEDEF")],
        ..Drawing::default()
    };
    let (tags, skipped) = drawing.tags();
    assert_eq!(skipped, 1, "the entity with no groups to write");
    assert_eq!(tags.last().map(|t| (t.code, t.str())), Some((0, "EOF".to_string())));
    let d = Document::read(&tags);
    assert_eq!(d.sections, 6);
    assert!(d.skipped.is_empty());
    // Every handle once, and the seed above them all.
    let handles: Vec<u64> = tags.iter().filter(|t| matches!(t.code, 5 | 105)).filter_map(|t| u64::from_str_radix(&t.str(), 16).ok()).collect();
    let unique: HashSet<u64> = handles.iter().copied().collect();
    let seed = d.header.handseed().expect("a handle seed");
    assert_eq!(unique.len(), handles.len());
    assert!(handles.iter().filter(|h| **h != seed).all(|h| *h < seed));
    assert_eq!(d.entities.iter().filter(|e| e.record().handle() == Some(0x400)).count(), 1);
    assert!(d.entities.iter().all(|e| e.record().handle() != Some(0xA)));
    assert_eq!(d.entities.len(), 4, "the raw record is written after the entities");
    assert_eq!(d.entities[3].kind, "MULTILEADER");
    // The header: the writer's version and seed, the caller's variables over the defaults.
    assert_eq!((d.header.acadver().as_deref(), d.header.ltscale(), d.header.insunits()), (Some("AC1032"), 2.0, 6));
    assert_eq!(d.header.vars.iter().filter(|(n, _)| *n == "$INSUNITS" || *n == "$ACADVER" || *n == "$HANDSEED").count(), 3);
    assert_eq!(d.header.point("$EXTMAX"), Some([100.0, 50.0, 0.0]));
    // Blocks, their records and the layouts that name them.
    let names = d.block_names();
    let block = d.block("DOOR").expect("the block");
    assert_eq!((block.base(), block.description().as_deref(), block.entities.len()), ([1.0, 2.0, 3.0], Some("A door"), 1));
    assert_eq!(block.block_record().and_then(|h| names.get(&h).copied()), Some("DOOR"));
    assert_eq!(Entity::read(&block.entities[0]).common.owner, block.block_record());
    let mut layouts = d.layouts();
    layouts.sort_by_key(|l| l.tab_order);
    let of = |i: usize| (layouts[i].name.as_str(), layouts[i].block_record.and_then(|h| names.get(&h).copied()));
    assert_eq!((of(0), of(1)), (("Model", Some("*Model_Space")), ("Layout1", Some("*Paper_Space"))));
    let root = d.root_dictionary().expect("the root dictionary");
    assert_eq!((root.get("ACAD_LAYOUT"), root.get("ACAD_IMAGE_DICT")), (Some(fixed::LAYOUTS), Some(0x901)));
    assert!(d.objects.iter().any(|r| r.handle() == Some(0x901)));
    assert!(d.classes.iter().any(|r| r.groups().s(1).as_deref() == Some("IMAGEDEF")));
    // The file opens at its extents.
    let vp = &d.vports()[0];
    assert_eq!((vp.center, vp.height > 50.0), (Some([50.0, 25.0]), true));
}

#[test]
fn one_entity_for_a_file_the_caller_writes() {
    let insert = sample(true).remove(19);
    let mut next = 0x10;
    let tags =
        entity_tags(&Entity { common: Common { handle: Some(0x5), ..insert.common.clone() }, ..insert.clone() }, Format::R2018, 0x1F, &mut next);
    let (recs, strays) = craft_dxf::model::entity_records(&records(&tags));
    assert_eq!((recs.len(), strays), (1, 0));
    let back = Entity::read(&recs[0]);
    assert_eq!((back.common.handle, back.common.owner), (Some(0x5), Some(0x1F)), "its own handle, whatever its number");
    assert_eq!(recs[0].seqend.map(|s| s.handle()), Some(Some(0x10)));
    assert_eq!(next, 0x11, "the next free handle is past the ones used");
    assert!(entity_tags(&Entity { common: common("0"), kind: EntityKind::Other }, Format::R2018, 0, &mut next).is_empty());
}

#[test]
fn records_kept_from_another_file_are_renumbered() {
    use craft_dxf::model::write::{is_pointer_code, remap_handles};
    let kept = parse(
        b"0\nMULTILEADER\n5\n80\n102\n{ACAD_REACTORS\n330\n81\n102\n}\n330\n1F\n100\nAcDbEntity\n8\n0\n340\n0\n344\nZZ\n1001\nAPP\n1005\n82\n0\nDIMSTYLE\n105\n90\n2\nX\n340\n81\n",
    )
    .expect("parses");
    let map = |h: u64| match h {
        0x80 => Some(0x500),
        0x81 => Some(0x501),
        0x90 => Some(0x502),
        _ => None,
    };
    let out = remap_handles(&kept, map);
    let groups: Vec<(i32, String)> = out.iter().map(|t| (t.code, t.str())).collect();
    let want: Vec<(i32, String)> = [
        (0, "MULTILEADER"),
        (5, "500"),
        (102, "{ACAD_REACTORS"),
        (330, "501"),
        (102, "}"),
        // An owner the new file has no counterpart for is the null handle.
        (330, "0"),
        (100, "AcDbEntity"),
        (8, "0"),
        (340, "0"),
        // Text that is no handle is left as it is.
        (344, "ZZ"),
        (1001, "APP"),
        (1005, "0"),
        (0, "DIMSTYLE"),
        (105, "502"),
        (2, "X"),
        (340, "501"),
    ]
    .iter()
    .map(|(c, v)| (*c, v.to_string()))
    .collect();
    assert_eq!(groups, want);
    assert!(is_pointer_code(330) && is_pointer_code(1005) && is_pointer_code(390) && !is_pointer_code(5) && !is_pointer_code(370));
    // The renumbered records go through the writer as they are.
    let (tags, skipped) = Drawing { raw_entities: out[..12].to_vec(), ..Drawing::default() }.tags();
    assert_eq!(skipped, 0);
    assert!(Document::read(&tags).entities.iter().any(|e| e.kind == "MULTILEADER" && e.record().handle() == Some(0x500)));
}
