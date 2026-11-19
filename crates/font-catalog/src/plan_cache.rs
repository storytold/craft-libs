//! Per-face bounded shaping plan reuse.

use std::sync::{Arc, Mutex};

/// A cache for one font face and variation instance. Do not share it across faces or instances.
/// Plans are created outside the lock, then checked again to avoid retaining duplicates.
#[derive(Default)]
pub struct ShapePlanCache {
    plans: Mutex<Vec<Arc<harfrust::ShapePlan>>>,
}

impl ShapePlanCache {
    /// Maximum number of plans retained; the oldest is evicted when this is reached.
    pub const CAPACITY: usize = 32;

    /// Returns a matching plan or builds and retains one for this face.
    pub fn get_or_create(
        &self,
        shaper: &harfrust::Shaper<'_>,
        instance: Option<&harfrust::ShaperInstance>,
        direction: harfrust::Direction,
        script: Option<harfrust::Script>,
        language: Option<&harfrust::Language>,
        features: &[harfrust::Feature],
    ) -> Arc<harfrust::ShapePlan> {
        let key = harfrust::ShapePlanKey::new(script, direction).language(language).instance(instance).features(features);
        let kept = |plans: &[Arc<harfrust::ShapePlan>]| plans.iter().rev().find(|p| key.matches(p)).cloned();
        if let Some(plan) = kept(&self.plans.lock().unwrap_or_else(|e| e.into_inner())) {
            return plan;
        }
        let plan = Arc::new(harfrust::ShapePlan::new(shaper, direction, script, language, features));
        let mut plans = self.plans.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(made) = kept(&plans) {
            return made;
        }
        if plans.len() >= Self::CAPACITY {
            plans.remove(0);
        }
        plans.push(plan.clone());
        plan
    }

    /// Number of retained plans.
    pub fn len(&self) -> usize {
        self.plans.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether this cache contains no plans.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
