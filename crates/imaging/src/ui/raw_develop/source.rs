//! What the app gives the RAW Develop workspace: the develop and its pictures.

use std::sync::Arc;

use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use craft_ui::runtime::pending::Job;

use crate::raw::{Demosaic, RawDevelopSettings};
use crate::scopes::ClippingMask;
use crate::ui::scopes::{ScopeFrame, ScopeReadout};
use crate::ui::wide_angle::PreviewDone;

/// One develop of the preview: the proxy, and the visible part at full resolution when the view shows more pixels than
/// the proxy has.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RawPreviewRequest {
    /// Counts the source changes, so a late preview of old pixels is dropped.
    pub revision: u64,
    /// The settings to develop with.
    pub settings: RawDevelopSettings,
    /// The demosaic, while a raw is being opened.
    pub demosaic: Option<Demosaic>,
    /// The visible part in document pixels.
    pub visible: PixelRect,
    /// Screen pixels per document pixel.
    pub zoom: f64,
}

impl RawPreviewRequest {
    /// A request for `settings` at `revision`.
    pub fn new(revision: u64, settings: RawDevelopSettings, demosaic: Option<Demosaic>, visible: PixelRect, zoom: f64) -> Self {
        Self { revision, settings, demosaic, visible, zoom }
    }
}

/// The app's develop: GPU or CPU passes, proxies, detail refinement, histograms, clipping masks and samples stay the
/// app's.
pub trait RawDevelopSource: Send + Sync {
    /// The picture as developed with the settings.
    fn after(&self) -> &dyn RasterSource;

    /// The picture before the settings (Before, Y).
    fn before(&self) -> &dyn RasterSource;

    /// The job that develops `request` into [`after`](Self::after).
    fn develop(&self, request: &RawPreviewRequest) -> Job<PreviewDone, String>;

    /// The settings Auto picks for `settings`.
    fn auto(&self, settings: &RawDevelopSettings) -> RawDevelopSettings;

    /// Shows what the scope counted of the picture before or after, of the selected region or the whole picture,
    /// with the point `at` marked: `show` gets the frame, `None` while there is none.
    fn scope(&self, before: bool, region: bool, at: Option<[f64; 2]>, show: &mut dyn FnMut(Option<ScopeFrame<'_>>));

    /// The clipping mask and its revision.
    fn clipping(&self) -> Option<(Arc<ClippingMask>, Revision)>;

    /// The sample at `at` in the form `lab` asks for; the host does the colour management.
    fn sample(&self, at: [f64; 2], before: bool, lab: bool) -> Option<ScopeReadout>;

    /// Whether the document has a selection, for the scope's region commands.
    fn has_selection(&self) -> bool;
}
