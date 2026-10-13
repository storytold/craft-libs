//! Who is signed in, and how to stop being signed in.
//!
//! Every office suite puts this in the same place, for the same reason: a
//! document is about to be saved somewhere and shared with somebody, and the
//! one thing that decides where and as whom is the account. An application
//! that never shows it leaves the user guessing which of their identities is
//! in play.
//!
//! Three states, and the control has to read differently in each. Nobody is
//! signed in. Somebody is signed in but has linked no storage, so Open and
//! Save have nowhere to go. Somebody is signed in and linked, which is the
//! ordinary case and the quietest one.
//!
//! This holds no credential. It asks the service who the caller is and shows
//! the answer, and signing out is the service forgetting the session rather
//! than this forgetting a token.

use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Align, Context, Layout, RichText, Ui};

/// What the service says about the signed-in user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    /// What to show. A display name, never an identity.
    pub display: String,
    pub email: Option<String>,
    /// Where they are signed in, such as the service's host. Shown because
    /// somebody with accounts on two deployments needs to tell them apart.
    pub service: String,
    /// Whether storage is linked. Without it Open and Save have nowhere to go,
    /// and saying so here is kinder than a failure at the moment of saving.
    pub linked: bool,
}

impl Account {
    /// The letter on the badge. The first character of the display name, which
    /// is what every suite uses and what people recognise at that size.
    pub fn initial(&self) -> String {
        self.display.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".to_owned())
    }
}

/// What the menu asks the application to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountAction {
    /// Begin a sign-in. The application opens a browser and polls.
    SignIn,
    /// End the session, at the service and here.
    SignOut,
    /// Connect storage for an account that has none.
    Link,
    /// Open the service's own account page.
    Manage,
}

/// Strings, so an application can translate them.
#[derive(Clone, Debug)]
pub struct AccountLabels {
    pub signed_out: String,
    pub sign_in: String,
    pub sign_out: String,
    pub link: String,
    pub linked: String,
    pub not_linked: String,
    pub manage: String,
    pub working: String,
}

impl Default for AccountLabels {
    fn default() -> AccountLabels {
        AccountLabels {
            signed_out: "Not signed in".into(),
            sign_in: "Sign in".into(),
            sign_out: "Sign out".into(),
            link: "Connect storage".into(),
            linked: "Storage connected".into(),
            not_linked: "No storage connected".into(),
            manage: "Account settings".into(),
            working: "Working\u{2026}".into(),
        }
    }
}

/// The account badge and its menu.
pub struct AccountMenu {
    account: Option<Account>,
    labels: AccountLabels,
    /// Set while a sign-in or sign-out is in flight, so the control says so
    /// rather than looking unresponsive.
    busy: bool,
    actions: (Sender<AccountAction>, Receiver<AccountAction>),
}

impl Default for AccountMenu {
    fn default() -> AccountMenu {
        AccountMenu::new()
    }
}

impl AccountMenu {
    pub fn new() -> AccountMenu {
        AccountMenu { account: None, labels: AccountLabels::default(), busy: false, actions: channel() }
    }

    pub fn with_labels(mut self, labels: AccountLabels) -> AccountMenu {
        self.labels = labels;
        self
    }

    /// Tell the menu who is signed in. `None` means nobody.
    pub fn set(&mut self, account: Option<Account>) {
        self.account = account;
        self.busy = false;
    }

    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    pub fn is_signed_in(&self) -> bool {
        self.account.is_some()
    }

    /// Mark something as in flight. Cleared by the next [`AccountMenu::set`].
    pub fn working(&mut self) {
        self.busy = true;
    }

    /// What the user asked for since the last call, if anything.
    ///
    /// Drained rather than handed a callback, so the application acts on its
    /// own thread and this stays free of any notion of what signing in means.
    pub fn take_action(&mut self) -> Option<AccountAction> {
        self.actions.1.try_recv().ok()
    }

    /// Draw the badge. Put it where the window's own controls are.
    pub fn show(&mut self, ui: &mut Ui) {
        let tx = self.actions.0.clone();
        let labels = self.labels.clone();
        let busy = self.busy;

        let Some(account) = self.account.clone() else {
            // Signed out. A word, because a bare badge with no name on it says
            // nothing about what pressing it would do.
            let label = if busy { labels.working.as_str() } else { labels.sign_in.as_str() };
            if ui.add_enabled(!busy, egui::Button::new(label).small()).clicked() {
                let _ = tx.send(AccountAction::SignIn);
            }
            return;
        };

        let badge = egui::Button::new(RichText::new(account.initial()).strong()).min_size(egui::vec2(26.0, 26.0)).corner_radius(13.0);
        let hit = ui.add(badge).on_hover_text(account.display.as_str());

        egui::Popup::menu(&hit).show(|ui| {
            ui.set_min_width(240.0);
            // The deployment first, because somebody with an account on two of
            // them needs to know which one this is before reading anything
            // else.
            ui.horizontal(|ui| {
                ui.label(RichText::new(account.service.as_str()).small().color(ui.visuals().weak_text_color()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button(labels.sign_out.as_str()).clicked() {
                        let _ = tx.send(AccountAction::SignOut);
                        ui.close();
                    }
                });
            });
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(RichText::new(account.initial()).size(26.0).strong());
                ui.vertical(|ui| {
                    ui.label(RichText::new(account.display.as_str()).strong().size(15.0));
                    if let Some(email) = &account.email {
                        ui.label(RichText::new(email.as_str()).small().color(ui.visuals().weak_text_color()));
                    }
                });
            });

            ui.add_space(6.0);

            // Storage, which is the thing that actually stops working when it
            // is missing. Said here rather than discovered at the moment of
            // saving.
            if account.linked {
                ui.label(RichText::new(labels.linked.as_str()).small().color(ui.visuals().weak_text_color()));
            } else {
                ui.label(RichText::new(labels.not_linked.as_str()).small().color(ui.visuals().warn_fg_color));
                if ui.button(labels.link.as_str()).clicked() {
                    let _ = tx.send(AccountAction::Link);
                    ui.close();
                }
            }

            ui.add_space(4.0);
            if ui.link(labels.manage.as_str()).clicked() {
                let _ = tx.send(AccountAction::Manage);
                ui.close();
            }
        });
    }

    /// Draw the badge in a context that has no `Ui` to hand.
    pub fn show_in_ctx(&mut self, ctx: &Context) {
        egui::Area::new(egui::Id::new("craft-ui/account")).anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0)).show(ctx, |ui| {
            self.show(ui);
        });
    }
}
