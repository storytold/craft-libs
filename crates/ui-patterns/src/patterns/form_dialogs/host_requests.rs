//! `HostRequests`: what a hosted dialog's rows ask of the host, kept until the host takes it.

use super::rows::ParamRowsOutput;
use crate::runtime::requests::{ColorRequest, EyedropperRequest, HostAnswer, PathRequest};
use craft_theme::Srgba;
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

/// The requests of parameter rows that the host answers: the colour picker, the file dialog and the eyedropper. A
/// hosted dialog embeds one, feeds it each pass's rows with [`absorb`](HostRequests::absorb) and offers the
/// `take_*` methods to its host. A request stays until it is taken; a newer one of the same kind replaces it.
#[derive(Clone, Debug, Default)]
pub struct HostRequests {
    color: Option<ColorRequest>,
    path: Option<PathRequest>,
    eyedropper: Option<EyedropperRequest>,
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
        let HostRequests { color, path, eyedropper, answers } = other;
        self.color = color.or(self.color.take());
        self.path = path.or(self.path.take());
        self.eyedropper = eyedropper.or(self.eyedropper.take());
        self.answers.extend(answers);
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
