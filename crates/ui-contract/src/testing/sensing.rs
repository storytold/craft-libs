//! Probe components for the sensing core: roots, members and parts described as data, shown through the same
//! registration, lifecycle and response paths a component's pipeline uses.

use crate::primitives::sensing::{self, MemberSpec, PartDecl, PartResult, RootSpec};
use crate::runtime::cursor;
use crate::runtime::egui::areas::{self, Forward};
use crate::runtime::egui::{fixture, response};
use crate::runtime::env::ArenaId;
use crate::runtime::memory::sub;
use crate::runtime::pointer;
use craft_behavior::focus::{PressFocus, classify_press};
use craft_behavior::{CanvasTransform, Cursor, CursorIcon, CursorPriority, HitLevel, HitPriority, PartKey, PressPhase, PressState, Sense};
use egui::emath::TSTransform;
use egui::{Context, Id, Pos2, Rect, Ui, Vec2};

/// A part of a probe.
#[derive(Clone, Debug)]
pub struct ProbePart {
    key: PartKey,
    index: Option<u64>,
    rect: Rect,
    sense: Sense,
    priority: HitPriority,
    over: bool,
    cursor: Option<CursorIcon>,
}

impl ProbePart {
    /// A part `key` over `rect` sensing `sense`.
    pub fn new(key: PartKey, rect: Rect, sense: Sense) -> Self {
        ProbePart { key, index: None, rect, sense, priority: HitPriority::BODY, over: false, cursor: None }
    }

    /// The part's tier among the shapes of its stack position.
    pub fn level(mut self, level: HitLevel, sub: u8) -> Self {
        self.priority = HitPriority::new(level, sub);
        self
    }

    /// The part sits above the body members.
    pub fn over(mut self) -> Self {
        self.over = true;
        self
    }

    /// The cursor the part asks for while hovered.
    pub fn cursor(mut self, icon: CursorIcon) -> Self {
        self.cursor = Some(icon);
        self
    }

    /// One of a repeated part.
    pub fn at(mut self, index: u64) -> Self {
        self.index = Some(index);
        self
    }

    fn decl(&self) -> PartDecl {
        let mut decl = PartDecl::new(self.key, self.rect, self.sense);
        decl.index = self.index;
        decl.priority = self.priority;
        decl.over = self.over;
        decl.cursor = self.cursor.map(Cursor::System);
        decl
    }
}

/// A member of a probe arena, or a body child that is a root of its own in a probe that is no arena.
#[derive(Clone, Debug)]
pub struct ProbeMember {
    name: &'static str,
    rect: Rect,
    sense: Sense,
    parts: Vec<ProbePart>,
}

impl ProbeMember {
    /// A member named `name` over `rect` sensing `sense`.
    pub fn new(name: &'static str, rect: Rect, sense: Sense) -> Self {
        ProbeMember { name, rect, sense, parts: Vec::new() }
    }

    /// Adds a part.
    pub fn part(mut self, part: ProbePart) -> Self {
        self.parts.push(part);
        self
    }
}

/// A root: a leaf component, or with `arena` an arena root whose members join it.
#[derive(Clone, Debug)]
pub struct ProbeRoot {
    name: &'static str,
    rect: Rect,
    sense: Sense,
    parts: Vec<ProbePart>,
    members: Vec<ProbeMember>,
    arena: bool,
    transform: Option<CanvasTransform>,
    blocker: Option<Vec2>,
    enabled: bool,
    cursor: Option<(Cursor, CursorPriority)>,
}

impl ProbeRoot {
    /// A root named `name` over `rect` sensing `sense`.
    pub fn new(name: &'static str, rect: Rect, sense: Sense) -> Self {
        ProbeRoot {
            name,
            rect,
            sense,
            parts: Vec::new(),
            members: Vec::new(),
            arena: false,
            transform: None,
            blocker: None,
            enabled: true,
            cursor: None,
        }
    }

    /// Adds an own part.
    pub fn part(mut self, part: ProbePart) -> Self {
        self.parts.push(part);
        self
    }

    /// Adds a body member.
    pub fn member(mut self, member: ProbeMember) -> Self {
        self.members.push(member);
        self
    }

