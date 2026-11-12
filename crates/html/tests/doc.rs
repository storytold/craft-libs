//! HTML documents (feature `flow`): the HTML parts of WordCraft's cross-format and MathML tests
//! (`crates/formats/src/tests.rs`, `mathml.rs`): equations through HTML, hostile input, entities.

use craft_flow::{FBlock, Inline};
use craft_html::doc::{import, inlines_html, parse};
use std::time::{Duration, Instant};

fn equations(html: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for b in &parse(html).blocks {
        if let FBlock::Para(p) = b {
            for i in &p.inlines {
                if let Inline::Equation { linear, display } = i {
                    out.push((linear.clone(), *display));
                }
            }
        }
    }
    out
}

#[test]
fn html_export_round_trips_equations() {
    for (linear, display) in [("y=2x_s", false), ("x=(-b±√(b^2-4ac))/2a", true), ("∑_(k=1)^n▒k", false), ("■(a&b@c&d)", false)] {
        let html = inlines_html(&[Inline::Equation { linear: linear.into(), display }]);
        assert!(html.starts_with("<math"), "{html}");
        assert_eq!(equations(&format!("<p>Before {html} after</p>")), vec![(linear.to_string(), display)], "{html}");
    }
    // Text after the equation still arrives; an unclosed element ends with the input.
    assert!(import("<p><math><mi>x</mi></math> tail</p>").plain_text(Default::default()).contains("tail"));
    assert!(!equations("<p><math><mfrac><mi>x").is_empty());
    let deep = format!("<math>{}<mi>x</mi>{}</math>", "<mrow>".repeat(5000), "</mrow>".repeat(5000));
    assert!(equations(&deep).len() <= 1);
}

#[test]
fn math_over_the_token_cap_is_skipped_to_its_end() {
    // Past the cap, the rest of the element is dropped, not read as body text.
    let html = format!("<p><math>{}<mrow><mi>z</mi></mrow></math> tail</p>", "<mi>y</mi>".repeat(craft_math::mathml::MAX_NODES * 2));
    let mut text = String::new();
    for b in &parse(&html).blocks {
        if let FBlock::Para(p) = b {
            for i in &p.inlines {
                if let Inline::Text(t, _) = i {
                    text.push_str(t);
                }
            }
        }
    }
    assert_eq!(text.trim(), "tail");
}

#[test]
fn deep_nesting_is_bounded() {
    let html = "<div><table><tr><td>".repeat(5000) + "x";
    assert!(import(&html).plain_text(Default::default()).contains('x'));
}

#[test]
fn entities_in_text() {
    let d = import("<p>&lt;&amp;&#65;&eacute; &nosuchentity; &#x41;</p>");
    assert_eq!(d.plain_text(Default::default()).trim(), "<&Aé &nosuchentity; A");
    // An entity name longer than the scan window is left alone.
    let long = format!("<p>&{};</p>", "a".repeat(40));
    assert!(import(&long).plain_text(Default::default()).contains(&"a".repeat(40)));
}

/// A run of `&#` is read in linear time: four times as much takes well under the sixteen times
/// as long a quadratic read would (a ratio, so a busy machine does not fail it).
#[test]
fn a_run_of_references_takes_linear_time() {
    let fastest = |n: usize| {
        let text = "&#".repeat(n);
        (0..3)
            .map(|_| {
                let t = Instant::now();
                let _ = import(&text).plain_text(Default::default());
                t.elapsed()
            })
            .min()
            .unwrap_or_default()
    };
    let (small, large) = (fastest(125_000), fastest(500_000));
    assert!(large < small * 8 + Duration::from_millis(50), "{small:?} for 125,000, {large:?} for 500,000");
}

proptest::proptest! {
    #[test]
    fn fuzz_html(s in "(<(/)?(p|b|i|ul|ol|li|table|tr|td|th|pre|blockquote|a href=x|img src=data:,|br|h2|span style='color:red')>|[a-z &;#<>\"'=]){0,120}") {
        let d = import(&s);
        let _ = craft_html::doc::export(&d);
    }
}
