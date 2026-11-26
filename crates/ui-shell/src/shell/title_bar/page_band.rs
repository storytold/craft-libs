//! `PageBand`: the move band the shell's page shows over the window.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::bar::{Window, act_on_double_click, show_end, window, window_acting};
use super::measure::End;
use crate::contract::{
    BodyCx, BodyLayout, Common, Component, Container, ContainerProps, Cx, DragBand, EndRegion, HeaderRegion, Pad, Props as _, Role,
    ShowContainer as _, Sizing,
};
use crate::runtime::egui::pass;
use crate::runtime::env::{Env, PageBandRoom, ShellEntry};
use crate::runtime::windows::{WindowAction, WindowHost};
use crate::shell::{Caption, ShellWords};
use crate::style::{ComponentStyle, StyleCx, UiTokens};
use craft_behavior::{Metrics, Sense};
use egui::{Rect, Response, Ui, vec2};

/// The room of the page band of a shell whose window fills `window` in this pass: the title-bar-high strip at its top,
/// with the room the window's ends take at either end (the traffic lights' inset or the caption buttons; none for a
/// native frame). `None` while the window is in OS full screen, where it has no ends and the OS moves nothing. `acts`
/// is false for an embedded shell, whose band is a picture of a window's.
pub(crate) fn room(cx: &BodyCx<'_>, tokens: &UiTokens, window: Rect, acts: bool) -> Option<PageBandRoom> {
    let facts = window_acting(cx, None, None, acts);
    if facts.full_screen {
        return None;
    }
    let height = crate::shell::SlotSize::TitleBar.resolve(tokens).min(window.height());
    let rect = Rect::from_min_size(window.min, vec2(window.width(), height));
    Some(PageBandRoom::new(rect, taken(&facts.ends.start), taken(&facts.ends.end)))
}

/// The width one end takes: the lights' inset, then its caption buttons.
fn taken(end: &End) -> f32 {
    end.inset + end.captions.iter().map(|slot| slot.rect.width()).sum::<f32>()
}

/// The style of the page band: the page shows through it.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct PageBandStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The band over a shown page that moves the window, with the window's ends: the traffic lights' room or the caption
/// buttons. It has no title and no content of its own; the page under it reads the room of its ends with
/// `ShellCx::page_band` and keeps its own controls clear of the free band. A double click acts as on the title bar.
/// In an embedded shell it is a picture of a window: no move band and no window command.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub(crate) struct PageBand {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl PageBand {
    /// A band with the window's ends at the room the shell gives it.
    pub(crate) fn new() -> Self {
        // No surface: the page shows through the band. It fills the room the shell gives it, so its move band spans
        // the free part between the ends whatever they hold.
        let props = ContainerProps::new()
            .pad(Pad::None)
            .sizing(Sizing::Fill)
            .layout(BodyLayout::row())
            .header(HeaderRegion::new().band(DragBand::Move).inline(true))
            .end(EndRegion::new());
        PageBand { common: Common::new(), props }
    }

    /// Shows the band at the room of the shell `ui` runs in ([`ShellCx::page_band`](crate::runtime::env::ShellCx::page_band));
    /// nothing when no page band is set.
    pub(crate) fn show_over_page(self, ui: &mut Ui) {
        let env = Env::of(ui);
        let Some(room) = env.get::<ShellEntry>().0.and_then(|shell| shell.page_band()) else { return };
        let label = env.words::<ShellWords>().title_bar.clone();
        let _ = self.label(label).show_at(ui, room.rect, |_, _| {});
    }

    /// Shows one end of the band; its Close asks the window it acts on to close, through the close guard of a root
    /// window (`WindowAction::Close`), since the band has no output to report it through.
    fn end_of(ui: &mut Ui, cx: &BodyCx<'_>, end: &End, facts: &Window) {
        if show_end(ui, cx, end, facts) == Some(Caption::Close) && facts.acts {
            let ctx = cx.memory.ctx().clone();
            WindowHost::of(&ctx).act(pass::current_viewport(&ctx), WindowAction::Close);
        }
    }
}

impl Component for PageBand {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = PageBandStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PageBandStyleKey) -> PageBandStyle {
        PageBandStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for PageBand {
    /// The start end: the lights' room or the captions at the start edge. A double click on the free band acts here,
    /// once a pass.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let facts = window(cx, None, None);
        act_on_double_click(cx, &facts);
        Self::end_of(ui, cx, &facts.ends.start, &facts);
    }

    /// The end end: the lights' room or the captions at the end edge.
    fn end(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let facts = window(cx, None, None);
        Self::end_of(ui, cx, &facts.ends.end, &facts);
    }
}
