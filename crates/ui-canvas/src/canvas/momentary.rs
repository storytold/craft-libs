//! Momentary tools, as in Photoshop: tools in effect only while a key is held.
//!
//! Space is the Hand while held, Cmd+Space (Ctrl+Space off the Mac) Zoom In, Alt+Space Zoom Out; Alt added to a held
//! Zoom In zooms out. Rotate View is a command of its own, and with [`MomentaryShift::RotateView`] Shift added to the
//! held Hand is Rotate View. The keys are the [`MomentaryCommands`] set (`Kind::Momentary`): the board fires a press when the
//! key goes down and a release when it comes up. The set is scoped to the canvas (`canvasView`), so the commands go
//! to the canvas that holds the key focus or surrounds it, else the one that held it last.
//! [`ToolHost::run`](super::ToolHost::run) takes them for its canvas and folds them into [`Momentary`], which picks
//! the tool for the pass; [`ToolHost::momentary`](super::ToolHost::momentary) runs that tool instead of the selected
//! one. An app plumbs nothing. A drag that began with a tool finishes with it. Space during a marquee or shape drag is the
//! tool's own key ([`ToolContext::held`](super::ToolContext::held)), and the host withholds it from the shortcuts then.

use super::CanvasTool;
use super::navigate::NavigationOptions;
use super::tools::{HandTool, RotateViewTool, ZoomTool};
use crate::runtime::commands::FirePhase;
use craft_keymap::Mods;

/// The commands that put a tool in effect while their key is held. They are scoped like [`ViewCommands`](super::ViewCommands): a canvas is
/// an instance of the one `canvasView` scope, and a full-window dialog's canvas leaves it to the dialog. They fire
/// inside a dialog, so Space pans a full-window dialog's canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "canvasMomentary", scope = "canvasView")]
#[non_exhaustive]
pub enum MomentaryCommands {
    /// The Hand tool while the key is held. While it is held, the keys of the `canvasHand` scope come before every
    /// other scope's: an app binds there what a key does with the Hand held (Photoshop's Space+F).
    #[command(label = "Hand (Hold)", kind = "momentary", modal = "in_dialogs", default = "Space", holds = "canvasHand")]
    Hand,
    /// The Zoom tool, zooming in, while the keys are held.
    #[command(label = "Zoom In (Hold)", kind = "momentary", modal = "in_dialogs", default = "Cmd+Space")]
    ZoomIn,
    /// The Zoom tool, zooming out, while the keys are held.
    #[command(label = "Zoom Out (Hold)", kind = "momentary", modal = "in_dialogs", default = "Alt+Space")]
    ZoomOut,
    /// The Rotate View tool while the key is held.
    #[command(label = "Rotate View (Hold)", kind = "momentary", modal = "in_dialogs")]
    RotateView,
}

/// What a held Shift does to the momentary Hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MomentaryShift {
    /// Shift changes nothing: the Hand stays the Hand.
    #[default]
    Hand,
    /// Hand held with Shift (and no Cmd, Ctrl or Alt) is the Rotate View tool.
    RotateView,
}

/// A tool in effect only while its keys are held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MomentaryTool {
    /// The Hand tool.
    Hand,
    /// The Zoom tool, zooming in.
    ZoomIn,
    /// The Zoom tool, zooming out.
    ZoomOut,
    /// The Rotate View tool.
    RotateView,
}

impl MomentaryTool {
    /// Whether the Zoom tool zooms out.
    pub const fn zooms_out(self) -> bool {
        matches!(self, MomentaryTool::ZoomOut)
    }
}

impl From<MomentaryCommands> for MomentaryTool {
    fn from(command: MomentaryCommands) -> Self {
        match command {
            MomentaryCommands::Hand => MomentaryTool::Hand,
            MomentaryCommands::ZoomIn => MomentaryTool::ZoomIn,
            MomentaryCommands::ZoomOut => MomentaryTool::ZoomOut,
            MomentaryCommands::RotateView => MomentaryTool::RotateView,
        }
    }
}

/// The momentary tools and the one in effect, kept by the app across passes. The app hands it to
/// [`ToolHost::momentary`](super::ToolHost::momentary), which feeds it the fired [`MomentaryCommands`]s and runs its
/// Hand or Zoom tool in place of the selected tool while a key is held.
#[derive(Clone, Debug)]
pub struct Momentary {
    /// The command whose key is down.
    held: Option<MomentaryCommands>,
    /// The tool the last pass without a gesture chose: the one a drag keeps.
    latched: Option<MomentaryTool>,
    hand: HandTool,
    zoom: ZoomTool,
    rotate: RotateViewTool,
    /// What a held Shift does to the Hand.
    shift: MomentaryShift,
    /// The selected tool is the Hand: Cmd and Alt at a press then give Zoom In and Zoom Out.
    hand_selected: bool,
}

