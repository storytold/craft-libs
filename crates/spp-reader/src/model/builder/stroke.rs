//! `DataStroke` → [`ProjectStroke`]: the packed point and pressure lists read into typed keys.

use super::ModelBuilder;
use crate::model::path::{PathKey, PathKeyKind, StrokePath3d};
use crate::model::raw::{PackedList, RawObject};
use crate::model::stroke::{ProjectStroke, StoredMat4, StrokeKey, StrokeView, TimelineKey};

impl ModelBuilder {
    pub(super) fn stroke(&mut self, s: &RawObject) -> ProjectStroke {
        self.seen(s);
        let mut complete = true;
        let points = self
            .keys(s.packed("points"), &mut complete, |row| Some(StrokeKey { position: row.floats::<2>("position")?, time: row.f32("time").unwrap_or(0.0) }));
        let pressure = self.keys(s.packed("pressure"), &mut complete, |row| Some(TimelineKey { time: row.f32("x")?, value: row.f32("value")? }));
        let tilt_keys = s.packed("tilt").map_or(0, PackedList::len);
        let mat4 = |name: &str| -> StoredMat4 {
            match s.floats(name) {
                Some(m) if m.len() == 16 => std::array::from_fn(|i| m.get(i).copied().unwrap_or(0.0)),
                _ => [0.0; 16],
            }
        };
        ProjectStroke {
            uid: s.uid(),
            points,
            pressure,
            tilt_keys,
            complete,
            view_matrix: mat4("viewMatrix"),
            projection_matrix: mat4("projectionMatrix"),
            view: StrokeView::from_value(s.u64("viewType").unwrap_or(1)),
            duplicate_count: s.u64("duplicateCount").map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
            duplicate_matrix: mat4("duplicateMatrix"),
            random_seed: s.u64("randomSeed").map_or(0, |n| n as u32),
            lazy_distance: s.f32("lazyDistance").unwrap_or(0.0),
        }
    }

    /// `DataStroke3D` → [`StrokePath3d`]. Keys before format 92 hold `pressure` where newer ones hold `size` and `opacity`.
    pub(super) fn path(&mut self, s: &RawObject) -> StrokePath3d {
        self.seen(s);
        let mut complete = true;
        let points = self.keys(s.packed("points"), &mut complete, |row| {
            let pressure = row.f32("pressure");
            Some(PathKey {
                kind: PathKeyKind::from_value(row.uint("type")?),
                position: row.floats::<3>("position")?,
                normal: row.floats::<3>("normal").unwrap_or([0.0; 3]),
                size: row.f32("size").or(pressure).unwrap_or(1.0),
                opacity: row.f32("opacity").or(pressure).unwrap_or(1.0),
            })
        });
        let duplicate_matrix: StoredMat4 = match s.floats("duplicateMatrix") {
            Some(m) if m.len() == 16 => std::array::from_fn(|i| m.get(i).copied().unwrap_or(0.0)),
            _ => [0.0; 16],
        };
        StrokePath3d {
            uid: s.uid(),
            label: s.string("label").unwrap_or_default().to_string(),
            closed: s.bool("isClosed").unwrap_or(false),
            points,
            complete,
            random_seed: s.u64("randomSeed").map_or(0, |n| n as u32),
            duplicate_count: s.u64("duplicateCount").map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
            duplicate_matrix,
        }
    }

    /// The rows of a packed key list read with `read`; rows it cannot read, or an incomplete list, clear
    /// `complete`. Read rows count as recognised.
    fn keys<T>(&mut self, list: Option<&PackedList>, complete: &mut bool, read: impl Fn(&crate::model::raw::PackedRow) -> Option<T>) -> Vec<T> {
        let Some(list) = list else { return Vec::new() };
        *complete &= list.complete;
        let mut out = Vec::with_capacity(list.len());
        let mut recognised: Vec<(&str, usize)> = Vec::new();
        for row in list.rows() {
            let Some(key) = read(&row) else {
                *complete = false;
                continue;
            };
            out.push(key);
            match recognised.iter_mut().find(|(c, _)| *c == row.class()) {
                Some((_, n)) => *n += 1,
                None => recognised.push((row.class(), 1)),
            }
        }
        for (class, n) in recognised {
            self.report.recognise_many(class, n);
        }
        out
    }
}
