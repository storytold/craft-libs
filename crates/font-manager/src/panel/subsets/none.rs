//! Where the preview service's client does not build: no service.

use std::ops::Range;
use std::sync::Arc;

use craft_host::Wake;

use super::{Row, SubsetFont};
use crate::index::{Family, FontFile, Index};
use crate::store::Installer;

pub(crate) struct Subsets;

impl Subsets {
    pub(crate) fn new(_: Arc<dyn Installer>) -> Subsets {
        Subsets
    }

    pub(crate) fn read_settings_soon(&mut self) {}

    pub(crate) fn available(&self) -> bool {
        false
    }

    pub(crate) fn serves(&self, _: &Family) -> bool {
        false
    }

    pub(crate) fn loading(&self) -> bool {
        false
    }

    pub(crate) fn draws(&self, _: &Index, _: &Row<'_>) -> bool {
        false
    }

    pub(crate) fn draws_detail(&self, _: &Index, _: &Family, _: &str) -> bool {
        false
    }

    pub(crate) fn unreachable(&self) -> Option<(&str, &str)> {
        None
    }

    pub(crate) fn begin(&mut self, _: f64, _: &Wake) {}

    pub(crate) fn next_wake(&self) -> Option<f64> {
        None
    }

    pub(crate) fn typed(&mut self, _: String) {}

    pub(crate) fn font(&self, _: &Index, _: &Row<'_>) -> Option<SubsetFont> {
        None
    }

    pub(crate) fn detail_font(&self, _: &Index, _: &Family, _: &FontFile, _: &str) -> Option<SubsetFont> {
        None
    }

    pub(crate) fn window(visible: &Range<usize>, _: usize) -> Range<usize> {
        visible.clone()
    }

    pub(crate) fn request(&mut self, _: &Index, _: usize, _: &[Option<Row<'_>>], _: Range<usize>) {}

    pub(crate) fn request_detail(&mut self, _: Option<(&Index, &Family, &str)>, _: &[usize]) {}

    pub(crate) fn cancel_all(&mut self) {}
}
