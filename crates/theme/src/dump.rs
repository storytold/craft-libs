//! Token text dump: every token of a resolved [`Tokens`] as ordered `key = value` lines.
//!
//! The struct destructuring is exhaustive on purpose: a new token doesn't compile until it is
//! dumped.

use std::fmt::Write as _;

use crate::{
    BarSizes, BySize, CanvasSizes, ColorToken, ComponentTokens, ControlSizes, DockColumnSizes, Easing, Edge, Elevation, ElevationTokens, FloatSizes,
    FontSlot, Gaps, HandleSizes, HitSizes, IconSizes, Insets, LengthSizes, MarkerSizes, Motion, MotionTokens, Pads, PaneSizes, PlotSizes, RadiusSet,
    RailSizes, RampSizes, Rgba, RowStyle, RulerSizes, RulerTickSizes, ShapeTokens, SizeTokens, SliderTokens, SpaceTokens, StrokeTokens, ThumbSizes,
    Tokens, TypeStyle, TypeTokens, WindowSizes,
};

/// The text of every token, one `key = value` line each, in a fixed order.
pub fn tokens_text(tokens: &Tokens) -> String {
    let mut out = Lines::default();

    let Tokens { color, shape, space, size, stroke, typography, elevation, motion, component } = tokens;
    for &token in ColorToken::ALL {
        out.line(token.path(), hex(color.get(token)));
    }
    shape_lines(&mut out, shape);
    space_lines(&mut out, space);
    size_lines(&mut out, size);
    stroke_lines(&mut out, stroke);
    type_lines(&mut out, typography);
    elevation_lines(&mut out, elevation);
    motion_lines(&mut out, motion);
    component_lines(&mut out, component);
    out.0
}

/// Ordered `key = value` lines.
#[derive(Default)]
struct Lines(String);

impl Lines {
    fn line(&mut self, key: &str, value: impl std::fmt::Display) {
        let _ = writeln!(self.0, "{key} = {value}");
    }
}

/// `#rrggbbaa`, alpha always written.
fn hex(c: Rgba) -> String {
    let [r, g, b, a] = c.0;
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}

/// A length with up to two decimals.
fn len(v: f32) -> String {
    decimals(v, 2)
}