    /// Makes the root an arena: its members join it and have no egui widget.
    pub fn arena(mut self) -> Self {
        self.arena = true;
        self
    }

    /// Shows the body through `transform` on its own egui layer (an arena only).
    pub fn transformed(mut self, transform: CanvasTransform) -> Self {
        self.transform = Some(transform);
        self
    }

    /// Adds an allocating egui widget of `size` over the body, as app code may.
    pub fn blocker(mut self, size: Vec2) -> Self {
        self.blocker = Some(size);
        self
    }

    /// A cursor the root requests while the pointer is over it, as a tool does.
    pub fn requests(mut self, cursor: impl Into<Cursor>, priority: CursorPriority) -> Self {
        self.cursor = Some((cursor.into(), priority));
        self
    }

    /// Turns the root disabled.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// The id of the root named `name`.
    pub fn id(name: &'static str) -> Id {
        id_of(name)
    }

    /// The id of a transformed root's forwarding widget.
    pub fn transform_widget(name: &'static str) -> Id {
        sub(id_of(name), "transform")
    }
}

fn id_of(name: &'static str) -> Id {
    crate::runtime::egui::memory::unique(name)
}

/// What one part read this pass.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PartReport {
    /// The part.
    pub key: PartKey,
    /// Its repeat.
    pub index: Option<u64>,
    /// Its press state.
    pub press: PressState,
    /// The pointer is over it.
    pub hovered: bool,
}

/// What a member, or a root's own shape and parts, read this pass.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SenseReport {
    /// The component's id.
    pub id: Id,
    /// The press state of its own shape.
    pub press: PressState,
    /// Its parts.
    pub parts: Vec<PartReport>,
    /// A click activated the component's own shape or one of its parts.
    pub activated: bool,
    /// The `Response` reports a click.
    pub clicked: bool,
    /// The `Response` reports a hover.
    pub hovered: bool,
}

impl SenseReport {
    /// The press phase of the part `key`, `Idle` when the part is unknown.
    pub fn phase(&self, key: PartKey) -> PressPhase {
        self.parts.iter().find(|part| part.key == key).map_or(PressPhase::Idle, |part| part.press.phase)
    }
}

/// What a root and its members read this pass.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RootReport {
    /// The root.
    pub root: SenseReport,
    /// The member a click activated.
    pub activated_member: Option<Id>,
    /// The members, in show order.
    pub members: Vec<SenseReport>,
    /// The body layer's transform, for a transformed root.
    pub body: Option<TSTransform>,
}

impl RootReport {
    /// Where a point of the body appears on screen.
    pub fn to_screen(&self, local: Pos2) -> Pos2 {
        self.body.map_or(local, |transform| transform * local)
    }

    /// The report of the member named `name`.
    pub fn member(&self, name: &'static str) -> Option<&SenseReport> {
        self.members.iter().find(|member| member.id == id_of(name))
    }
}

fn parts_report(decls: &[PartDecl], results: &std::collections::HashMap<(PartKey, Option<u64>), PartResult>) -> Vec<PartReport> {
    decls
        .iter()
        .filter_map(|decl| {
            results.get(&(decl.key, decl.index)).map(|result| PartReport {
                key: decl.key,
                index: decl.index,
                press: result.press,
                hovered: result.hovered,
            })
        })
        .collect()
}

fn member_decls(parts: &[ProbePart]) -> Vec<PartDecl> {
    parts.iter().map(ProbePart::decl).collect()
}

fn request_cursor(ctx: &Context, decls: &[PartDecl], results: &std::collections::HashMap<(PartKey, Option<u64>), PartResult>) {
    if let Some(hovered) = sensing::hovered_cursor(decls, results) {
        cursor::request(ctx, hovered, CursorPriority::Part);
    }
}

fn show_members(ui: &mut Ui, arena: ArenaId, root: &ProbeRoot, ctx: &Context) -> Vec<SenseReport> {
    root.members
        .iter()
        .map(|member| {
            let decls = member_decls(&member.parts);
            let id = id_of(member.name);
            let result =
                sensing::sense_member(ui, arena, MemberSpec { id, rect: member.rect, sense: member.sense, parts: &decls, enabled: root.enabled });
            request_cursor(ctx, &decls, &result.parts);
            SenseReport {
                id,
                press: result.press,
                parts: parts_report(&decls, &result.parts),
                activated: result.activated,
                clicked: fixture::clicked(&result.response),
                hovered: result.response.hovered(),
            }
        })
        .collect()
}

