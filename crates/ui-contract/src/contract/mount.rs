//! The mount host (16.2): where steps 6 to 16 of a container run. `Inline` runs in the parent's `Ui`; the others
//! open a `Ui` of their own and call `pass` once for each pass that `Ui` gets.

use super::body::shown_placement;
use super::container::PaneEdge;
use super::overlay::{self, FloatPlace, Layer, Outside, Overlay, Place};
use super::run::{Host, HostCx, HostPass, ShowAt};
use super::{ContainerProps, Mount, ShellPane};
use crate::runtime::egui::areas;
use crate::runtime::egui::panels::{self, Edge};
use crate::runtime::env::{Env, Label, ShellEntry};
use crate::runtime::focus::FocusPolicy;
use crate::runtime::layout_memory;
use crate::runtime::memory::{MemoryStage, SaveKey, sub};
use crate::runtime::motion::{self, Presence};
use crate::runtime::overlay::{Backdrop, CloseReason, Rank};
use crate::runtime::windows::{self, ViewportOpen, WindowHost, WindowSpec};
use craft_behavior::ScopeKind;
use egui::{Id, Pos2, Rect, Ui, Vec2};

/// The host a mount chose, and whether it shows the container fading out.
pub(super) struct Mounted {
    pub host: Box<dyn Host>,
    /// The container is closed and its layer is still fading: the body still shows, as it was.
    pub exiting: bool,
}

/// The host for `props.mount`. A layer-like mount of a closed container shows nothing once its exit fade ended.
pub(super) fn host(props: &ContainerProps, at: &HostCx<'_>, open: bool, title: &Label) -> Mounted {
    let ctx = at.ui.ctx();
    let layer = |overlay: &Overlay| {
        let presence = if at.ui.is_sizing_pass() {
            Presence { t: 1.0, visible: open, exiting: false }
        } else {
            motion::layer_presence(ctx, sub(at.id, "presence"), at.id, at.env.motion, open, overlay.motion)
        };
        (presence.visible.then_some(presence.t), !open)
    };
    let closed = || Mounted { host: Box::new(ClosedHost), exiting: false };
    let mounted = match &props.mount {
        Mount::Inline => Mounted { host: Box::new(InlineHost), exiting: false },
        Mount::Pane(pane) => Mounted { host: Box::new(PaneHost(*pane)), exiting: false },
        Mount::Window { .. } => Mounted { host: Box::new(WindowContentHost), exiting: false },
        Mount::Layer(overlay) => match layer(overlay) {
            (Some(fade), exiting) => Mounted { host: Box::new(LayerHost::new(overlay.clone(), props.save.clone(), exiting, fade, at.size)), exiting },
            (None, _) => closed(),
        },
        Mount::FullWindow => match layer(&full_window_overlay(props.focus)) {
            (Some(fade), exiting) => Mounted { host: Box::new(FullWindowHost { focus: props.focus, exiting, fade, dismissed: None }), exiting },
            (None, _) => closed(),
        },
        Mount::Viewport { fallback, spec } => {
            if !WindowHost::of(ctx).viewports_available() {
                match layer(fallback) {
                    (Some(fade), exiting) => {
                        Mounted { host: Box::new(LayerHost::new(fallback.clone(), props.save.clone(), exiting, fade, at.size)), exiting }
                    }
                    (None, _) => closed(),
                }
            } else if open {
                let viewport = ViewportHost {
                    overlay: fallback.clone(),
                    title: title.clone(),
                    save: props.save.as_ref().map(|key| key.as_str().to_owned()),
                    spec: spec.clone(),
                    dismissed: None,
                };
                Mounted { host: Box::new(viewport), exiting: false }
            } else {
                closed()
            }
        }
    };
    if (open || mounted.exiting) && opens_dialog(&props.mount) {
        if let Some(shell) = at.env.get::<ShellEntry>().0 {
            shell.note_dialog();
        }
    }
    mounted
}

/// Whether a shown container of `mount` is a dialog, a modal or a window for the shell it shows in
/// (`ShellCx::dialog_shown`): an OS window or its fallback, a full-window layer, or a layer with a dialog's focus
/// scope, modal or not. Menus, list boxes, popovers, tooltips and floating panes are not.
fn opens_dialog(mount: &Mount) -> bool {
    match mount {
        Mount::Viewport { .. } | Mount::FullWindow => true,
        Mount::Layer(overlay) => overlay.focus.kind == ScopeKind::Dialog,
        Mount::Inline | Mount::Pane(_) | Mount::Window { .. } => false,
    }
}