/// `v` with up to `places` decimals, trailing zeros dropped.
fn decimals(v: f32, places: usize) -> String {
    let s = format!("{v:.places$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s == "-0" { "0".to_owned() } else { s.to_owned() }
}

fn edge(e: Edge) -> String {
    match e {
        Edge::Flat => "flat".to_owned(),
        Edge::Bevel { light, dark, width } => format!("bevel {} {} {}", hex(light), hex(dark), len(width)),
    }
}

fn shape_lines(out: &mut Lines, shape: &ShapeTokens) {
    let ShapeTokens { radius, edge: e, rows } = shape;
    let RadiusSet { sm, control, surface, window, panel, button, pill } = radius;
    for (k, v) in [("sm", sm), ("control", control), ("surface", surface), ("window", window), ("panel", panel), ("button", button), ("pill", pill)] {
        out.line(&format!("shape.radius.{k}"), len(*v));
    }
    out.line("shape.edge", edge(*e));
    out.line("shape.rows", if *rows == RowStyle::Divided { "divided" } else { "rounded" });
}

fn stroke_lines(out: &mut Lines, stroke: &StrokeTokens) {
    let StrokeTokens { thin, medium, thick, heavy, dash, dash_gap } = stroke;
    for (k, v) in [("thin", thin), ("medium", medium), ("thick", thick), ("heavy", heavy), ("dash", dash), ("dash_gap", dash_gap)] {
        out.line(&format!("stroke.{k}"), len(*v));
    }
}

fn space_lines(out: &mut Lines, space: &SpaceTokens) {
    let SpaceTokens { gap, pad, inset } = space;
    let Gaps { tight, inline, row, stack, item, section, card_column, card_row } = gap;
    for (k, v) in [
        ("tight", tight),
        ("inline", inline),
        ("row", row),
        ("stack", stack),
        ("item", item),
        ("section", section),
        ("card_column", card_column),
        ("card_row", card_row),
    ] {
        out.line(&format!("space.gap.{k}"), len(*v));
    }
    let Pads { control, regular, roomy, chip, segment } = pad;
    for (k, v) in [("control", control), ("regular", regular), ("roomy", roomy), ("chip", chip), ("segment", segment)] {
        out.line(&format!("space.pad.{k}"), len(*v));
    }
    let Insets { screen } = inset;
    out.line("space.inset.screen", len(*screen));
}

fn size_lines(out: &mut Lines, size: &SizeTokens) {
    let SizeTokens {
        control,
        icon,
        rail,
        bar,
        panel_header,
        popup_min,
        scroll_fade,
        thumb: thumbs,
        thumb_row,
        handle,
        hit,
        checker,
        dial,
        plot,
        length,
        groove,
        ramp,
        meter,
        marker,
        toggle,
        pointer_tip,
        float,
        window,
        pane,
        ruler,
        dock,
        canvas,
    } = size;
    let ControlSizes { sm, md } = control;
    out.line("size.control.sm", len(*sm));
    out.line("size.control.md", len(*md));
    let IconSizes { sm, md, lg, glyph_sm, glyph_md } = icon;
    out.line("size.icon.sm", len(*sm));
    out.line("size.icon.md", len(*md));
    out.line("size.icon.lg", len(*lg));
    out.line("size.icon.glyph_sm", len(*glyph_sm));
    out.line("size.icon.glyph_md", len(*glyph_md));
    let RailSizes { width, button } = rail;
    out.line("size.rail.width", len(*width));
    out.line("size.rail.button", len(*button));
    let BarSizes { title, tab, status, options, options_narrow } = bar;
    for (k, v) in [("title", title), ("tab", tab), ("status", status), ("options", options), ("options_narrow", options_narrow)] {
        out.line(&format!("size.bar.{k}"), len(*v));
    }
    out.line("size.panel_header", len(*panel_header));
    out.line("size.popup_min", len(*popup_min));
    out.line("size.scroll_fade", len(*scroll_fade));
    let ThumbSizes { sm, md, lg, card } = thumbs;
    out.line("size.thumb.sm", len(*sm));
    out.line("size.thumb.md", len(*md));
    out.line("size.thumb.lg", len(*lg));
    out.line("size.thumb.card", len(*card));
    out.line("size.thumb_row", len(*thumb_row));
    let HandleSizes { sm, md, ring, hit: hit_radius, rotate_reach, band_inner, band_outer } = handle;
    out.line("size.handle.sm", len(*sm));
    out.line("size.handle.md", len(*md));
    out.line("size.handle.ring", len(*ring));
    out.line("size.handle.hit", len(*hit_radius));
    out.line("size.handle.rotate_reach", len(*rotate_reach));
    out.line("size.handle.band_inner", len(*band_inner));
    out.line("size.handle.band_outer", len(*band_outer));
    let HitSizes { slop, target, dead_zone, reveal } = hit;
    out.line("size.hit.slop", len(*slop));
    out.line("size.hit.target", len(*target));
    out.line("size.hit.dead_zone", len(*dead_zone));
    out.line("size.hit.reveal", len(*reveal));
    out.line("size.checker", len(*checker));
    out.line("size.dial", len(*dial));
    let PlotSizes { width, height, most } = plot;
    out.line("size.plot.width", len(*width));
    out.line("size.plot.height", len(*height));
    out.line("size.plot.most", len(*most));
    let LengthSizes { field, track, short, number } = length;
    out.line("size.length.field", len(*field));
    out.line("size.length.track", len(*track));
    out.line("size.length.short", len(*short));
    out.line("size.length.number", len(*number));
    out.line("size.groove", len(*groove));
    let RampSizes { bar, groove: ramp_groove } = ramp;
    out.line("size.ramp.bar", len(*bar));
    out.line("size.ramp.groove", len(*ramp_groove));
    out.line("size.meter", len(*meter));
    let MarkerSizes { width: marker_width, height: marker_height } = marker;
    out.line("size.marker.width", len(*marker_width));
    out.line("size.marker.height", len(*marker_height));
    out.line("size.toggle", len(*toggle));
    out.line("size.pointer_tip.x", len(pointer_tip.x));
    out.line("size.pointer_tip.y", len(pointer_tip.y));
    let FloatSizes { width, height } = float;
    out.line("size.float.width", len(*width));
    out.line("size.float.height", len(*height));
    let WindowSizes { cascade } = window;
    out.line("size.window.cascade", len(*cascade));
    let PaneSizes { least } = pane;
    out.line("size.pane.least", len(*least));
    let RulerSizes { tick } = ruler;
    let RulerTickSizes { mid } = tick;
    out.line("size.ruler.tick.mid", len(*mid));
    let DockColumnSizes { column } = dock;
    out.line("size.dock.column", len(*column));
    let CanvasSizes { compass, stage_height, stage_width_min, stage_width_max } = canvas;
    out.line("size.canvas.compass", len(*compass));
    out.line("size.canvas.stage_height", len(*stage_height));
    out.line("size.canvas.stage_width_min", len(*stage_width_min));
    out.line("size.canvas.stage_width_max", len(*stage_width_max));
}

fn slot(s: FontSlot) -> &'static str {
    match s {
        FontSlot::Ui => "ui",
        FontSlot::UiSemibold => "ui_semibold",
        FontSlot::Mono => "mono",
    }
}

