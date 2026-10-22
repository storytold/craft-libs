use egui::{Context, Id, Rect};

use super::DropTarget;

#[derive(Clone)]
struct Entry<P> {
    source: DropTarget<P>,
    started: Option<f64>,
    kept: bool,
}

#[derive(Clone)]
pub(super) struct Strips<P> {
    entries: Vec<Entry<P>>,
    generation: u64,
}

pub(super) struct Header<P> {
    pub rect: Rect,
    pub frozen: Option<DropTarget<P>>,
}

pub(super) struct Options {
    pub duration: f32,
    pub dragging: bool,
    pub returning: bool,
}

impl<P: Clone + Eq + Send + Sync + 'static> Strips<P> {
    pub fn load(ctx: &Context, surface: Id, generation: u64) -> Self {
        let mut state = ctx
            .data(|data| data.get_temp::<Self>(surface.with("preview-strips")))
            .filter(|state| state.generation == generation)
            .unwrap_or(Self { entries: Vec::new(), generation });
        for entry in &mut state.entries {
            entry.kept = false;
        }
        state
    }

    pub fn active(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn header(&mut self, ctx: &Context, panels: &[P], destination: Rect, frozen: Option<DropTarget<P>>, options: Options) -> Option<Header<P>> {
        let Options { duration, dragging, returning } = options;
        if let Some(source) = frozen.filter(|target| {
            target.tabs.len() <= super::MAX_PANELS
                && super::targets::usable(target.rect)
                && target.tab_strip.is_some_and(super::targets::usable)
                && panels.contains(&target.anchor)
        }) {
            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.source.anchor == source.anchor) {
                entry.source = source.clone();
                entry.started = None;
                entry.kept = true;
            } else if self.entries.len() < super::MAX_PANELS {
                self.entries.push(Entry { source: source.clone(), started: None, kept: true });
            }
            return Some(Header { rect: source.rect, frozen: Some(source) });
        }
        if dragging {
            return None;
        }
        let entry = self.entries.iter_mut().find(|entry| panels.contains(&entry.source.anchor))?;
        let original_order = panels.iter().eq(entry.source.tabs.iter().map(|(panel, _)| panel));
        if entry.source.rect == destination && returning && original_order {
            entry.kept = true;
            return Some(Header { rect: destination, frozen: Some(entry.source.clone()) });
        }
        if entry.source.rect == destination || !duration.is_finite() || duration <= 0.0 {
            return None;
        }
        let now = ctx.input(|input| input.time);
        let started = *entry.started.get_or_insert(now);
        let t = ((now - started) / f64::from(duration.min(10.0))).clamp(0.0, 1.0) as f32;
        if t >= 1.0 {
            return None;
        }
        let t = t * t * (3.0 - 2.0 * t);
        let rect = Rect::from_min_max(entry.source.rect.min.lerp(destination.min, t), entry.source.rect.max.lerp(destination.max, t));
        entry.kept = true;
        ctx.request_repaint();
        Some(Header { rect, frozen: None })
    }

    pub fn finish(mut self, ctx: &Context, surface: Id) {
        self.entries.retain(|entry| entry.kept);
        ctx.data_mut(|data| data.insert_temp(surface.with("preview-strips"), self));
    }
}

pub(super) fn active<P: Clone + Send + Sync + 'static>(ctx: &Context, surface: Id) -> bool {
    ctx.data(|data| data.get_temp::<Strips<P>>(surface.with("preview-strips"))).is_some_and(|state| !state.entries.is_empty())
}