/// A closed layer-like container: nothing shows and no layer is recorded. The layer forgets what it remembered while
/// shown, so a container closed through its binding, with no exit fade, also closes the popups opened from its body.
struct ClosedHost;

impl Host for ClosedHost {
    fn run(&mut self, id: Id, parent: &mut Ui, _: &Env, _: &mut HostPass<'_>) {
        if !parent.is_sizing_pass() {
            overlay::forget_layer(parent.ctx(), id, id);
        }
    }
}

/// The parent `Ui` is the host `Ui`.
struct InlineHost;

impl Host for InlineHost {
    fn run(&mut self, _: Id, parent: &mut Ui, _: &Env, pass: &mut HostPass<'_>) {
        pass(parent, None, None);
    }
}

/// The container fills the room of the `Ui` it is shown in: the window's content rect for the window's root `Ui`.
struct WindowContentHost;

impl Host for WindowContentHost {
    fn run(&mut self, _: Id, parent: &mut Ui, _: &Env, pass: &mut HostPass<'_>) {
        let window = parent.max_rect();
        pass(parent, Some(ShowAt::Rect(window)), None);
        crate::runtime::egui::alloc::advance_past(parent, window);
    }
}

/// A pane slot of the shell: the one place `egui::Panel` and `CentralPanel` are called.
struct PaneHost(ShellPane);

/// The pane memory of an edge pane with no fixed extent: the size it first took, or the extent the user gave it with
/// its grip.
pub(super) fn pane_extent_id(id: Id) -> Id {
    sub(id, "pane_extent")
}

/// The grip gesture of an edge pane, in shown memory: the extent the pane had when the gesture began and the extent the
/// pointer gives it. The body step writes it; the pane host shows the gesture's extent while it runs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PaneGrip {
    pub began: Option<f32>,
    pub extent: Option<f32>,
}

pub(super) fn pane_grip_id(id: Id) -> Id {
    sub(id, "pane_grip")
}

impl Host for PaneHost {
    fn run(&mut self, id: Id, parent: &mut Ui, env: &Env, pass: &mut HostPass<'_>) {
        // A pane the caller laid out shows in its rect and leaves the room of its siblings alone.
        if let Some(rect) = self.0.at {
            pass(parent, Some(ShowAt::Rect(rect)), None);
            return;
        }
        let rtl = env.direction.is_rtl();
        let (edge, along_x) = match self.0.edge {
            PaneEdge::Start => (if rtl { Edge::Right } else { Edge::Left }, true),
            PaneEdge::End => (if rtl { Edge::Left } else { Edge::Right }, true),
            PaneEdge::Top => (Edge::Top, false),
            PaneEdge::Bottom => (Edge::Bottom, false),
            _ => (Edge::Center, false),
        };
        let ctx = parent.ctx().clone();
        // A grip gesture's extent wins while it runs. Else a fixed extent, else the pane's own memory, seeded from the
        // size the pane first took and kept until the user resizes it: the body's size does not carry it, since a
        // filling body takes what the pane gives.
        let extent = pane_extent_id(id);
        let grip = MemoryStage::peek::<PaneGrip>(&ctx, pane_grip_id(id)).extent;
        let last = grip.or(self.0.extent).or_else(|| layout_memory::read(&ctx, extent).map(|size| if along_x { size.x } else { size.y }));
        let outer = panels::show_pane(parent, edge, sub(id, ("pane", self.0.name)), last, |ui| {
            // The pane is a layout room of its own: the grid or content column its parent body flows in ends here.
            // It fills its panel's rect, so no rule of the parent's flow (a content column holding a filling child at
            // its measured width) sizes it, and the panel keeps the extent it offered.
            let room = env.new_room();
            let installed = Env::install_for(ui, room.clone());
            let panel = ui.max_rect();
            pass(ui, Some(ShowAt::Rect(panel)), Some(&room));
            crate::runtime::egui::alloc::advance_past(ui, panel);
            Env::restore(&ui.ctx().clone(), installed);
        });
        if last.is_none() && edge != Edge::Center && outer.is_positive() {
            layout_memory::record(&ctx, extent, outer.size());
        }
    }
}

