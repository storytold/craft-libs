//! macOS presentation.
//!
//! Open and Folder show as a free-standing panel (`-[NSOpenPanel beginWithCompletionHandler:]`),
//! as native apps show them: the panel doesn't block the window, and it stays where the user moves
//! it when the window is resized. A sheet stays centred on its window, which feels wrong for Open.
//! rfd can't show this panel: its async dialogs always begin a sheet, on the main or first window
//! when no parent is given (rfd 0.17.2 `backend/macos/modal_future.rs:71-102`).
//!
//! Save and message boxes are sheets on the parent window, as native apps show them. A Save that
//! offers two or more file types is a sheet made here ([`show_save_sheet`]), with a Format menu:
//! rfd's Save panel lists the extensions of every type and has no menu to choose one
//! (rfd 0.17.2 `backend/macos/file_dialog/panel_ffi.rs:107-120`). Other Saves and message boxes
//! are rfd's sheets. rfd falls back to the blocking `runModal` when the app isn't running or has no
//! window (`modal_future.rs:79-118`), so [`sheet_parent`] lets a sheet start only with a parent in a
//! running app.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::rc::{Retained, Weak};
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSModalResponse, NSModalResponseOK, NSOpenPanel, NSPopUpButton, NSSavePanel, NSTextField, NSView, NSWindow,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{Answer, FORMAT_LABEL, FileDialog, Filter, Kind, Reply, format_menu};

const NOT_MAIN: &str = "dialogs show from the main thread only";
const NOT_RUNNING: &str = "the app isn't running yet, so the dialog can't be shown";

/// The free-standing panel a [`crate::Dialogs`] has on screen.
pub(crate) type Panel = Retained<NSOpenPanel>;

/// Brings `panel` to the front, so a command that would show it again shows where it is. A panel
/// that has closed, but whose answer the app hasn't collected yet, stays closed.
pub(crate) fn raise(panel: &Panel) {
    if panel.isVisible() {
        panel.makeKeyAndOrderFront(None);
    }
}

/// The parent a sheet can be shown on, or why there is none.
pub(crate) fn sheet_parent<W: HasWindowHandle>(parent: Option<&W>) -> Result<&W, String> {
    let mtm = MainThreadMarker::new().ok_or(NOT_MAIN)?;
    let app = NSApplication::sharedApplication(mtm);
    if !app.isRunning() {
        return Err(NOT_RUNNING.into());
    }
    let parent = parent.ok_or("no window to show the dialog on")?;
    // rfd `expect`s the parent's view to be in a window (rfd 0.17.2 `backend/macos/utils.rs:34-46`),
    // and a panic aborts under `panic = "abort"`. Look for the view among the app's windows.
    let view = match parent.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::AppKit(h)) => h.ns_view.as_ptr().cast_const(),
        _ => return Err("the window has no macOS view".into()),
    };
    if !app.windows().iter().any(|window| window.contentView().is_some_and(|content| contains(&content, view))) {
        return Err("the window to show the dialog on isn't on screen".into());
    }
    Ok(parent)
}

/// Whether `view` is `root` or one of its subviews.
fn contains(root: &NSView, view: *const c_void) -> bool {
    std::ptr::eq((root as *const NSView).cast(), view) || root.subviews().iter().any(|child| contains(&child, view))
}

/// The app's window that `parent`'s view is in.
fn window_of<W: HasWindowHandle>(mtm: MainThreadMarker, parent: &W) -> Option<Retained<NSWindow>> {
    let view = match parent.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::AppKit(h)) => h.ns_view.as_ptr().cast_const(),
        _ => return None,
    };
    NSApplication::sharedApplication(mtm).windows().iter().find(|window| window.contentView().is_some_and(|content| contains(&content, view)))
}

/// The Format menu's height and the room around and between its label and menu, in points.
const ACCESSORY_HEIGHT: f64 = 38.0;
const MARGIN: f64 = 12.0;
const GAP: f64 = 8.0;
/// The Format menu's size, in points.
const MENU: NSSize = NSSize::new(260.0, 26.0);

