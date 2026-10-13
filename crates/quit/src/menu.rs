//! The macOS app menu, with a Quit that asks the app first.
//!
//! winit's default app menu sends Quit (⌘Q) straight to `-[NSApplication terminate:]`, and winit
//! does not implement `applicationShouldTerminate:`, so the app is never asked: unsaved work is
//! lost without a prompt. This menu has the same items, built with muda, but its Quit asks the
//! window to close (`ViewportCommand::Close`). That goes through the app's own close handling,
//! the same as the close button, and [`crate::on_close_request`] then quits. Apps with their own
//! muda menu use [`crate::QUIT_MENU_ID`] and [`crate::on_menu_event`] directly.

use std::cell::RefCell;

use egui::Context;
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

thread_local! {
    /// The installed menu. muda's items must outlive the native menu that shows them.
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

pub(crate) fn install(ctx: &Context, app_name: &str) -> muda::Result<()> {
    if MENU.with_borrow(Option::is_some) {
        return Ok(());
    }
    let quit = MenuItem::with_id(crate::QUIT_MENU_ID, format!("Quit {app_name}"), true, Some(Accelerator::new(Modifiers::META, Code::KeyQ)));
    let app_menu = Submenu::new(app_name, true);
    app_menu.append_items(&[
        &PredefinedMenuItem::about(Some(&format!("About {app_name}")), None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(Some(&format!("Hide {app_name}"))),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let menu = Menu::new();
    menu.append(&app_menu)?;
    menu.init_for_nsapp();

    let ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        crate::on_menu_event(&ctx, event.id.as_ref());
    }));
    MENU.set(Some(menu));
    Ok(())
}
