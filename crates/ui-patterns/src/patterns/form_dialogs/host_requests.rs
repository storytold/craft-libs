//! `HostRequests`: what a hosted dialog's rows ask of the host, kept until the host takes it.

use super::rows::ParamRowsOutput;
use crate::runtime::requests::{ColorRequest, EyedropperRequest, HostAnswer, ListingRequest, PathRequest};
use craft_theme::Srgba;
use egui::Pos2;
use std::path::PathBuf;

/// A request to render a preview of `options`, numbered so the host's answer can be matched to the settings it was
/// rendered for: an answer with an older `revision` than the dialog's latest request is stale.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PreviewRequest<O> {
    /// The request's number; it grows with every change of `options`.
    pub revision: u64,
    /// The settings to render with.
    pub options: O,
}

impl<O> PreviewRequest<O> {
    /// A request numbered `revision` for `options`.
    pub fn new(revision: u64, options: O) -> Self {
        Self { revision, options }
    }
}

/// The requests of parameter rows that the host answers: the colour picker, the file dialog, the eyedropper and a
/// folder listing, the click on a dialog's preview picture (an eyedropper there samples it) and a footer button that
/// keeps the dialog open. A hosted dialog embeds one, feeds it each pass's rows with [`absorb`](HostRequests::absorb)
/// and offers the `take_*` methods to its host. A request stays until it is taken; a newer one of the same kind
/// replaces it. It also keeps the colours a child picker previews in the dialog's colour rows
/// ([`preview_color`](HostRequests::preview_color)).
#[derive(Clone, Debug, Default)]
pub struct HostRequests {
    color: Option<ColorRequest>,
    path: Option<PathRequest>,
    eyedropper: Option<EyedropperRequest>,
    listing: Option<ListingRequest>,
    preview_click: Option<Pos2>,
    footer_action: Option<String>,
    /// The previewed colour of each colour row a child picker edits, by row key; one entry per key.
    previews: Vec<(String, Srgba)>,
    answers: Vec<HostAnswer>,
}

impl HostRequests {
    /// No request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the requests of `rows` in, leaving the rows' fields empty.
    pub fn absorb(&mut self, rows: &mut ParamRowsOutput) {
        if let Some(request) = rows.pick_color.take() {
            self.request_color(request);
        }
        if let Some(request) = rows.path.take() {
            self.request_path(request);
        }
        if let Some(request) = rows.eyedropper.take() {
            self.request_eyedropper(request);
        }
        if let Some(request) = rows.listing.take() {
            self.request_listing(request);
        }
    }

    /// Files a colour request; it replaces one not taken yet.
    pub fn request_color(&mut self, request: ColorRequest) {
        self.color = Some(request);
    }

    /// Files a Browse request; it replaces one not taken yet.
    pub fn request_path(&mut self, request: PathRequest) {
        self.path = Some(request);
    }

    /// Files an eyedropper request; it replaces one not taken yet.
    pub fn request_eyedropper(&mut self, request: EyedropperRequest) {
        self.eyedropper = Some(request);
    }

    /// Files a click on the preview picture at `point`, in the picture's units; it replaces one not taken yet.
    pub fn request_preview_click(&mut self, point: Pos2) {
        self.preview_click = Some(point);
    }

    /// The colour parameter whose button was pressed, once.
    pub fn take_color(&mut self) -> Option<ColorRequest> {
        self.color.take()
    }

    /// The Browse button that was pressed, once.
    pub fn take_path(&mut self) -> Option<PathRequest> {
        self.path.take()
    }

    /// The eyedropper toggle that was pressed, once.
    pub fn take_eyedropper(&mut self) -> Option<EyedropperRequest> {
        self.eyedropper.take()
    }

    /// The preview picture's point the last click landed on, once.
    pub fn take_preview_click(&mut self) -> Option<Pos2> {
        self.preview_click.take()
    }

    /// Answers the colour request `key` with `color`: the dialog writes it into its values in its next pass.
    pub fn answer_color(&mut self, key: impl Into<String>, color: Srgba) {
        self.color = None;
        self.answers.push(HostAnswer::Color { key: key.into(), color });
    }

    /// Answers the Browse request `key` with `path`: the dialog writes it into its values in its next pass.
    pub fn answer_path(&mut self, key: impl Into<String>, path: PathBuf) {
        self.path = None;
        self.answers.push(HostAnswer::Path { key: key.into(), path });
    }

    /// Answers the eyedropper request `key` with the sampled `color`.
    pub fn answer_eyedropper(&mut self, key: impl Into<String>, color: Srgba) {
        self.answers.push(HostAnswer::Eyedropper { key: key.into(), color });
    }

    /// The answers the host gave since the last call, in order, for the dialog to apply.
    pub fn take_answers(&mut self) -> Vec<HostAnswer> {
        std::mem::take(&mut self.answers)
    }

    /// Moves the requests and answers of `other` in; its requests replace the ones not taken yet.
    pub fn extend(&mut self, other: HostRequests) {
        let HostRequests { color, path, eyedropper, listing, preview_click, footer_action, previews, answers } = other;
        self.color = color.or(self.color.take());
        self.path = path.or(self.path.take());
        self.eyedropper = eyedropper.or(self.eyedropper.take());
        self.listing = listing.or(self.listing.take());
        self.preview_click = preview_click.or(self.preview_click.take());
        self.footer_action = footer_action.or(self.footer_action.take());
        for (key, color) in previews {
            self.preview_color(key, Some(color));
        }
        self.answers.extend(answers);
    }

    /// Files a footer button's id the user pressed; it replaces one not taken yet.
    pub fn request_footer_action(&mut self, id: String) {
        self.footer_action = Some(id);
    }

    /// The footer button id that was pressed, once.
    pub fn take_footer_action(&mut self) -> Option<String> {
        self.footer_action.take()
    }

    /// Files a request for the files of a folder; it replaces one not taken yet.
    pub fn request_listing(&mut self, request: ListingRequest) {
        self.listing = Some(request);
    }

    /// The folder listing that was requested, once.
    pub fn take_listing(&mut self) -> Option<ListingRequest> {
        self.listing.take()
    }

    /// Answers the listing request `key` with the `files` of its folder: the dialog applies them in its next pass.
    pub fn answer_listing(&mut self, key: impl Into<String>, files: Vec<PathBuf>) {
        self.listing = None;
        self.answers.push(HostAnswer::Listing { key: key.into(), files });
    }

    /// Shows `color` as the live preview of the colour row `key` while it is picked; `None` ends the preview. A newer
    /// colour for the same row replaces the earlier one.
    pub fn preview_color(&mut self, key: impl Into<String>, color: Option<Srgba>) {
        let key = key.into();
        self.previews.retain(|(previewed, _)| *previewed != key);
        if let Some(color) = color {
            self.previews.push((key, color));
        }
    }

    /// The colours child pickers preview, by colour row key: the dialog draws each in its row in place of the row's
    /// value, and writes none of them into its values.
    pub fn previewed_colors(&self) -> &[(String, Srgba)] {
        &self.previews
    }

    /// Drops the colour request not taken yet (the picker was cancelled).
    pub fn cancel_color(&mut self) {
        self.color = None;
    }

    /// Drops the eyedropper request not taken yet.
    pub fn cancel_eyedropper(&mut self) {
        self.eyedropper = None;
    }

    /// Drops every request and answer not taken yet (the dialog closed).
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
}