fn type_lines(out: &mut Lines, t: &TypeTokens) {
    let TypeTokens { caption, label, body, body_strong, title, heading, body_large, mono } = t;
    let roles = [
        ("caption", caption),
        ("label", label),
        ("body", body),
        ("body_strong", body_strong),
        ("title", title),
        ("heading", heading),
        ("body_large", body_large),
        ("mono", mono),
    ];
    for (k, style) in roles {
        let TypeStyle { size, line_height, font } = style;
        out.line(&format!("typography.{k}.size"), len(*size));
        out.line(&format!("typography.{k}.line_height"), len(*line_height));
        out.line(&format!("typography.{k}.font"), slot(*font));
    }
}

fn elevation_lines(out: &mut Lines, e: &ElevationTokens) {
    let ElevationTokens { popup, floating, knob } = e;
    for (k, level) in [("popup", popup), ("floating", floating), ("knob", knob)] {
        let Elevation { offset: [x, y], blur, spread, alpha } = level;
        out.line(&format!("elevation.{k}.offset"), format!("{} {}", len(*x), len(*y)));
        out.line(&format!("elevation.{k}.blur"), len(*blur));
        out.line(&format!("elevation.{k}.spread"), len(*spread));
        out.line(&format!("elevation.{k}.alpha"), decimals(*alpha, 3));
    }
}

fn easing(e: Easing) -> &'static str {
    match e {
        Easing::Standard => "standard",
        Easing::Enter => "enter",
        Easing::Exit => "exit",
    }
}

fn motion_lines(out: &mut Lines, m: &MotionTokens) {
    let MotionTokens { hover, open, r#move, caret, outline, reduced } = m;
    for (k, motion) in [("hover", hover), ("open", open), ("move", r#move), ("caret", caret), ("outline", outline)] {
        let Motion { millis, easing: e } = motion;
        out.line(&format!("motion.{k}.millis"), millis);
        out.line(&format!("motion.{k}.easing"), easing(*e));
    }
    out.line("motion.reduced", reduced);
}

fn by_size(out: &mut Lines, key: &str, v: &BySize<f32>) {
    let BySize { sm, md } = v;
    out.line(&format!("{key}.sm"), len(*sm));
    out.line(&format!("{key}.md"), len(*md));
}

fn component_lines(out: &mut Lines, c: &ComponentTokens) {
    let ComponentTokens { slider } = c;
    let SliderTokens { thumb_width, thumb_height } = slider;
    by_size(out, "slider.thumb_width", thumb_width);
    by_size(out, "slider.thumb_height", thumb_height);
}
