//! The `WordSetCx` trait: the label and built-in text every context reads.

use super::body::BodyCx;
use super::context::{Cx, CxCore};
use super::item::ItemCx;
use super::measure::MeasureCx;
use crate::contract::Component;
use crate::runtime::env::Label;
use craft_i18n::{Format, WordSet};
use std::sync::Arc;

/// The label a component shows and the built-in text it reads (chapter 19).
pub trait WordSetCx {
    /// The label as given, a cheap clone.
    fn label(&self) -> Label;

    /// The word set `L` in the language of the scope; built once per installed `Words`.
    fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L>;

    /// How the scope writes numbers: locale digits, separators and grouping.
    fn format(&self) -> &Format;
}

impl WordSetCx for CxCore<'_> {
    fn label(&self) -> Label {
        self.resolved.label_value.clone()
    }

    fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        self.env.words()
    }

    fn format(&self) -> &Format {
        self.env.format()
    }
}

impl<C: Component> WordSetCx for Cx<'_, C> {
    fn label(&self) -> Label {
        self.core.label()
    }

    fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        self.core.words()
    }

    fn format(&self) -> &Format {
        self.core.format()
    }
}

impl<C: Component> WordSetCx for MeasureCx<'_, C> {
    fn label(&self) -> Label {
        self.resolved.label_value.clone()
    }

    fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        self.env.words()
    }

    fn format(&self) -> &Format {
        self.env.format()
    }
}

impl WordSetCx for BodyCx<'_> {
    fn label(&self) -> Label {
        self.resolved.label_value.clone()
    }

    fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        self.env.words()
    }

    fn format(&self) -> &Format {
        self.env.format()
    }
}

impl<K> WordSetCx for ItemCx<'_, '_, K> {
    /// An item has no label of its own: the empty label.
    fn label(&self) -> Label {
        Label::default()
    }

    fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        self.env.words()
    }

    fn format(&self) -> &Format {
        self.env.format()
    }
}
