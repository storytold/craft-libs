//! Gated on the `share` feature, which is off by default.
#![cfg(feature = "share")]

//! Renders of the shared controls, for review.
//!
//! The other suites read the accessibility tree and never look at pixels. That
//! catches wrong wording and missing names. It cannot show whether a dialog
//! looks like it belongs in an office suite, and these two controls will sit in
//! every Craft app, so somebody has to be able to see them.
//!
//! Writes nothing unless `CRAFT_UI_RENDERS` names a directory. A suite that
//! wrote files on every run would turn a checkout dirty by testing it.
//!
//! ```sh
//! CRAFT_UI_RENDERS=/tmp/renders cargo test -p craft-ui --all-features --test renders
//! ```

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use craft_cloud::{Capability, Contact, Permission, Share, ShareTarget};
use craft_ui::account::{Account, AccountMenu};
use craft_ui::presence::{Peer, PresenceBar};
use craft_ui::share::{Answer, Failure, Replier, Request, ShareBackend, ShareDialog};
use egui::{Context, Event, FullOutput, PointerButton, Pos2, RawInput, Rect, Theme, Visuals, accesskit, pos2, vec2};
use egui_kittest::{TestRenderer, wgpu::WgpuTestRenderer};

const DOCUMENT: &str = "files/Budget 2026.xlsx";

/// A backend that answers what a control needs and parks the rest.
#[derive(Clone, Default)]
struct Fake {
    capabilities: Vec<Capability>,
    access: Vec<Share>,
    parked: Arc<Mutex<Vec<(Request, Replier)>>>,
}

impl ShareBackend for Fake {
    fn submit(&self, request: Request, reply: Replier) {
        match &request {
            Request::Capabilities => {
                reply.answer(Answer::Capabilities { capabilities: self.capabilities.clone(), linked: true });
            }
            Request::Access { .. } => reply.answer(Answer::Access(self.access.clone())),
            _ => {
                if let Ok(mut parked) = self.parked.lock() {
                    parked.push((request, reply));
                }
            }
        }
    }
}

impl Fake {
    fn answer(&self, answer: Answer) {
        let taken = self.parked.lock().ok().and_then(|mut p| if p.is_empty() { None } else { Some(p.remove(0)) });
        if let Some((_, reply)) = taken {
            reply.answer(answer);
        }
    }
}

/// One control, driven frame by frame, with the pixels kept.
struct Shot {
    ctx: Context,
    /// One renderer per context. A renderer shared between two contexts
    /// takes a texture id from the first and is then asked by the second
    /// to free one it never registered.
    renderer: WgpuTestRenderer,
    time: f64,
    size: [f32; 2],
    typing: bool,
    /// The ground the control is painted on. Without it the render carries a
    /// transparent background, which reads as a rendering fault.
    fill: egui::Color32,
}

impl Shot {
    fn new(theme: Theme, size: [f32; 2]) -> Shot {
        let ctx = Context::default();
        ctx.enable_accesskit();
        let visuals = if theme == Theme::Dark { Visuals::dark() } else { Visuals::light() };
        let fill = visuals.panel_fill;
        ctx.set_visuals(visuals);
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.0;
            style.visuals.text_cursor.blink = false;
        });
        Shot { ctx, renderer: WgpuTestRenderer::new(), time: 0.0, size, typing: false, fill }
    }

    fn frame(&mut self, events: Vec<Event>, mut draw: impl FnMut(&mut egui::Ui)) -> FullOutput {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.size[0], self.size[1]))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        if self.typing {
            ctx.memory_mut(|m| m.request_focus(ShareDialog::query_field_id()));
        }
        let fill = self.fill;
        let size = self.size;
        let mut output = ctx.run_ui(raw, |ui| {
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(size[0], size[1]));
            ui.painter().rect_filled(screen, 0.0, fill);
            draw(ui);
        });
        // Every frame's delta has to reach the renderer. The font atlas arrives
        // on the first frame, and a renderer that saw only the last one paints
        // no text at all.
        self.renderer.handle_delta(&mut output.textures_delta);
        output
    }

    /// Write what the last frame drew.
    fn save(&mut self, into: &std::path::Path, name: &str, theme: Theme, output: &FullOutput) {
        let image = self.renderer.render(&self.ctx, output).expect("an offscreen renderer is required to write renders");
        let suffix = if theme == Theme::Dark { "dark" } else { "light" };
        let path = into.join(format!("{name}-{suffix}.png"));
        image.save(&path).expect("the render directory must be writable");
        println!("wrote {}", path.display());
    }

    /// Press the control carrying this exact accessible name.
    fn click(&mut self, label: &str, draw: &mut impl FnMut(&mut egui::Ui)) {
        let output = self.frame(Vec::new(), &mut *draw);
        let Some(at) = control(&output, label) else { panic!("nothing to click named {label:?}; on screen are {:?}", controls(&output)) };
        self.frame(vec![Event::PointerMoved(at)], &mut *draw);
        self.frame(vec![press(at, true)], &mut *draw);
        self.frame(vec![press(at, false)], &mut *draw);
    }
}

