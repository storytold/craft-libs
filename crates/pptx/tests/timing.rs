//! Transitions and animations, written and read back.

mod common;

use std::sync::Arc;

use craft_pptx::anim::{ANIMATIONS, AnimStart, Animation, TRANSITIONS, TextBuild, Transition};
use craft_pptx::{Chart, Defaults, Plain, Presentation, Shape, ShapeId, ShapeKind, Slide, SlideId, TextBody, Xfrm, chart::Series};
use craft_xml::{El, Local, Options};

/// One slide per transition option, then one per animation option, each with a three-paragraph
/// box the animation targets.
fn deck() -> Presentation {
    let mut p = Presentation::default();
    let (m, last) = Plain.master(p.slide_size, 0);
    let layout = m.layouts[0].id;
    p.masters.push(Arc::new(m));
    p.next_id = last;
    let slide = |p: &mut Presentation, f: &mut dyn FnMut(&mut Slide, ShapeId)| {
        let id = SlideId(p.alloc_id().unwrap());
        let shape = ShapeId(p.alloc_id().unwrap());
        let mut s = Slide { id, layout, ..Default::default() };
        s.shapes.push(Shape {
            id: shape,
            xfrm: Some(Xfrm::new(10.0, 10.0, 200.0, 100.0)),
            text: Some(TextBody::from_text("One\nTwo\nThree")),
            ..Default::default()
        });
        f(&mut s, shape);
        p.slides.push(Arc::new(s));
    };
    // `none` is no transition at all, and reads back as such.
    for (kind, _, _, dur, opts) in TRANSITIONS.iter().filter(|t| t.0 != "none") {
        let opts: Vec<&str> = if opts.is_empty() { vec![""] } else { opts.to_vec() };
        for (i, o) in opts.iter().enumerate() {
            slide(&mut p, &mut |s, _| {
                s.transition = Some(Transition {
                    kind: kind.to_string(),
                    option: o.to_string(),
                    duration_ms: *dur,
                    advance_after_ms: (i % 2 == 1).then_some(3000),
                    ..Default::default()
                });
            });
        }
    }
    for (k, (effect, _, class, _, dur, opts)) in ANIMATIONS.iter().enumerate() {
        let opts: Vec<&str> = if opts.is_empty() { vec![""] } else { opts.to_vec() };
        for (i, o) in opts.iter().enumerate() {
            slide(&mut p, &mut |s, shape| {
                let start = [AnimStart::OnClick, AnimStart::WithPrevious, AnimStart::AfterPrevious][(k + i) % 3];
                let text_build = if (k + i) % 2 == 0 { TextBuild::AsOne } else { TextBuild::ByParagraph };
                s.animations.push(Animation {
                    shape,
                    class: *class,
                    effect: effect.to_string(),
                    option: o.to_string(),
                    start,
                    duration_ms: *dur,
                    text_build,
                    ..Default::default()
                });
            });
        }
    }
    p
}

#[test]
fn every_transition_and_animation_survives_writing_and_reading() {
    let p = deck();
    let q = craft_pptx::import(&craft_pptx::export(&p, &Plain).unwrap(), &Plain).unwrap();
    assert_eq!(q.slides.len(), p.slides.len());
    for (a, b) in p.slides.iter().zip(&q.slides) {
        let (ta, tb) = (a.transition.as_ref(), b.transition.as_ref());
        assert_eq!(ta.map(|t| (&t.kind, &t.option, t.advance_after_ms)), tb.map(|t| (&t.kind, &t.option, t.advance_after_ms)), "slide {}", a.id);
        assert_eq!(a.animations.len(), b.animations.len(), "slide {}", a.id);
        for (x, y) in a.animations.iter().zip(&b.animations) {
            // Effects and options are kept as far as PowerPoint's preset ids and subtypes tell
            // them apart (`circle` reads back as `shape`, `boxIn` as `circleIn`): compare the
            // preset.
            let preset = |a: &Animation| craft_pptx::anim::animation_info(&a.effect, a.class).map(|i| i.3);
            assert_eq!(
                (preset(x), x.class, x.start, x.text_build),
                (preset(y), y.class, y.start, y.text_build),
                "slide {}: {} / {}",
                a.id,
                x.effect,
                y.effect
            );
            assert_eq!(y.shape, b.shapes[0].id);
        }
    }
}