/// A layer through the overlay host, at the container's placement: the overlay's own place until the user moves it.
/// The placement lives in the container's saved slot when it has a `SaveKey`, else in shown memory.
struct LayerHost {
    overlay: Overlay,
    save: Option<SaveKey>,
    exiting: bool,
    fade: f32,
    /// The size the container measured for this pass.
    size: Vec2,
    dismissed: Option<CloseReason>,
}

impl LayerHost {
    fn new(overlay: Overlay, save: Option<SaveKey>, exiting: bool, fade: f32, size: Vec2) -> Self {
        LayerHost { overlay, save, exiting, fade, size, dismissed: None }
    }
}

impl Host for LayerHost {
    fn run(&mut self, id: Id, parent: &mut Ui, env: &Env, pass: &mut HostPass<'_>) {
        // A sizing pass of the parent shows no layer: a layer takes no room in its parent.
        if parent.is_sizing_pass() {
            return;
        }
        let ctx = parent.ctx().clone();
        // A resizable layer with a default size is placed by it until the user or the caller sizes it.
        let region = overlay::region_for(env, &self.overlay);
        let mut overlay = if region.is_some() { self.overlay.in_region() } else { self.overlay.clone() };
        let screen = overlay::screen_for(&ctx, env, &overlay);
        let placement = shown_placement(&ctx, id, self.save.as_ref(), &overlay, screen, env.tokens.space.inset.screen);
        // An anchored resizable layer has no position to seed: its default size is held as its minimum until the
        // user resizes it.
        if overlay.fit == overlay::OverlayFit::Resizable
            && placement.size.is_none()
            && let Some(size) = overlay.size
        {
            overlay.size_limits.min = overlay.size_limits.min.max(size);
        }
        if let Some(pos) = placement.pos {
            overlay.place = Place::Float(FloatPlace::at(Pos2::from(pos)));
        }
        let at = placement.pos.zip(placement.size).map(|(pos, size)| ShowAt::Rect(Rect::from_min_size(Pos2::from(pos), Vec2::from(size))));
        // Without an opener, an anchor given as a rect or point is in the parent's layer, and `Response` is the
        // parent's cursor.
        let own = Rect::from_min_max(parent.cursor().min, parent.cursor().min);
        let anchor = overlay::resolve_anchor(&ctx, id, &overlay, own, areas::layer_to_screen(&ctx, parent.layer_id()));
        // The layer is placed by the size the container takes in this pass: its placed rect, else what it measured.
        let size = at.map_or(self.size, |shown| match shown {
            ShowAt::Rect(rect) => rect.size(),
            ShowAt::Flow => self.size,
        });
        let layer = Layer { id, owner: id, overlay: &overlay, anchor, env, exiting: self.exiting, fade: self.fade, size: Some(size), region };
        let out = overlay::host(&ctx, &layer, &mut |ui| {
            // The container runs in the env the layer installed, not in its opener's.
            pass(ui, Some(at.unwrap_or(ShowAt::Flow)), Some(&Env::of(ui)));
            // A body shown at a rect takes no room in the layer's flow: the layer covers the rect, so the pointer
            // finds the layer under the body and its rect holds what the body paints.
            if let Some(ShowAt::Rect(rect)) = at {
                crate::runtime::egui::alloc::advance_past(ui, rect);
            }
        });
        self.dismissed = out.dismissed;
    }

    fn dismissed(&self) -> Option<CloseReason> {
        self.dismissed
    }
}

/// The overlay of a full-window mount: a modal rank with no backdrop (the dialog covers the window) and the
/// container's focus policy as its scope, so the layer stack gives it the key class, blocking and dismissal rules
/// of a modal.
pub(super) fn full_window_overlay(focus: FocusPolicy) -> Overlay {
    let mut overlay = Overlay::modal().fit(overlay::OverlayFit::Content).focus(focus);
    overlay.rank = Rank::modal(Backdrop::None);
    overlay
}

/// Over the whole window, on a layer of its own.
struct FullWindowHost {
    focus: FocusPolicy,
    exiting: bool,
    fade: f32,
    dismissed: Option<CloseReason>,
}