impl Momentary {
    /// The momentary tools under `options`. The Zoom tool reads the document's size from the canvas's space.
    pub fn new(options: NavigationOptions) -> Self {
        Momentary {
            held: None,
            latched: None,
            hand: HandTool::new(options).lent(),
            zoom: ZoomTool::new(options).lent(),
            rotate: RotateViewTool::default(),
            shift: MomentaryShift::Hand,
            hand_selected: false,
        }
    }

    /// What a held Shift does to the momentary Hand: with [`MomentaryShift::RotateView`], Hand and Shift give the Rotate
    /// View tool.
    #[must_use]
    pub fn shift(mut self, shift: MomentaryShift) -> Self {
        self.shift = shift;
        self
    }

    /// Tells the set whether the selected tool is the Hand: with the Hand selected and no Space held, Cmd and Alt at a
    /// press give Zoom In and Zoom Out with the same latch.
    pub fn hand_selected(&mut self, selected: bool) {
        self.hand_selected = selected;
    }

    /// The command whose key is down, if any: the signal an app reads to know the Hand key is held.
    pub fn held(&self) -> Option<MomentaryCommands> {
        self.held
    }

    /// Follows a change of the navigation options.
    pub fn set_options(&mut self, options: NavigationOptions) {
        self.hand.options = options;
        self.zoom.options = options;
    }

    /// Ends a glide of the momentary Hand: the app's clamp of the view moved it, so the view hit a scroll limit.
    pub fn stop_glide(&mut self) {
        self.hand.stop_glide();
    }

    /// Takes a fired momentary command: a press puts its tool in effect, a release ends it.
    pub fn fired(&mut self, command: MomentaryCommands, phase: FirePhase) {
        match phase {
            FirePhase::Press | FirePhase::Repeat => self.held = Some(command),
            FirePhase::Release => self.held = None,
        }
    }

    /// The tool for this pass with `mods` held. Modifiers pressed after the key still count, in any order: Cmd turns
    /// the Hand into Zoom In, and Alt turns either into Zoom Out. While `in_gesture` (a drag runs), the tool of the
    /// pass before keeps the drag, whatever the keys do.
    pub fn tool(&mut self, mods: Mods, in_gesture: bool) -> Option<MomentaryTool> {
        self.tool_holding(None, false, mods, in_gesture)
    }

    /// [`tool`](Self::tool) with `injected`, the command that the keys injected input holds with no key event
    /// ([`ToolInput::hold`](super::ToolInput::hold)) run through the keymap: it counts before the command whose window
    /// key is down, as the real key would. With `navigates` ([`ToolHost::navigates`](super::ToolHost::navigates)) the
    /// Hand is in effect whatever the keys hold.
    pub(crate) fn tool_holding(
        &mut self,
        injected: Option<MomentaryCommands>,
        navigates: bool,
        mods: Mods,
        in_gesture: bool,
    ) -> Option<MomentaryTool> {
        if in_gesture {
            return self.latched;
        }
        let tool = if navigates { Some(MomentaryTool::Hand) } else { self.pick(injected.or(self.held), mods) };
        self.latched = tool;
        tool
    }

    fn pick(&self, held: Option<MomentaryCommands>, mods: Mods) -> Option<MomentaryTool> {
        match held {
            Some(held) => {
                let tool = MomentaryTool::from(held);
                Some(if tool == MomentaryTool::RotateView {
                    tool
                } else if mods.alt {
                    MomentaryTool::ZoomOut
                } else if tool == MomentaryTool::Hand && (mods.cmd || mods.ctrl) {
                    MomentaryTool::ZoomIn
                } else if tool == MomentaryTool::Hand && mods.shift && self.shift == MomentaryShift::RotateView {
                    MomentaryTool::RotateView
                } else {
                    tool
                })
            }
            // The selected Hand: Alt and Cmd at the press zoom, with the same latch as a held key.
            None if self.hand_selected && mods.alt => Some(MomentaryTool::ZoomOut),
            None if self.hand_selected && (mods.cmd || mods.ctrl) => Some(MomentaryTool::ZoomIn),
            None => None,
        }
    }

    /// The tool the host runs for `tool`, the momentary tool in effect: the set's Hand or Zoom, else `selected`.
    pub(crate) fn run_as<'t>(&'t mut self, tool: Option<MomentaryTool>, selected: &'t mut dyn CanvasTool) -> &'t mut dyn CanvasTool {
        match tool {
            Some(MomentaryTool::Hand) => &mut self.hand,
            Some(MomentaryTool::RotateView) => &mut self.rotate,
            Some(tool) => {
                self.zoom.out = tool.zooms_out();
                &mut self.zoom
            }
            None => selected,
        }
    }

    /// The momentary Hand while it glides after a flick.
    pub(crate) fn gliding_hand(&mut self) -> Option<&mut HandTool> {
        self.hand.is_gliding().then_some(&mut self.hand)
    }
}
