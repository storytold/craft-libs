//! `CommandSearch`: the command-search opener, a search field's face that fires its command.

use super::ButtonOutput;
use super::face::{self, Arrangement, FaceLook, FaceSpec};
use crate::contract::{Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Pointer as _, Props as _, Role};
use crate::display::Kbd;
use crate::primitives::{HAlign, IconSpec};
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, FocusLook, IconLook, StyleCx, SurfaceLook, TextLook, TypeRole};
use crate::text::FieldStyle;
use craft_behavior::{Flow, IconId, Metrics, PartKey, Sense};
use craft_theme::{Stateful, States};
use egui::{Rect, Vec2};

/// The part the opener shows its command's chord in.
const CHORD: PartKey = PartKey::new("chord");

/// The style of the command-search opener: a text field's surface, padding and glyph, and the placeholder in the
/// colour a field's placeholder takes.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CommandSearchStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The placeholder.
    #[style(text)]
    text: TextLook,
    /// The search glyph.
    #[style(icon)]
    icon: IconLook,
    face: FaceLook,
}

/// The command-search opener: a search field's face showing the search glyph, a placeholder and the chord of its
/// command at the reading end. It takes no text: a click, Enter or Space fires the command, which opens the search.
/// The placeholder is its name; cut to fit, it shows whole in the tooltip, and the chord keeps its width.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct CommandSearch {
    #[common]
    common: Common,
    glyph: IconSpec<'static>,
    /// The command whose chord the face shows; the opener is bound to it.
    chord: CommandRef,
}

impl CommandSearch {
    /// An opener showing `placeholder` that fires `command`.
    pub fn new(command: impl Into<CommandRef>, placeholder: impl Into<Label>) -> Self {
        let command = command.into();
        CommandSearch { common: Common::new(), glyph: IconSpec::svg(IconId::new("search")), chord: command.clone() }
            .command(command)
            .label(placeholder)
    }

    /// The room the glyph and placeholder take: the content less the chord's slot, `chord` wide, and the gap before it.
    fn face_area(content: Rect, rtl: bool, chord: f32, gap: f32) -> Rect {
        if chord > 0.0 { Flow::new(content, rtl).take_end(chord + gap).1.rect() } else { content }
    }

    fn spec<'s>(&'s self, label: &'s str) -> FaceSpec<'s> {
        FaceSpec {
            arrangement: Arrangement::Inline,
            label,
            detail: None,
            chord: None,
            role: Some(TypeRole::Body),
            lead: Some(&self.glyph),
            picture: None,
            trail: None,
            loading: false,
            mnemonic: None,
            align: HAlign::Start,
        }
    }
}

impl Component for CommandSearch {
    const ROLE: Role = Role::BUTTON;
    const OVERLAYS: bool = false;
    type Style = CommandSearchStyle;
    type Output = ButtonOutput;

    fn style(cx: &StyleCx<'_>, _: CommandSearchStyleKey) -> CommandSearchStyle {
        let field = FieldStyle::at(cx);
        let text = &cx.tokens.color.text;
        let (faint, off) = (cx.color(text.faint), cx.color(text.disabled));
        CommandSearchStyle {
            // The opener carries no value, so it is never invalid: the well without the field's failure edge.
            surface: cx.text_well(),
            metrics: field.metrics(),
            focus: face::focus(cx),
            text: TextLook::new(TypeRole::Body, Stateful::new(faint).off(States::new(faint, faint, faint, off))),
            icon: field.glyph().clone(),
            face: FaceLook::of(cx),
        }
    }

    /// The chord's room is kept at the least width too: the placeholder is cut first.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let size = face::measure(cx, &self.spec(cx.resolved.label));
        let chord = cx.face(Kbd::command(self.chord.clone())).x;
        if chord > 0.0 { Vec2::new(size.x + cx.style.face.gap + chord, size.y) } else { size }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    /// The chord's slot at the reading end, taken before the placeholder's room; it paints as the face of its part.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let chord = cx.face(Kbd::command(self.chord.clone())).x;
        if chord > 0.0 {
            let (slot, _) = Flow::new(cx.content_rect, cx.flow.is_rtl()).take_end(chord);
            cx.part(CHORD, slot);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
    }

    fn activated(&self, cx: &Cx<'_, Self>) -> bool {
        cx.activation().is_some()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let chord = cx.part_if_declared(CHORD).map_or(0.0, |part| part.rect.width());
        let area = Self::face_area(cx.content_rect, cx.flow.is_rtl(), chord, cx.style.face.gap);
        face::paint(ui, cx, (&cx.style.text, &cx.style.icon), &cx.style.face, &self.spec(cx.resolved.label), area);
        if chord > 0.0 {
            ui.face(CHORD, Kbd::command(self.chord.clone()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ButtonOutput {
        ButtonOutput::of(&cx, self.activated(&cx))
    }
}
