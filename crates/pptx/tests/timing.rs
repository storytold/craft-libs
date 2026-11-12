//! Transitions and animations, written and read back.

use std::sync::Arc;

use craft_pptx::anim::{ANIMATIONS, AnimStart, Animation, TRANSITIONS, TextBuild, Transition};
use craft_pptx::{Defaults, Plain, Presentation, Shape, ShapeId, Slide, SlideId, TextBody, Xfrm};

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
