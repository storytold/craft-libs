//! The Layer Style dialog's draft: the style being edited plus the dialog's own view state, and
//! the rules every edit follows.

use super::model::{EffectId, EffectInstance, EffectKind, LayerStyle, LayerStylePage, StyleParams, StyleValue};
use super::shadow::ShadowDrag;
use super::spec::EffectSpec;

/// What the Layer Style dialog edits: the [`LayerStyle`] and the dialog's view state (the page,
/// the Blend If channel shown, the Preview check, a shadow drag in progress).
///
/// The app owns the draft for as long as the dialog is open. The dialog reports the style as the
/// value of its edits (`EditEvent<LayerStyle>`) and the view state beside them (the page, the
/// Blend If channel and Preview as `Option`s of what changed); the app keeps both, previews the
/// style while [`preview`](Self::preview) is on, applies [`style`](Self::style) as one history
/// step on OK and drops it on Cancel. Only the style is recorded by `Dialog::history`, so Undo
/// never moves the view, the view state never reaches the document, and
/// [`render_hash`](Self::render_hash) ignores it: a page change re-renders nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerStyleDraft {
    /// The effect and blending values being edited.
    pub style: LayerStyle,
    /// The page shown.
    pub page: LayerStylePage,
    /// The Blend If entry shown on the Blending Options page, an index into
    /// `style.blending.blend_if`.
    pub blend_if_channel: usize,
    /// The Preview check box: whether the host shows the draft on the canvas. On by default.
    pub preview: bool,
    /// The shadow drag in progress, from its press to its release.
    pub shadow_drag: Option<ShadowDrag>,
}

impl Default for LayerStyleDraft {
    fn default() -> Self {
        LayerStyleDraft::new(LayerStyle::default())
    }
}

impl LayerStyleDraft {
    /// A draft of `style` showing its first instance's page, or Blending Options when it has
    /// none, with Preview on.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:529-571 `initial_fields` (no page asked: the
    // first instance).
    pub fn new(style: LayerStyle) -> LayerStyleDraft {
        let page = style.effects.first().map_or(LayerStylePage::Blending, |e| LayerStylePage::Effect(e.id));
        LayerStyleDraft { style, page, blend_if_channel: 0, preview: true, shadow_drag: None }
    }

    /// Opens on `kind`'s page (Layer › Layer Style › Drop Shadow…): its first instance, turned on,
    /// or a new instance when the layer has none.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:549-567 (`select` names a kind).
    pub fn open_on(&mut self, kind: EffectKind) {
        let first = self.style.instances(kind).next();
        if let Some(id) = first {
            if let Some(e) = self.style.effect_mut(id) {
                e.on = true;
            }
            self.page = LayerStylePage::Effect(id);
        } else {
            self.add_instance(kind, &EffectSpec::photoshop(kind).defaults);
        }
    }

    /// The selected instance's kind; `None` on Styles and Blending Options.
    pub fn selected_kind(&self) -> Option<EffectKind> {
        match self.page {
            LayerStylePage::Effect(id) => self.style.effect(id).map(|e| e.kind),
            _ => None,
        }
    }

    /// Adds an instance of `kind`, turned on, with `defaults` (the kind's spec defaults) and,
    /// when its `useGlobalLight` is on, `angle` set to the shared light angle. The id is one past
    /// the highest `fxN` in the style (the lowest free number if that overflows). It goes right
    /// after the last instance of its kind (at the end when there is none), so the render order
    /// keeps the layer's effect order, and its page is selected.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:449-460 `next_id`, :463-473 `add_instance`,
    // :82-88 `fresh_params`.
    pub fn add_instance(&mut self, kind: EffectKind, defaults: &StyleParams) -> EffectId {
        let id = self.next_id();
        let mut params = defaults.clone();
        if params.bool("useGlobalLight") == Some(true) {
            params.set("angle", StyleValue::Number(f64::from(self.style.global_light)));
        }
        let at = self.style.effects.iter().rposition(|e| e.kind == kind).map_or(self.style.effects.len(), |i| i + 1);
        self.style.effects.insert(at, EffectInstance { id, kind, on: true, params });
        self.page = LayerStylePage::Effect(id);
        id
    }

    /// Removes instance `id`. When its page was selected, the selection moves to the first
    /// remaining instance of the same kind, else to the Styles page.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:476-491 `remove_instance` (falls back to
    // `STYLES_PAGE`).
    pub fn remove_instance(&mut self, id: EffectId) {
        let kind = self.style.effect(id).map(|e| e.kind);
        self.style.effects.retain(|e| e.id != id);
        if self.page == LayerStylePage::Effect(id) {
            let sibling = kind.and_then(|kind| self.style.instances(kind).next());
            self.page = sibling.map_or(LayerStylePage::Styles, LayerStylePage::Effect);
        }
    }