fn press(at: Pos2, down: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::default() }
}

struct Seen {
    role: accesskit::Role,
    label: String,
    bounds: Option<accesskit::Rect>,
    disabled: bool,
}

impl Seen {
    fn interactive(&self) -> bool {
        matches!(self.role, accesskit::Role::Button | accesskit::Role::TextInput | accesskit::Role::CheckBox | accesskit::Role::Link)
    }

    fn at(&self) -> Option<Pos2> {
        self.bounds.map(|b| pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
    }
}

fn seen(output: &FullOutput) -> Vec<Seen> {
    let Some(update) = &output.platform_output.accesskit_update else { return Vec::new() };
    update
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            node.label().or_else(|| node.value()).map(|text| Seen {
                role: node.role(),
                label: text.to_owned(),
                bounds: node.bounds(),
                disabled: node.is_disabled(),
            })
        })
        .collect()
}

fn control(output: &FullOutput, label: &str) -> Option<Pos2> {
    seen(output).iter().find(|s| s.interactive() && !s.disabled && s.label == label).and_then(Seen::at)
}

fn controls(output: &FullOutput) -> Vec<String> {
    seen(output).into_iter().filter(Seen::interactive).map(|s| s.label).collect()
}

fn person(id: &str, name: &str) -> Contact {
    Contact {
        id: id.into(),
        target: ShareTarget::User(id.into()),
        name: name.into(),
        email: Some(format!("{id}@example.com")),
        avatar: None,
        is_group: false,
    }
}

fn existing(id: &str, target: ShareTarget, permission: Permission) -> Share {
    Share { id: id.into(), target, display: None, permission, url: None, note: None, created: None }
}

fn ordinary() -> Vec<Capability> {
    vec![Capability::Files, Capability::Sharing, Capability::Contacts, Capability::ShareNotify, Capability::LinkShares]
}

