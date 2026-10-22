use egui::{Context, Id, Rect};

#[derive(Clone)]
struct Motion {
    from: Rect,
    target: Rect,
    current: Rect,
    started: f64,
}

impl Motion {
    fn sample(&self, now: f64, duration: f32) -> Rect {
        if duration <= 0.0 {
            return self.target;
        }
        let t = ((now - self.started) / f64::from(duration)).clamp(0.0, 1.0) as f32;
        if t >= 1.0 {
            return self.target;
        }
        let eased = t * t * (3.0 - 2.0 * t);
        Rect::from_min_max(self.from.min.lerp(self.target.min, eased), self.from.max.lerp(self.target.max, eased))
    }

    fn moving(&self) -> bool {
        self.current != self.target
    }
}

/// Display-only rectangles, keyed by panel identity rather than changing split paths.
#[derive(Clone)]
pub(super) struct Geometry<P> {
    entries: Vec<(P, Motion)>,
    generation: u64,
    pub active: bool,
}

pub(super) struct Options {
    pub generation: u64,
    pub duration: f32,
}

impl<P: Clone + Eq + Send + Sync + 'static> Geometry<P> {
    pub fn prepare(ctx: &Context, surface: Id, options: Options, preview: bool, desired: &[(P, Rect)], moving: &[P], origins: &[(P, Rect)]) -> Self {
        let Options { generation, duration } = options;
        let key = surface.with("preview-geometry");
        let previous = ctx.data(|data| data.get_temp::<Self>(key)).filter(|state| state.generation == generation);
        let now = ctx.input(|input| input.time);
        let duration = if duration.is_finite() { duration.clamp(0.0, 10.0) } else { 0.0 };
        let animate = duration > 0.0 && (preview || previous.as_ref().is_some_and(|state| state.active));
        let mut entries: Vec<(P, Motion)> = Vec::with_capacity(desired.len());
        for (panel, target) in desired {
            if entries.iter().any(|(known, _)| known == panel) {
                continue;
            }
            // Joined tabs share the receiver's motion, even if the incoming tab is first.
            let prior = previous.as_ref().and_then(|state| {
                desired
                    .iter()
                    .filter(|(member, rect)| rect == target && !moving.contains(member))
                    .find_map(|(member, _)| state.entries.iter().find(|(known, _)| known == member).map(|(_, motion)| motion))
            });
            let origin = desired
                .iter()
                .filter(|(member, rect)| rect == target && !moving.contains(member))
                .find_map(|(member, _)| origins.iter().find(|(known, _)| known == member).map(|(_, rect)| *rect))
                .unwrap_or(*target);
            let mut motion = prior.cloned().unwrap_or(Motion { from: origin, target: origin, current: origin, started: now });
            let current = motion.sample(now, duration);
            if !animate {
                motion = Motion { from: *target, target: *target, current: *target, started: now };
            } else {
                if motion.target != *target {
                    motion.from = current;
                    motion.target = *target;
                    motion.started = now;
                }
                motion.current = motion.sample(now, duration);
            }
            entries.extend(desired.iter().filter(|(_, rect)| rect == target).map(|(member, _)| (member.clone(), motion.clone())));
        }
        let transitioning = entries.iter().any(|(_, motion)| motion.moving());
        if transitioning {
            ctx.request_repaint();
        }
        let state = Self { entries, generation, active: duration > 0.0 && (preview || transitioning) };
        ctx.data_mut(|data| data.insert_temp(key, state.clone()));
        state
    }

    pub fn rect(&self, panels: &[P], fallback: Rect) -> Rect {
        panels.iter().find_map(|panel| self.entries.iter().find(|(known, _)| known == panel).map(|(_, motion)| motion.current)).unwrap_or(fallback)
    }
}

/// Whether a surface is showing a candidate or finishing its display-only transition.
pub fn geometry_transition_active<P: Clone + Send + Sync + 'static>(ctx: &Context, surface: Id) -> bool {
    ctx.data(|data| data.get_temp::<Geometry<P>>(surface.with("preview-geometry"))).is_some_and(|state| state.active)
        || super::strips::active::<P>(ctx, surface)
}