    /// The check box of instance `id`: turns it on or off and selects its page (selecting a page
    /// alone never turns an effect on, #2288).
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:927-933.
    pub fn toggle(&mut self, id: EffectId) {
        if let Some(e) = self.style.effect_mut(id) {
            e.on = !e.on;
        }
        self.page = LayerStylePage::Effect(id);
    }

    /// Sets parameter `key` of instance `id`. The shared light angle: when `key` is `angle` and
    /// the instance's `useGlobalLight` is on, or `key` is `useGlobalLight` turned on, the angle
    /// (the new one, or the shared one) becomes `style.global_light` and every instance with
    /// `useGlobalLight` on, disabled ones included, takes it as its `angle`.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:415-446 `set_param`.
    pub fn set_param(&mut self, id: EffectId, key: &str, value: StyleValue) {
        let Some(instance) = self.style.effect(id) else {
            return;
        };
        let lit = instance.params.bool("useGlobalLight") == Some(true);
        let shared = match (key, &value) {
            ("angle", StyleValue::Number(n)) if lit => Some(*n),
            ("useGlobalLight", StyleValue::Bool(true)) => Some(f64::from(self.style.global_light)),
            _ => None,
        };
        if let Some(e) = self.style.effect_mut(id) {
            e.params.set(key, value);
        }
        if let Some(angle) = shared {
            self.style.global_light = angle as f32;
            for e in &mut self.style.effects {
                if e.params.bool("useGlobalLight") == Some(true) {
                    e.params.set("angle", StyleValue::Number(angle));
                }
            }
        }
    }

    /// Replaces the instances with a style preset's (`(kind, on, params)` in order), numbered
    /// `fx1`, `fx2`, … The app sets the preset's blend mode and opacities on
    /// `style.blending`; the light angle stays and the page stays on Styles, where presets
    /// are picked.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:752-775 `apply_style_preset`.
    pub fn apply_preset(&mut self, effects: Vec<(EffectKind, bool, StyleParams)>) {
        self.style.effects = (1..).map(EffectId).zip(effects).map(|(id, (kind, on, params))| EffectInstance { id, kind, on, params }).collect();
        if matches!(self.page, LayerStylePage::Effect(id) if self.style.effect(id).is_none()) {
            self.page = LayerStylePage::Styles;
        }
    }

    /// Whether a drag on the image moves the selected effect now: a Drop Shadow or Inner Shadow
    /// page is selected.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:498-501 `drags_shadow`.
    pub fn drags_shadow(&self) -> bool {
        self.selected_kind().is_some_and(EffectKind::drags_shadow)
    }

    /// A drag on the image with a shadow page selected. `defaults` are the selected kind's spec
    /// defaults, which a parameter the instance lacks takes ([`StyleParams::over`]). `start` is the
    /// press (it records the shadow's angle, 120 when the defaults lack it too, and distance, 0 in
    /// that case); `total` is the pointer's
    /// movement since, in document pixels. The new angle and distance ([`ShadowDrag::moved`]) go
    /// through [`set_param`](Self::set_param), so with Use Global Light the shared angle moves
    /// too. Returns whether the style changed; does nothing on other pages. The host calls it on
    /// every move (an `Update` of its outer gesture) and clears
    /// [`shadow_drag`](Self::shadow_drag) on release.
    // pc: crates/ui-egui/src/dialogs/layer_style.rs:508-527 `drag_shadow`.
    pub fn drag_shadow(&mut self, defaults: &StyleParams, start: bool, total: [f64; 2]) -> bool {
        let LayerStylePage::Effect(id) = self.page else {
            return false;
        };
        if !self.drags_shadow() {
            return false;
        }
        let Some(instance) = self.style.effect(id) else {
            return false;
        };
        if start || self.shadow_drag.is_none() {
            let shown = instance.params.over(defaults);
            let angle = shown.number("angle").unwrap_or(120.0);
            let distance = shown.number("distance").unwrap_or(0.0);
            self.shadow_drag = Some(ShadowDrag::begin(angle, distance));
        }
        let Some((angle, distance)) = self.shadow_drag.and_then(|drag| drag.moved(total)) else {
            return false;
        };
        let before = self.style.clone();
        self.set_param(id, "angle", StyleValue::Number(angle));
        self.set_param(id, "distance", StyleValue::Number(distance));
        self.style != before
    }

    /// An id one past the highest `fxN`, or the lowest free number when that overflows.
    fn next_id(&self) -> EffectId {
        let max = self.style.effects.iter().map(|e| e.id.0).max().unwrap_or(0);
        match max.checked_add(1) {
            Some(next) => EffectId(next),
            None => EffectId((1..).find(|n| self.style.effect(EffectId(*n)).is_none()).unwrap_or(1)),
        }
    }

    /// The style's [`render_hash`](LayerStyle::render_hash); the page, the Blend If channel,
    /// the Preview check and the drag don't count.
    pub fn render_hash(&self) -> u64 {
        self.style.render_hash()
    }
}
