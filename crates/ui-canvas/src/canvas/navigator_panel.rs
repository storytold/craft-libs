//! `NavigatorPanel`: the Navigator panel's body, the preview box over the zoom row, bound to one view.

use crate::canvas::{NavigationOptions, Navigator, NavigatorPreview, NavigatorZoomRow, ProxyColor};
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::layout_components::Stack;
use crate::runtime::env::Label;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvents};
use egui::{Response, Ui};

#[cfg(feature = "testing")]
mod stories;

/// What the Navigator panel reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NavigatorPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The view changes of this pass, from the preview box or the zoom row's field and slider, in order. A preview
    /// zoom under Animated Zoom is the change to its target; the canvas eases to it.
    pub view: EditEvents<CanvasView>,
}

impl Outcome for NavigatorPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Navigator panel (Photoshop's Window, Navigator): the [`Navigator`] preview box over the [`NavigatorZoomRow`],
/// both bound to one view.
///
/// - Both parts edit the one bound view. Under Animated Zoom a preview zoom and the row's Zoom In and Zoom Out ease on
///   the active canvas: the canvas the view commands reach shows the way to the new view.
/// - The row's Zoom In and Zoom Out are the view commands of the active canvas: the canvas steps its own view and
///   reports it in [`CanvasOutput`](crate::canvas::CanvasOutput).
/// - Tab goes from the preview box to the zoom row's percent field, then through the row in reading order.
/// - `enabled(false)` disables the preview box and every control of the zoom row.
#[must_use = "show the panel with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct NavigatorPanel<'a> {
    label: Label,
    view: Bind<'a, CanvasView>,
    document: Size,
    /// The picture in the preview box.
    #[prop]
    preview: NavigatorPreview<'a>,
    /// The app's navigation options, for the preview box.
    #[prop]
    navigation: NavigationOptions,
    /// The colour of the proxy rectangle.
    #[prop]
    proxy: ProxyColor,
    /// Whether the preview box and the zoom row take input.
    #[prop]
    enabled: bool,
}

impl<'a> NavigatorPanel<'a> {
    /// A Navigator panel named `label` that edits `view`, over a document of `document` units.
    pub fn new(label: impl Into<Label>, view: Bind<'a, CanvasView>, document: Size) -> Self {
        NavigatorPanel {
            label: label.into(),
            view,
            document,
            preview: NavigatorPreview::None,
            navigation: NavigationOptions::default(),
            proxy: ProxyColor::Default,
            enabled: true,
        }
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> NavigatorPanelOutput {
        let NavigatorPanel { label, mut view, document, mut preview, navigation, proxy, enabled } = self;
        let mut change = EditEvents::new();
        let shown = Stack::new().enabled(enabled).show(ui, |ui, _| {
            // The body runs for the measuring pass and again for the real one, so each run lends the picture.
            let preview =
                Navigator::new(label.clone(), view.reborrowed(), document).preview(preview.reborrowed()).navigation(navigation).proxy(proxy).show(ui);
            let row = NavigatorZoomRow::new(view.reborrowed(), document).show(ui);
            change = preview.view.events.clone().into_iter().chain(row.view).collect();
        });
        NavigatorPanelOutput { response: shown.output, view: change }
    }
}
