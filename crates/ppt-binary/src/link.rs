//! Hyperlinks and click actions: the `MouseClickInteractiveInfoContainer` and
//! `MouseOverInteractiveInfoContainer` of a shape's client data or of a text range ([MS-PPT]
//! §2.6.8–2.6.11, §2.9.56–2.9.59), with link targets from the document's `ExObjListContainer`.
//!
//! What is read: jumps to the next, previous, first, last and last viewed slide and to the end of
//! the show; web and file links (the `TargetAtom`); named shows, programs to run and media to
//! play, as the `.pptx` reader keeps them; and "stop other sounds". What is left out: macros and
//! OLE verbs (the model holds neither), the sound to play (this reader has no sound collection),
//! and a link to a specific slide of the same presentation, whose `LocationAtom` format
//! [MS-PPT] does not specify.

use std::collections::HashMap;

use craft_pptx::text::{Action, Hyperlink};

use crate::doc::{cstring_units, rt};
use crate::record::Rec;

/// `MouseClickInteractiveInfoContainer` / `MouseClickTextInteractiveInfoAtom` record instance.
pub const CLICK: u16 = 0;
/// `MouseOverInteractiveInfoContainer` / `MouseOverTextInteractiveInfoAtom` record instance.
pub const OVER: u16 = 1;

// InteractiveInfoActionEnum (§2.13.13).
const RUN_PROGRAM: u8 = 0x02;
const JUMP: u8 = 0x03;
const HYPERLINK: u8 = 0x04;
const MEDIA: u8 = 0x06;
const CUSTOM_SHOW: u8 = 0x07;

// LinkToEnum (§2.13.15).
const LT_NEXT_SLIDE: u8 = 0x00;
const LT_PREVIOUS_SLIDE: u8 = 0x01;
const LT_FIRST_SLIDE: u8 = 0x02;
const LT_LAST_SLIDE: u8 = 0x03;
const LT_URL: u8 = 0x08;
const LT_OTHER_PRESENTATION: u8 = 0x09;
const LT_OTHER_FILE: u8 = 0x0A;

/// `MacroNameAtom`'s CString instance (§2.6.11).
const MACRO_NAME: u16 = 2;

/// An `InteractiveInfoAtom` (§2.6.10) and its container's `MacroNameAtom`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    action: u8,
    jump: u8,
    link: u32,
    link_type: u8,
    stop_sound: bool,
    show_returns: bool,
    /// The macro, program or named show.
    name: String,
}

/// Reads an `InteractiveInfo` container (`MouseClick…` or `MouseOver…`, told apart by its
/// instance). `None` when its atom is missing or short.
pub fn read(container: &Rec) -> Option<Info> {
    let atom = container.child(rt::INTERACTIVE_INFO_ATOM)?;
    // soundIdRef (4), exHyperlinkIdRef (4), action, oleVerb, jump, flags, hyperlinkType.
    let flags = atom.u8(11)?;
    let name = container
        .children()
        .find(|r| r.kind == rt::CSTRING && r.instance == MACRO_NAME)
        .map(|r| String::from_utf16_lossy(&cstring_units(r.data).collect::<Vec<u16>>()))
        .unwrap_or_default();
    Some(Info {
        link: atom.u32(4)?,
        action: atom.u8(8)?,
        jump: atom.u8(10)?,
        // fAnimated is bit 0, fStopSound bit 1, fCustomShowReturn bit 2.
        stop_sound: flags & 0x02 != 0,
        show_returns: flags & 0x04 != 0,
        link_type: atom.u8(12)?,
        name,
    })
}

/// The action `info` performs, with `targets` the document's hyperlink targets by id. `None` for
/// no action, one the model can't hold, or a link whose target is missing.
pub fn hyperlink(info: &Info, targets: &HashMap<u32, String>) -> Option<Hyperlink> {
    let named = || Some(info.name.clone()).filter(|n| !n.is_empty());
    let action = match info.action {
        // InteractiveInfoJumpEnum (§2.13.14).
        JUMP => match info.jump {
            0x01 => Action::NextSlide,
            0x02 => Action::PreviousSlide,
            0x03 => Action::FirstSlide,
            0x04 => Action::LastSlide,
            0x05 => Action::LastViewed,
            0x06 => Action::EndShow,
            _ => return None,
        },
        HYPERLINK => match info.link_type {
            LT_NEXT_SLIDE => Action::NextSlide,
            LT_PREVIOUS_SLIDE => Action::PreviousSlide,
            LT_FIRST_SLIDE => Action::FirstSlide,
            LT_LAST_SLIDE => Action::LastSlide,
            LT_URL | LT_OTHER_PRESENTATION | LT_OTHER_FILE => Action::Url { url: targets.get(&info.link).filter(|t| !t.is_empty())?.clone() },
            _ => return None,
        },
        CUSTOM_SHOW => Action::CustomShow { name: named()?, returns: info.show_returns },
        RUN_PROGRAM => Action::Program { path: named()? },
        MEDIA => Action::PlayMedia,
        _ => return None,
    };
    Some(Hyperlink { action, tooltip: String::new(), highlight_click: false, sound: None, stop_sound: info.stop_sound, zoom_return: None })
}