fn preset_nodes<'a>(node: &'a El, found: &mut Vec<&'a El>) {
    if node.is(Local("cTn")) && node.attr_local("presetClass").is_some() {
        found.push(node);
    }
    for child in node.elements() {
        preset_nodes(child, found);
    }
}

const CHART_SLIDE: &str = "ppt/slides/slide1.xml";

fn chart_animation_package(series_token: &str, whole_chart_build: bool) -> Vec<u8> {
    // Original synthetic chart and timing markup, using ISO/IEC 29500-1 chart build elements.
    let mut p = common::deck();
    let id = ShapeId(p.alloc_id().unwrap());
    let chart = Chart {
        categories: vec!["Morning".into(), "Afternoon".into()],
        series: vec![
            Series { name: "North".into(), values: vec![Some(2.0), Some(5.0)], ..Default::default() },
            Series { name: "South".into(), values: vec![Some(4.0), Some(3.0)], ..Default::default() },
        ],
        ..Default::default()
    };
    Arc::make_mut(&mut p.slides[0]).shapes.push(Shape {
        id,
        name: "Daylight readings".into(),
        kind: ShapeKind::Chart(Box::new(chart)),
        xfrm: Some(Xfrm::new(30.0, 20.0, 320.0, 180.0)),
        ..Default::default()
    });
    let bytes = common::export(&p);
    let slide_xml = common::part(&bytes, CHART_SLIDE);
    let slide = Options::default().strict().parse(slide_xml.as_bytes()).unwrap();
    let spid = slide
        .find(Local("graphicFrame"))
        .and_then(|frame| frame.path(&[Local("nvGraphicFramePr"), Local("cNvPr")]))
        .and_then(|props| props.attr_local("id"))
        .unwrap();
    let last_group = u32::MAX.to_string();
    let shape_target = format!(r#"<p:spTgt spid="{spid}"/>"#);
    let chart_target = |attrs: &str| format!(r#"<p:spTgt spid="{spid}"><p:graphicEl><a:chart {attrs}/></p:graphicEl></p:spTgt>"#);
    let effect = |id: u32, group: &str, target: &str| {
        let child = id + 1;
        format!(
            r#"<p:par><p:cTn id="{id}" presetID="1" presetClass="entr" nodeType="clickEffect" grpId="{group}"><p:childTnLst><p:set><p:cBhvr><p:cTn id="{child}" dur="1" fill="hold"/><p:tgtEl>{target}</p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="visible"/></p:to></p:set></p:childTnLst></p:cTn></p:par>"#
        )
    };
    let whole_build =
        if whole_chart_build { format!(r#"<p:bldGraphic spid="{spid}" grpId="7" uiExpand="1"><p:bldAsOne/></p:bldGraphic>"#) } else { String::new() };
    let timing = format!(
        r#"<p:timing><p:tnLst>{}{}{}{}</p:tnLst><p:bldLst>
        <p:bldGraphic spid="{spid}" grpId="0" uiExpand="1"><p:bldSub><a:bldChart bld="{series_token}" animBg="1"/></p:bldSub></p:bldGraphic>
        <p:bldGraphic spid="{spid}" grpId="{last_group}" uiExpand="0"><p:bldSub><a:bldChart bld="seriesEl" animBg="0"/></p:bldSub></p:bldGraphic>
        {whole_build}</p:bldLst></p:timing>"#,
        effect(10, "0", &chart_target(r#"bldStep="gridLegend""#)),
        effect(20, "0", &chart_target(&format!(r#"bldStep="{series_token}" seriesIdx="1""#))),
        effect(30, &last_group, &chart_target(r#"bldStep="ptInSeries" seriesIdx="1" categoryIdx="0""#)),
        effect(40, "7", &shape_target)
    );
    let original_slide = slide_xml.replace("</p:sld>", &format!("{timing}</p:sld>"));
    assert_ne!(original_slide, slide_xml);
    common::rewrite(&bytes, |name, data| Some(if name == CHART_SLIDE { original_slide.as_bytes() } else { data }.to_vec()), &[])
}

#[test]
fn chart_targets_and_build_groups_survive_two_saves() {
    let mut bytes = chart_animation_package("series", false);
    let last_group = u32::MAX.to_string();
    for _ in 0..2 {
        let p = common::import(&bytes);
        assert_eq!(p.slides[0].animations.len(), 4);
        bytes = common::export(&p);
        let slide = Options::default().strict().parse(&common::bytes_of(&bytes, CHART_SLIDE)).unwrap();
        let timing = slide.child(Local("timing")).unwrap();
        let mut effects = vec![];
        preset_nodes(timing, &mut effects);
        assert_eq!(effects.len(), 4);
        let targets: Vec<_> = effects
            .iter()
            .map(|e| {
                e.find(Local("spTgt"))
                    .and_then(|t| t.path(&[Local("graphicEl"), Local("chart")]))
                    .map(|c| (c.attr_local("bldStep").unwrap(), c.attr_local("seriesIdx"), c.attr_local("categoryIdx")))
            })
            .collect();
        assert_eq!(targets, [Some(("gridLegend", None, None)), Some(("series", Some("1"), None)), Some(("ptInSeries", Some("1"), Some("0"))), None]);
        assert_eq!(effects[0].attr_local("grpId"), Some("0"));
        assert_eq!(effects[1].attr_local("grpId"), Some("0"));
        assert_eq!(effects[2].attr_local("grpId"), Some(last_group.as_str()));
        let whole_group = effects[3].attr_local("grpId").unwrap();
        assert_ne!(whole_group, "0");
        assert_ne!(whole_group, last_group.as_str());
        let builds: Vec<_> = timing
            .child(Local("bldLst"))
            .unwrap()
            .children(Local("bldGraphic"))
            .map(|g| {
                let chart = g.path(&[Local("bldSub"), Local("bldChart")]).unwrap();
                (g.attr_local("grpId"), g.attr_local("uiExpand"), chart.attr_local("bld"), chart.attr_local("animBg"))
            })
            .collect();
        assert_eq!(builds, [(Some("0"), Some("1"), Some("series"), Some("1")), (Some(last_group.as_str()), Some("0"), Some("seriesEl"), Some("0"))]);
    }
}

#[test]
fn chart_timing_preserves_padded_selector_and_build_tokens() {
    let bytes = chart_animation_package(" &#9;series&#xA; ", false);
    let saved = common::export(&common::import(&bytes));
    let slide = Options::default().strict().parse(&common::bytes_of(&saved, CHART_SLIDE)).unwrap();
    let timing = slide.child(Local("timing")).unwrap();
    let mut effects = vec![];
    preset_nodes(timing, &mut effects);
    let step =
        effects[1].find(Local("spTgt")).and_then(|t| t.path(&[Local("graphicEl"), Local("chart")])).and_then(|chart| chart.attr_local("bldStep"));
    let build = timing.path(&[Local("bldLst"), Local("bldGraphic"), Local("bldSub"), Local("bldChart")]).and_then(|chart| chart.attr_local("bld"));
    assert_eq!((step, build), (Some("series"), Some("series")));
}

#[test]
fn whole_chart_build_stays_separate_from_series_builds_after_two_saves() {
    let mut bytes = chart_animation_package("series", true);
    for _ in 0..2 {
        bytes = common::export(&common::import(&bytes));
        let slide = Options::default().strict().parse(&common::bytes_of(&bytes, CHART_SLIDE)).unwrap();
        let timing = slide.child(Local("timing")).unwrap();
        let mut effects = vec![];
        preset_nodes(timing, &mut effects);
        let whole = timing.child(Local("bldLst")).unwrap().children(Local("bldGraphic")).find(|graphic| graphic.attr_local("grpId") == Some("7"));
        assert_eq!(
            (effects[3].attr_local("grpId"), whole.and_then(|g| g.attr_local("uiExpand")), whole.and_then(|g| g.child(Local("bldAsOne"))).is_some()),
            (Some("7"), Some("1"), true)
        );
        assert!(whole.unwrap().child(Local("bldSub")).is_none());
        assert_eq!(effects[1].attr_local("grpId"), Some("0"));
    }
}
