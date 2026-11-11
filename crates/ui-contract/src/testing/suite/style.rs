//! Style cases: a listed variant looks unlike the default, every flag look of the style is reachable by some state
//! of its type, and text and icons read on what they sit on. They measure the styles the pipeline resolved while
//! the type's stories ran (settled, then driven), resolved again for every listed variant
//! and built-in theme. A type with no story is measured through its `#[component(sample = ..)]`. A type with
//! neither fails, since its style is never measured.

use super::drive::{input_fixture, run, variants};
use super::{ANY, Case, Subject};
use crate::style::UiTokens;
use crate::style::{StyleColor, SurfaceKind, SurfaceLook};
use crate::testing::fixture::Fixture;
use crate::testing::style::{StyleLooks, StyleSeen, themes_at, tokens, with_cx};
use craft_behavior::Input;
use craft_theme::{
    Appearance, ColorToken, Contrast, ControlSize, Flags, Mode, Resolve, Rgba, Size, State, StateMix, Stateful, TextSize, TokenRef, builtins, explain,
};
use std::sync::Arc;

pub(super) const CASES: &[Case] = &[
    Case::new("variants-differ", ANY, listed_variants_differ_from_the_default),
    Case::new("looks-reachable", ANY, every_flag_look_is_reachable),
    Case::new("text-contrast", ANY, text_and_icons_meet_the_text_minimum),
];

/// A style of the subject that the cases measure: one the pipeline resolved for a story, or the one of the type's
/// sample.
pub(super) enum Source {
    Seen(Arc<StyleSeen>),
    Sample,
}

/// The styles of `subject` the stories resolve, read once for the style cases: each story settled, then each of its
/// drives run once. One entry per style key, variant and size; the cases resolve each again under the themes they
/// measure.
fn sources(subject: &Subject) -> Result<&[Source], String> {
    subject.style_sources.get_or_init(|| read_sources(subject)).as_deref().map_err(Clone::clone)
}

fn read_sources(subject: &Subject) -> Result<Vec<Source>, String> {
    let name = subject.probe.type_name();
    let mut seen: Vec<Arc<StyleSeen>> = Vec::new();
    let mut take = |fixture: &Fixture| {
        for style in fixture.styles_of(name) {
            if !seen.iter().any(|known| known.inputs == style.inputs && known.variant == style.variant && known.size == style.size) {
                seen.push(style);
            }
        }
    };
    for story in &subject.stories {
        let base = subject.base(story);
        take(&base.fixture);
        let plan = base.plan;
        for variant in variants(plan, subject.probe) {
            let mut fixture = input_fixture(story);
            run(&mut fixture, plan, variant);
            take(&fixture);
        }
    }
    if !seen.is_empty() {
        return Ok(seen.into_iter().map(Source::Seen).collect());
    }
    if subject.probe.sample.is_some() {
        return Ok(vec![Source::Sample]);
    }
    Err(format!("{name} has a style and neither a story that shows it nor a sample, so no measurement reads its style"))
}

/// The context the style cases resolve looks in, one per type. A style hook that measures text needs the fonts, which
/// egui installs in its first frame; the style cache keys each look by its key, env, variant and size, so the looks
/// resolved before stand in for nothing else.
fn measuring_ctx(subject: &Subject) -> &egui::Context {
    subject.style_ctx.get_or_init(|| {
        let ctx = egui::Context::default();
        drop(crate::runtime::egui::fixture::run_frame_shapes(&ctx, 0.0, egui::vec2(800.0, 600.0), |_| {}));
        ctx
    })
}

/// The resolved looks of `source` under `variant` and `tokens`.
fn looks_in(subject: &Subject, source: &Source, tokens: &UiTokens, variant: craft_theme::Variant) -> Option<StyleLooks> {
    let ctx = measuring_ctx(subject);
    match source {
        Source::Seen(seen) => Some(with_cx(ctx, tokens, variant, seen.size, |cx| seen.resolve_in(cx))),
        Source::Sample => {
            let make = subject.probe.sample?;
            let instance = make();
            with_cx(ctx, tokens, variant, ControlSize::Md, |cx| (subject.probe.looks)(&*instance, cx))
        }
    }
}

fn looks_under(subject: &Subject, source: &Source, variant: craft_theme::Variant) -> Option<StyleLooks> {
    looks_in(subject, source, &tokens(Mode::Light, Contrast::Standard), variant)
}

fn listed_variants_differ_from_the_default(subject: &Subject) -> Result<(), String> {
    for source in sources(subject)? {
        let Some(listed) = looks_under(subject, source, craft_theme::Variant::SECONDARY) else {
            continue;
        };
        let Some((&default, others)) = listed.variants.split_first() else {
            continue;
        };
        let base = looks_under(subject, source, default);
        for &variant in others {
            if let (Some(base), Some(looks)) = (&base, looks_under(subject, source, variant))
                && looks.same_as(base)
            {
                return Err(format!("the variant {variant:?} resolves to the style of {default:?}"));
            }
        }
    }
    Ok(())
}