#[test]
fn renders_of_every_state_in_both_themes() {
    let Ok(directory) = std::env::var("CRAFT_UI_RENDERS") else {
        println!("set CRAFT_UI_RENDERS to a directory to write renders");
        return;
    };
    let into = PathBuf::from(directory);
    std::fs::create_dir_all(&into).expect("the render directory must be creatable");
    for theme in [Theme::Light, Theme::Dark] {
        // ---- the account control -------------------------------------------
        let mut shot = Shot::new(theme, [320.0, 300.0]);
        // A fresh menu is a signed-out one, which is the first state.
        let mut draw = {
            let mut menu = AccountMenu::new();
            move |ui: &mut egui::Ui| menu.show(ui)
        };
        let output = shot.frame(Vec::new(), &mut draw);
        shot.save(&into, "account-signed-out", theme, &output);

        for (name, linked) in [("account-no-storage", false), ("account-linked", true)] {
            let mut shot = Shot::new(theme, [320.0, 300.0]);
            let mut draw = {
                let mut menu = AccountMenu::new();
                menu.set(vec![
                    Account {
                        id: "work".into(),
                        display: "Will Dunn".into(),
                        email: Some("will@example.com".into()),
                        service: "collabcraft.example.com".into(),
                        linked,
                    },
                    Account {
                        id: "personal".into(),
                        display: "Will at home".into(),
                        email: Some("will@example.net".into()),
                        service: "cloud.example.net".into(),
                        linked: true,
                    },
                ]);
                move |ui: &mut egui::Ui| menu.show(ui)
            };
            // The menu is behind the badge, so it takes a press to show.
            shot.click("Account, Will Dunn", &mut draw);
            let output = shot.frame(Vec::new(), &mut draw);
            shot.save(&into, name, theme, &output);
        }

        // ---- who else is here ----------------------------------------------
        let mut shot = Shot::new(theme, [420.0, 80.0]);
        let mut bar = PresenceBar::new();
        bar.set(vec![
            Peer::new("s1", "Ann Reed").at("editing B4"),
            Peer::new("s2", "Bob Vine").at("editing D12"),
            Peer::new("s3", "Cal Nye"),
            Peer::new("s4", "Dee Shaw"),
            Peer::new("s5", "Eve Marsh"),
            Peer::new("s6", "Fay Orr"),
        ]);
        let mut draw = move |ui: &mut egui::Ui| {
            ui.add_space(24.0);
            bar.show(ui);
        };
        let output = shot.frame(Vec::new(), &mut draw);
        shot.save(&into, "presence", theme, &output);

        // ---- the share dialog ----------------------------------------------
        let cases: Vec<(&str, Vec<Capability>, Vec<Share>, &str)> = vec![
            ("dialog-fresh", ordinary(), Vec::new(), DOCUMENT),
            (
                "dialog-who-has-access",
                ordinary(),
                vec![
                    existing("s1", ShareTarget::User("ann".into()), Permission::Edit),
                    existing("s2", ShareTarget::Group("finance".into()), Permission::View),
                ],
                DOCUMENT,
            ),
            ("dialog-no-link-shares", vec![Capability::Sharing, Capability::Contacts], Vec::new(), DOCUMENT),
            ("dialog-unsaved", ordinary(), Vec::new(), ""),
        ];

        for (name, capabilities, access, id) in cases {
            let mut shot = Shot::new(theme, [620.0, 560.0]);
            let fake = Fake { capabilities, access, parked: Arc::default() };
            let mut dialog = ShareDialog::new();
            dialog.connect(fake.clone());
            let held = Rc::new(RefCell::new(dialog));
            let mut draw = {
                let held = Rc::clone(&held);
                move |ui: &mut egui::Ui| held.borrow_mut().show_in(ui)
            };
            // One frame asks what the provider can do, the next reads the answer.
            shot.frame(Vec::new(), &mut draw);
            shot.frame(Vec::new(), &mut draw);
            held.borrow_mut().open(id.to_owned(), "Budget 2026");
            shot.frame(Vec::new(), &mut draw);
            let output = shot.frame(Vec::new(), &mut draw);
            shot.save(&into, name, theme, &output);
        }

        // ---- recipients picked, then one of them refused -------------------
        let fake = Fake { capabilities: ordinary(), access: Vec::new(), parked: Arc::default() };
        let mut dialog = ShareDialog::new();
        dialog.connect(fake.clone());
        let held = Rc::new(RefCell::new(dialog));
        let mut draw = {
            let held = Rc::clone(&held);
            move |ui: &mut egui::Ui| held.borrow_mut().show_in(ui)
        };
        let mut shot = Shot::new(theme, [620.0, 560.0]);
        // One frame asks what the provider can do, the next reads the answer.
        // Opening before that answer arrives does nothing, which is the point
        // of a control that refuses to appear when it cannot work.
        shot.frame(Vec::new(), &mut draw);
        shot.frame(Vec::new(), &mut draw);
        held.borrow_mut().open(DOCUMENT.to_owned(), "Budget 2026");
        shot.frame(Vec::new(), &mut draw);
        shot.typing = true;
        shot.frame(vec![Event::Text("an".to_owned())], &mut draw);
        shot.time += 0.25;
        shot.frame(Vec::new(), &mut draw);
        fake.answer(Answer::Lookup(vec![person("ann", "Ann Reed")]));
        shot.frame(Vec::new(), &mut draw);
        shot.click("Ann Reed", &mut draw);
        shot.typing = false;
        let output = shot.frame(Vec::new(), &mut draw);
        shot.save(&into, "dialog-recipients", theme, &output);

        shot.click("Send", &mut draw);
        fake.answer(Answer::Granted {
            shared: vec![existing("s1", ShareTarget::User("ann".into()), Permission::View)],
            failed: vec![Failure {
                target: ShareTarget::User("bob".into()),
                code: "invalid_address".into(),
                detail: "bob@example.con was rejected".into(),
            }],
        });
        let output = shot.frame(Vec::new(), &mut draw);
        shot.save(&into, "dialog-partial-failure", theme, &output);
    }
}
