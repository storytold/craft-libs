//! A container's `ThemeScope`: the container, everything it shows and the layers opened from inside it resolve another
//! theme definition, mode, contrast or density, and the root token cache resolves an unchanged definition once.

use craft_theme::{Appearance, ColorToken, Contrast, Mode, ModeChoice, Rgba, Size, TextSize, ThemeDef, ThemeId, builtins, resolve};
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Mount, NoValue, Outcome, Overlay, Props as _, Role, ShowContainer as _,
    ThemeScope,
};
use craft_ui::runtime::env::{Env, RootEnv};
use craft_ui::style::StyleCx;
use craft_ui::testing::style::root_token_resolves;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Response, Ui, Vec2};
use std::cell::Cell;
use std::sync::Arc;

/// A container that records the mode its own `Cx` resolves in.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Themed<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    own_mode: Option<&'a Cell<Option<Mode>>>,
}

impl Themed<'_> {
    fn new(scope: ThemeScope) -> Self {
        Themed { common: Common::new(), props: ContainerProps::new().theme(scope), own_mode: None }
    }
}

impl Component for Themed<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ThemedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(own_mode) = self.own_mode {
            own_mode.set(Some(cx.env.mode()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Themed<'_> {}

/// A floating layer with a bound open state.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Sheet<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    open: Bind<'a, bool>,
}

struct SheetOut(Response);

impl Outcome for SheetOut {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.0
    }

    fn unshown(response: Response) -> Option<Self> {
        Some(SheetOut(response))
    }
}

impl Component for Sheet<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = SheetOut;

    fn style(_: &StyleCx<'_>, _: SheetStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> SheetOut {
        SheetOut(cx.response.clone())
    }
}

impl Container for Sheet<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }
}

/// A built-in theme under an id of its own: a library theme as the editor holds it.
fn library(builtin: &str, id: &str) -> ThemeDef {
    let mut def = builtins().iter().find(|theme| theme.id.as_str() == builtin).expect("a built-in").clone();
    def.id = ThemeId::parse(id).expect("a valid id");
    def
}

/// The panel colour and the medium control height of `def` resolved in `mode`, `contrast` and `density`.
fn expected(def: &ThemeDef, mode: Mode, contrast: Contrast, density: Size) -> (Rgba, f32) {
    let appearance = Appearance { mode, contrast, size: density, text_size: TextSize::default(), reduced_motion: false };
    let tokens = resolve(def, appearance, None).tokens;
    (tokens.color(ColorToken::SurfacePanel), tokens.size.control.md)
}

/// The panel colour and the medium control height `ui` resolves.
fn seen(ui: &Ui) -> (Rgba, f32) {
    let env = Env::of(ui);
    (env.tokens.tokens().color(ColorToken::SurfacePanel), env.tokens.size.control.md)
}

/// The window on the suite default in dark mode, standard contrast and its density.
fn dark_window() -> Frames {
    let mut frames = Frames::new();
    frames.root(RootEnv::new().mode(ModeChoice::Dark));
    frames
}

#[trial]
fn a_body_under_a_theme_scope_reads_its_theme_and_the_parent_keeps_the_windows() {
    let def = Arc::new(library("craft.studio", "mine.studio"));
    let scope = ThemeScope::new().theme(def.clone()).mode(Mode::Light).contrast(Contrast::High).density(Size::Comfortable);
    let mut frames = dark_window();
    let own_mode = Cell::new(None);
    let (mut inside, mut before, mut after) = (None, None, None);
    frames.frame(Vec::new(), |ui| {
        before = Some(seen(ui));
        Themed { own_mode: Some(&own_mode), ..Themed::new(scope.clone()) }.id_salt("themed").show(ui, |ui, _| inside = Some(seen(ui)));
        after = Some(seen(ui));
    });
    let window = before.expect("a pass ran");
    assert_eq!(inside, Some(expected(&def, Mode::Light, Contrast::High, Size::Comfortable)), "the body resolves the scope's theme and values");
    assert_eq!(own_mode.get(), Some(Mode::Light), "the container itself resolves in its scope");
    assert_ne!(inside, Some(window), "the scope's tokens differ from the window's");
    assert_eq!(after, Some(window), "the parent keeps the window's tokens after the scope");
}

#[trial]
fn a_layer_opened_inside_a_theme_scope_draws_with_the_scopes_tokens() {
    let def = Arc::new(library("craft.studio", "mine.studio"));
    let scope = ThemeScope::new().theme(def.clone()).mode(Mode::Light);
    let mut frames = dark_window();
    let (mut window, mut layer) = (None, None);
    frames.idle(3, |ui| {
        window = Some(seen(ui));
        Themed::new(scope.clone()).id_salt("themed").show(ui, |ui, _| {
            let mut open = true;
            let props = ContainerProps::new().mount(Mount::Layer(Overlay::floating()));
            Sheet { common: Common::new(), props, open: Bind::bound(&mut open) }.id_salt("sheet").show(ui, |ui, _| layer = Some(seen(ui)));
        });
    });
    let want = expected(&def, Mode::Light, Contrast::Standard, Size::default());
    assert_eq!(layer, Some(want), "the layer's body resolves its opener's scope");
    assert_ne!(layer, window, "not the window's tokens");
}

#[trial]
fn a_theme_scope_resolves_an_unchanged_definition_once_across_frames() {
    let mut frames = dark_window();
    let held = Arc::new(library("craft.studio", "mine.studio"));
    let show = |ui: &mut Ui, def: &Arc<ThemeDef>, salt: &str| {
        Themed::new(ThemeScope::new().theme(def.clone())).id_salt(salt).show(ui, |_, _| {});
    };
    frames.frame(Vec::new(), |ui| show(ui, &held, "draft"));
    let first = root_token_resolves(&frames.ctx);
    frames.idle(4, |ui| show(ui, &held, "draft"));
    assert_eq!(root_token_resolves(&frames.ctx), first, "the same definition resolves nothing more");

    let equal = Arc::new((*held).clone());
    frames.idle(2, |ui| show(ui, &equal, "draft"));
    assert_eq!(root_token_resolves(&frames.ctx), first, "an equal definition in another allocation resolves nothing more");

    let edited = Arc::new(library("craft.studio", "mine.edited"));
    frames.idle(4, |ui| show(ui, &edited, "draft"));
    assert_eq!(root_token_resolves(&frames.ctx), first + 1, "a changed definition resolves once");

    // More themed regions in one frame than the cache keeps beside the ones in use: each still resolves once.
    let many: Vec<Arc<ThemeDef>> = (0..9).map(|at| Arc::new(library("craft.studio", &format!("mine.card-{at}")))).collect();
    let show_many = |ui: &mut Ui| {
        for (at, def) in many.iter().enumerate() {
            show(ui, def, &format!("card {at}"));
        }
    };
    frames.frame(Vec::new(), show_many);
    let shown = root_token_resolves(&frames.ctx);
    frames.idle(4, show_many);
    assert_eq!(root_token_resolves(&frames.ctx), shown, "regions in use keep their resolves");
}