/// The flags some state of the type can set (core 4.1, "Reachable flags"): those step 12 takes from any component's
/// props, scope and bound command or from its `flags()`, and those its input, value, popup and shape add.
fn reachable(subject: &Subject) -> Flags {
    let probe = subject.probe;
    let mut flags = Flags::READ_ONLY | Flags::SELECTED | Flags::CHECKED | Flags::BUSY | Flags::EMPHASIZED;
    if !matches!(probe.input, Input::Display) {
        flags |= Flags::FOCUSED | Flags::FOCUS_VISIBLE;
    }
    if probe.carries_value() {
        flags |= Flags::MIXED | Flags::INVALID | Flags::WARN;
    }
    if probe.overlay {
        flags |= Flags::OPEN;
    }
    if matches!(probe.shape, crate::contract::ShapeKind::Items) || super::nodes::is_row_role(probe.role) {
        flags |= Flags::LEAD | Flags::DRAGGING | Flags::DROP_TARGET | Flags::CURRENT;
    }
    if super::nodes::is_menu_item_role(probe.role) {
        flags |= Flags::LEAD;
    }
    if !matches!(probe.shape, crate::contract::ShapeKind::Leaf) {
        flags |= Flags::EXPANDED;
    }
    flags
}

fn unreachable_in(look: &Stateful<StyleColor>, reachable: Flags) -> Option<String> {
    for (mask, _) in &look.when {
        if !reachable.contains(mask.0) {
            return Some(format!("a `when` entry needs {:?}, which no state of the type sets", mask.0 - reachable));
        }
    }
    if look.on.is_some() && !reachable.intersects(Flags::ON) {
        return Some("an `on` look, and no state of the type sets selected, checked or mixed".to_owned());
    }
    None
}

fn every_flag_look_is_reachable(subject: &Subject) -> Result<(), String> {
    let reachable = reachable(subject);
    for source in sources(subject)? {
        let Some(looks) = looks_under(subject, source, craft_theme::Variant::SECONDARY) else {
            continue;
        };
        if let Some(SurfaceLook { fill, border, .. }) = &looks.surface {
            if let Some(problem) = unreachable_in(fill, reachable) {
                return Err(format!("the surface fill has {problem}"));
            }
            if let Some(problem) = border.as_ref().and_then(|border| unreachable_in(border, reachable)) {
                return Err(format!("the surface border has {problem}"));
            }
        }
        if looks.focus.is_some() && !reachable.intersects(Flags::FOCUSED | Flags::FOCUS_VISIBLE) {
            return Err("the style has a focus ring and the type takes no focus".to_owned());
        }
    }
    Ok(())
}

/// The flag sets worth resolving a state in: none, and each `when` mask of `looks` that the type can reach.
fn flag_sets<'a>(looks: impl Iterator<Item = &'a Stateful<crate::style::StyleColor>>, reachable: Flags) -> Vec<Flags> {
    let mut sets = vec![Flags::empty()];
    for look in looks {
        sets.extend(look.when.iter().map(|(mask, _)| mask.0).filter(|mask| reachable.contains(*mask)));
    }
    sets
}

/// The states text is measured in: rest, with the `on` axis off and on. Hover and pressed fills are transient and not
/// measured for text.
const MEASURED: [(State, StateMix); 2] = [(State::Rest, StateMix::new(0.0, 0.0, 0.0)), (State::Rest, StateMix::new(0.0, 0.0, 1.0))];

/// The opaque surfaces a translucent fill can sit on: the chrome surfaces of the pair table. The canvas holds the
/// document, whose colours no theme pairs with the interface's inks.
const GROUNDS: [SurfaceKind; 5] = [SurfaceKind::Background, SurfaceKind::Chrome, SurfaceKind::Panel, SurfaceKind::Raised, SurfaceKind::Sunken];

/// Text and icon colours against what they are painted on at rest, with the `on` axis off and on and under each
/// reachable `when` mask, for every variant the type lists and every built-in theme at both contrasts, against the
/// theme's pair table. Hover, pressed and disabled states are not measured. The ink is measured
/// against the ink's declared ground, else the style's surface fill, composited over each of `GROUNDS` when it is not opaque; a style without a surface
/// fill, or a fully transparent one, is measured against each of `GROUNDS`.
///
/// An ink on a ground that the pair table does not list fails the component: it must use a paired ink. A listed pair
/// passes: its minimum is the theme's, and a pair below it is a finding about the theme, which the theme's own
/// tests report, not about the component.
fn text_and_icons_meet_the_text_minimum(subject: &Subject) -> Result<(), String> {
    let reachable = reachable(subject);
    for source in sources(subject)? {
        measure_contrast(subject, source, reachable)?;
    }
    Ok(())
}