fn show_leaves(ui: &mut Ui, root: &ProbeRoot, ctx: &Context) -> Vec<SenseReport> {
    root.members
        .iter()
        .map(|member| {
            let decls = member_decls(&member.parts);
            let id = id_of(member.name);
            let result = sensing::sense_root(ui, RootSpec { id, rect: member.rect, own_sense: member.sense, own: &decls, enabled: root.enabled });
            request_cursor(ctx, &decls, &result.parts);
            SenseReport {
                id,
                press: result.press,
                parts: parts_report(&decls, &result.parts),
                activated: result.activation || fixture::clicked(&result.response),
                clicked: fixture::clicked(&result.response),
                hovered: result.response.hovered(),
            }
        })
        .collect()
}

/// Shows `roots` in order on `ui`, each through the sensing core, and reads back what each saw.
pub fn show(ui: &mut Ui, roots: &[ProbeRoot]) -> Vec<RootReport> {
    let ctx = ui.ctx().clone();
    roots.iter().map(|root| show_root(ui, &ctx, root)).collect()
}

fn show_root(ui: &mut Ui, ctx: &Context, root: &ProbeRoot) -> RootReport {
    let id = id_of(root.name);
    let decls = member_decls(&root.parts);
    let result = sensing::sense_root(ui, RootSpec { id, rect: root.rect, own_sense: root.sense, own: &decls, enabled: root.enabled });
    request_cursor(ctx, &decls, &result.parts);
    if let Some((cursor, priority)) = &root.cursor
        && result.response.hovered()
    {
        cursor::request(ctx, cursor.clone(), *priority);
    }
    let arena = ArenaId::new(id);
    let mut body = None;
    let members = if !root.arena {
        show_leaves(ui, root, ctx)
    } else if let Some(transform) = root.transform {
        let parent = ui.layer_id();
        let layer = areas::body_layer(parent, id);
        let map = areas::body_transform(root.rect.min, transform.scale, transform.offset);
        body = Some(map);
        sensing::set_layer(ctx, arena, layer);
        let visible = root.rect.intersect(ui.clip_rect());
        let forward = Forward { id: sub(id, "transform"), sense: response::to_egui(result.sense), rect: visible };
        let body_layer = areas::BodyLayer { parent, layer, transform: map, at: root.rect.min, clip: visible, forward: Some(forward) };
        areas::show_body_layer(ctx, body_layer, |body_ui| {
            if let Some(size) = root.blocker {
                fixture::allocate_blocker(body_ui, size);
            }
            show_members(body_ui, arena, root, ctx)
        })
    } else {
        show_members(ui, arena, root, ctx)
    };
    // The owner places its `over` parts once its body has shown: above its members.
    sensing::place_over_widgets(ui, id, None, &decls);
    RootReport {
        root: SenseReport {
            id,
            press: result.press,
            parts: parts_report(&decls, &result.parts),
            activated: result.activation,
            clicked: fixture::clicked(&result.response),
            hovered: result.response.hovered(),
        },
        activated_member: result.activated_member,
        members,
        body,
    }
}

/// The widget id egui's hit test chose as the click target at the latest pointer position.
pub fn egui_click_target(ctx: &Context) -> Option<Id> {
    fixture::click_target(ctx)
}

/// How the focus service classifies a press at `at`, from last pass's shape record and egui's hit test at the
/// latest pointer position, so the pointer must have moved to `at` in a frame before; `None` when no shape of the
/// root egui hands the press to takes it.
pub fn press_focus(ctx: &Context, at: Pos2) -> Option<PressFocus> {
    let (click, drag) = crate::runtime::egui::input::hit_targets(ctx);
    pointer::press_winner(ctx, at, pointer::PressTargets { click, drag }).map(|(winner, _)| classify_press(Some(winner.sense)))
}

/// Shows an egui area of `size` over everything else, as a popup does.
pub fn cover(ctx: &Context, size: Vec2) {
    areas::show_cover(ctx, "probe_cover", |ui| fixture::allocate_blocker(ui, size));
}