/// Shows a Save sheet with a Format menu on `parent` (a [`sheet_parent`]) when `d` offers two or more file types
/// ([`format_menu::entries`]), and answers through `reply` with the type chosen in the menu. Choosing a type puts the
/// type's extensions first in the allowed types, which AppKit adds to a name typed without one, and sets the name
/// field to [`format_menu::renamed`]. The SDK's `NSSavePanel.h` says the name field can only be set while the panel is
/// configured; macOS 27.2 shows the new name. Where a system keeps the suggested name, the answer's `filter` still names the chosen
/// type, so the save flow settles the extension (`ExtensionPolicy::picked_type`, or the app's own rule) and asks
/// before replacing a file the panel did not ask about; a kept name with another extension reports that extension's
/// type ([`format_menu::answered_type`]). Gives `reply` back when `d` offers fewer types, for rfd's panel.
///
/// The menu is the panel's accessory view, set before `beginSheetModalForWindow:completionHandler:` shows the sheet:
/// an accessory set after the sheet began made macOS dismiss the sheet at once (storytold/vectorcraft#1800). The
/// completion handler holds the panel, the menu and the menu's target until the sheet closes; the target holds the
/// panel weakly.
pub(crate) fn show_save_sheet<W: HasWindowHandle>(d: &FileDialog, parent: &W, reply: Reply) -> Result<(), Reply> {
    let entries = format_menu::entries(&d.filters);
    if entries.is_empty() {
        return Err(reply);
    }
    let Some(mtm) = MainThreadMarker::new() else { return Err(reply) };
    let Some(window) = window_of(mtm, parent) else { return Err(reply) };
    let Some(panel) = craft_appkit::save_panel::new_save_panel(mtm) else {
        reply.send(Answer::Unavailable("macOS returned no Save panel".into()));
        return Ok(());
    };
    let name = d.file_name.clone().unwrap_or_default();
    let selected = format_menu::initial(&name, &d.filters, &entries);
    set_allowed_types(&panel, &d.filters, &entries, selected);
    if let Some(title) = &d.title {
        panel.setMessage(Some(&NSString::from_str(title)));
    }
    if let Some(directory) = d.directory.as_deref().and_then(NSURL::from_directory_path) {
        panel.setDirectoryURL(Some(&directory));
    }
    panel.setExtensionHidden(false);
    if !name.is_empty() {
        panel.setNameFieldStringValue(&NSString::from_str(&name));
    }
    let label = d.format_label.as_deref().unwrap_or(FORMAT_LABEL);
    let (accessory, popup) = accessory(mtm, label, &d.filters, &entries, selected);
    panel.setAccessoryView(Some(&accessory));

    // The name the field kept when the last choice could not rename it.
    let kept: Rc<RefCell<Option<String>>> = Rc::default();
    let (filters, listed, weak, kept_by_choice) = (d.filters.clone(), entries.clone(), Weak::new(&*panel), kept.clone());
    let choice = craft_appkit::save_panel::on_choose(mtm, &popup, move |at| {
        let (Some(panel), Some(chosen)) = (weak.load(), listed.get(at).and_then(|&i| filters.get(i))) else { return };
        set_allowed_types(&panel, &filters, &listed, at);
        let name = panel.nameFieldStringValue().to_string();
        let new_name = format_menu::renamed(&name, chosen, &filters);
        panel.setNameFieldStringValue(&NSString::from_str(&new_name));
        let shown = panel.nameFieldStringValue().to_string();
        *kept_by_choice.borrow_mut() = (shown != new_name).then_some(shown);
    });
    let reply = Cell::new(Some(reply));
    let answered = panel.clone();
    let (filters, opened_with) = (d.filters.clone(), d.file_name.clone());
    let handler = RcBlock::new(move |response: NSModalResponse| {
        // The menu sends its action to `choice` while the sheet is open; the handler keeps it until then.
        let _target = &choice;
        let Some(reply) = reply.take() else { return };
        if response != NSModalResponseOK {
            return reply.send(Answer::Cancelled);
        }
        let Some(path) = answered.URL().and_then(|url| url.to_file_path()) else { return reply.send(Answer::Cancelled) };
        let returned = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let filter = usize::try_from(popup.indexOfSelectedItem())
            .ok()
            .and_then(|at| format_menu::answered_type(&returned, kept.borrow().as_deref(), opened_with.as_deref(), &filters, &entries, at));
        reply.send(Answer::Saved { path, filter });
    });
    panel.beginSheetModalForWindow_completionHandler(&window, &handler);
    Ok(())
}