impl Host for FullWindowHost {
    fn run(&mut self, id: Id, parent: &mut Ui, env: &Env, pass: &mut HostPass<'_>) {
        if parent.is_sizing_pass() {
            return;
        }
        let ctx = parent.ctx().clone();
        // Opened in an embedded shell, the full-window layer fills the shell's region, as a modal there does.
        let region = overlay::region_for(env, &full_window_overlay(self.focus));
        let window = region.unwrap_or_else(|| ctx.content_rect());
        let overlay = full_window_overlay(self.focus).place(Place::Float(FloatPlace::at(window.min)));
        let layer = Layer { id, owner: id, overlay: &overlay, anchor: Rect::ZERO, env, exiting: self.exiting, fade: self.fade, size: None, region };
        // The body is shown at the window's rect, which takes no room in the layer; the layer claims the window, so its
        // recorded rect is the window's.
        let out = overlay::host(&ctx, &layer, &mut |ui| {
            ui.set_min_size(window.size());
            pass(ui, Some(ShowAt::Rect(window)), Some(&Env::of(ui)));
        });
        self.dismissed = out.dismissed;
    }

    fn dismissed(&self) -> Option<CloseReason> {
        self.dismissed
    }
}

/// An OS viewport of the integration; the fallback layer is `LayerHost`. The fallback overlay's rank, key class,
/// focus scope and dismissal rules (but for outside presses, which an OS window has no outside for) apply to the
/// window as they do to the layer: the window is an entry of its viewport's layer stack.
struct ViewportHost {
    overlay: Overlay,
    title: Label,
    /// The key the window's placement is saved under.
    save: Option<String>,
    /// How the window opens; none opens by the fallback's rank.
    spec: Option<WindowSpec>,
    dismissed: Option<CloseReason>,
}

impl Host for ViewportHost {
    fn run(&mut self, id: Id, parent: &mut Ui, env: &Env, pass: &mut HostPass<'_>) {
        // A sizing pass of the parent shows no window: an OS window takes no room in its parent.
        if parent.is_sizing_pass() {
            return;
        }
        let ctx = parent.ctx().clone();
        let focus = self.overlay.focus;
        let mut window_rules = self.overlay.clone();
        window_rules.dismiss.outside = Outside::Keep;
        let title = env.view().resolve(&self.title);
        // Without a spec, a modal fallback is a dialog window and anything else a tool window. The size limits,
        // the position and the resizability are the fallback overlay's, whichever form the container takes.
        let by_rank = || if self.overlay.rank.is_modal() { WindowSpec::dialog() } else { WindowSpec::tool() };
        let spec = self.spec.clone().unwrap_or_else(by_rank);
        let placed = self.overlay.placed;
        let open = ViewportOpen {
            id,
            title: title.as_str(),
            size: layout_memory::read(&ctx, id).or(placed.map(|rect| rect.size())),
            save: self.save.as_deref(),
            spec: &spec,
            limits: (Some(self.overlay.size_limits.min), self.overlay.size_limits.max),
            position: placed.map(|rect| rect.min),
            resizable: self.overlay.resize.grips,
        };
        let dismissed = &mut self.dismissed;
        windows::show_viewport(&ctx, &open, |ui| {
            // The container's scope on the child viewport's own root: its scale, active state and window facts.
            let opener = env.capture().env();
            let scoped_env = Env::root_of(&ctx).map_or_else(|| opener.clone(), |root| opener.rebase(&root));
            let installed = Env::install_for(ui, scoped_env.clone());
            let window = ui.max_rect();
            let real = !ui.is_sizing_pass();
            let scoped = focus != FocusPolicy::NONE && real;
            if scoped {
                crate::runtime::focus::push_scope(&ctx, id, focus, None);
            }
            if real {
                crate::runtime::overlay::begin_layer(&ctx, id, false);
            }
            pass(ui, Some(ShowAt::Rect(window)), Some(&scoped_env));
            if real {
                crate::runtime::overlay::end_layer(&ctx);
            }
            if scoped {
                crate::runtime::focus::pop_scope(&ctx);
            }
            if real {
                let layer = Layer {
                    id,
                    owner: id,
                    overlay: &window_rules,
                    anchor: Rect::ZERO,
                    env: &scoped_env,
                    exiting: false,
                    fade: 1.0,
                    size: None,
                    region: None,
                };
                *dismissed = overlay::host_window(&ctx, &layer, window).dismissed;
            }
            Env::restore(&ctx, installed);
        });
    }

    fn dismissed(&self) -> Option<CloseReason> {
        self.dismissed
    }
}