fn measure_contrast(subject: &Subject, source: &Source, reachable: Flags) -> Result<(), String> {
    for contrast in [Contrast::Standard, Contrast::High] {
        for (theme, tokens) in themes_at(contrast) {
            let table = PairTable::of(&theme, contrast);
            let Some(first) = looks_in(subject, source, &tokens, craft_theme::Variant::SECONDARY) else {
                continue;
            };
            let variants = if first.variants.is_empty() { &[craft_theme::Variant::SECONDARY][..] } else { first.variants };
            for &variant in variants {
                let Some(looks) = looks_in(subject, source, &tokens, variant) else {
                    continue;
                };
                let surface_fill = looks.surface.as_ref().map(|surface| &surface.fill);
                let grounds: Vec<(String, egui::Color32)> =
                    GROUNDS.iter().map(|&kind| (format!("{kind:?}"), crate::style::surface_style(&tokens, kind).fill)).collect();
                let inks = looks
                    .texts
                    .iter()
                    .map(|text| ("text", &text.color, text.ground.as_ref()))
                    .chain(looks.icons.iter().map(|icon| ("icon", &icon.color, icon.ground.as_ref())));
                for (what, ink, ground) in inks {
                    let fill = ground.or(surface_fill);
                    for flags in flag_sets(fill.into_iter().chain([ink]), reachable) {
                        for (state, t) in MEASURED {
                            let paint = ink.resolve(state, flags, t).egui();
                            let fill = fill.map(|fill| fill.resolve(state, flags, t).egui()).filter(|fill| fill.a() != 0);
                            let under: Vec<(String, egui::Color32)> = match fill {
                                Some(fill) if fill.a() == 255 => vec![("surface fill".to_owned(), fill)],
                                Some(fill) => {
                                    grounds.iter().map(|(name, ground)| (format!("surface fill over {name}"), ground.blend(fill))).collect()
                                }
                                None => grounds.iter().map(|(name, ground)| (format!("{name} ground"), *ground)).collect(),
                            };
                            let ink = Rgba::from(paint);
                            let opaque_fill = fill.is_some_and(|fill| fill.a() == 255);
                            // An ink on a fill of its own must pair with that fill. On a ground the style does not choose
                            // (no fill, or a translucent one, or an ink painted on a part's own fill) the ink must
                            // be a foreground of the table.
                            let unpaired = if opaque_fill {
                                under.iter().find(|(_, ground)| !table.lists(ink, Rgba::from(*ground)))
                            } else {
                                (!table.has_ink(ink)).then_some(&under[0])
                            };
                            if let Some((name, ground)) = unpaired {
                                return Err(format!(
                                    "{theme} {contrast:?} {variant:?}: {what} {paint:?} ({}) on {name} {ground:?} ({}) in {state:?} with {flags:?} is no pair of the theme's table, so the component needs a paired ink",
                                    table.names(ink),
                                    table.names(Rgba::from(*ground))
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// The colour pairs a built-in theme declares at one contrast, as colours.
struct PairTable {
    pairs: Vec<(Rgba, Rgba)>,
    colors: Vec<(String, Rgba)>,
}

impl PairTable {
    /// The built-in theme `theme` (`<name>.<mode>`) at `contrast`.
    fn of(theme: &str, contrast: Contrast) -> Self {
        let (name, mode) = theme.rsplit_once('.').unwrap_or((theme, "light"));
        let mode = if mode == "light" { Mode::Light } else { Mode::Dark };
        let id = format!("craft.{name}");
        let appearance = Appearance { mode, contrast, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
        let Some(def) = builtins().iter().find(|def| def.id.as_str() == id) else {
            return PairTable { pairs: Vec::new(), colors: Vec::new() };
        };
        let explained = explain(def, appearance, None);
        let colors = &explained.resolved.tokens.color;
        let pairs = explained
            .pairs
            .iter()
            .filter_map(|report| match (report.pair.fg, report.pair.bg) {
                (TokenRef::Color(fg), TokenRef::Color(bg)) => Some((colors.get(fg), colors.get(bg))),
                _ => None,
            })
            .collect();
        let colors = ColorToken::ALL.iter().map(|&token| (token.path().to_owned(), colors.get(token))).collect();
        PairTable { pairs, colors }
    }

    fn has_ink(&self, ink: Rgba) -> bool {
        self.pairs.iter().any(|(fg, _)| *fg == ink)
    }

    fn lists(&self, ink: Rgba, ground: Rgba) -> bool {
        self.pairs.contains(&(ink, ground))
    }

    /// The tokens with colour `color`, for messages.
    fn names(&self, color: Rgba) -> String {
        let names: Vec<&str> = self.colors.iter().filter(|(_, value)| *value == color).map(|(name, _)| name.as_str()).collect();
        if names.is_empty() { "no token".to_owned() } else { names.join("/") }
    }
}