/// Sets `panel`'s allowed file types to [`format_menu::allowed`].
fn set_allowed_types(panel: &NSSavePanel, filters: &[Filter], entries: &[usize], selected: usize) {
    let types: Vec<Retained<NSString>> = format_menu::allowed(filters, entries, selected).iter().map(|e| NSString::from_str(e)).collect();
    // As rfd does, and as `show_panel` does for Open.
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&types)));
}

/// The label and the Format menu of `filters`' `entries`, `selected` chosen, in a view for the panel's accessory.
fn accessory(
    mtm: MainThreadMarker,
    text: &str,
    filters: &[Filter],
    entries: &[usize],
    selected: usize,
) -> (Retained<NSView>, Retained<NSPopUpButton>) {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.sizeToFit();
    let size = label.frame().size;
    label.setFrameOrigin(NSPoint::new(MARGIN, (ACCESSORY_HEIGHT - size.height) / 2.0));
    let x = MARGIN + size.width + GAP;
    let frame = NSRect::new(NSPoint::new(x, (ACCESSORY_HEIGHT - MENU.height) / 2.0), MENU);
    let popup = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), frame, false);
    // The label beside the menu is not tied to it for VoiceOver, so the menu carries the label's words as its name.
    popup.setAccessibilityLabel(Some(&NSString::from_str(text.trim_end_matches([':', '：']))));
    for title in format_menu::titles(filters, entries) {
        popup.addItemWithTitle(&NSString::from_str(&title));
    }
    if let Ok(index) = isize::try_from(selected) {
        popup.selectItemAtIndex(index);
    }
    let view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, NSSize::new(x + MENU.width + MARGIN, ACCESSORY_HEIGHT)));
    view.addSubview(&label);
    view.addSubview(&popup);
    (view, popup)
}

/// Shows an Open or Folder panel that doesn't block the window and answers through `reply` when
/// it closes. AppKit calls the completion handler on the main thread.
pub(crate) fn show_panel(d: &FileDialog, reply: Reply) -> Option<Panel> {
    let Some(mtm) = MainThreadMarker::new() else {
        reply.send(Answer::Unavailable(NOT_MAIN.into()));
        return None;
    };
    // Begun before the app runs, the panel would never show or answer.
    if !NSApplication::sharedApplication(mtm).isRunning() {
        reply.send(Answer::Unavailable(NOT_RUNNING.into()));
        return None;
    }
    let panel = NSOpenPanel::openPanel(mtm);
    let folder = d.kind == Kind::Folder;
    panel.setCanChooseFiles(!folder);
    panel.setCanChooseDirectories(folder);
    panel.setCanCreateDirectories(folder);
    panel.setAllowsMultipleSelection(d.kind == (Kind::Open { multiple: true }));
    if let Some(title) = &d.title {
        // Panels have no title bar on current macOS; the message line shows above the files.
        panel.setMessage(Some(&NSString::from_str(title)));
    }
    let extensions: Vec<_> =
        d.filters.iter().flat_map(|f| &f.extensions).map(|e| e.trim_start_matches('.')).filter(|e| !e.is_empty()).map(NSString::from_str).collect();
    if !extensions.is_empty() {
        // As rfd does. `allowedContentTypes` would add a dependency for UTType bindings; plain
        // extensions match in any letter case.
        #[allow(deprecated)]
        panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&extensions)));
    }
    if let Some(directory) = d.directory.as_deref().and_then(NSURL::from_directory_path) {
        panel.setDirectoryURL(Some(&directory));
    }
    // The handler keeps the panel alive until it closes; AppKit releases the handler after
    // calling it, which ends the cycle. Never called, the reply drops and answers Unavailable.
    let reply = Cell::new(Some(reply));
    let answered = panel.clone();
    let handler = RcBlock::new(move |response: NSModalResponse| {
        let Some(reply) = reply.take() else { return };
        if response != NSModalResponseOK {
            return reply.send(Answer::Cancelled);
        }
        let paths: Vec<_> = answered.URLs().to_vec().iter().filter_map(|url| url.to_file_path()).collect();
        reply.send(if paths.is_empty() { Answer::Cancelled } else { Answer::Paths(paths) });
    });
    panel.beginWithCompletionHandler(&handler);
    Some(panel)
}
