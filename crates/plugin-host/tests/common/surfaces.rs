//! A recording `SurfaceSink`, and the fixture packages the data plane tests install.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Mutex;

use craft_command::Owner;
use craft_plugin_host::contrib::surface::{Surface, SurfaceSink};
use serde_json::{Value, json};

use super::package_from;

/// What the app would hold: the live surfaces, and the order of calls it got.
#[derive(Default)]
pub struct RecordingSink {
    live: Mutex<Vec<Surface>>,
    log: Mutex<Vec<String>>,
}

impl SurfaceSink for RecordingSink {
    fn add(&self, surface: Surface) {
        self.log.lock().unwrap().push(format!("add {}@{}:{}", surface.owner.plugin_id, surface.owner.generation, surface.contribution.id));
        self.live.lock().unwrap().push(surface);
    }

    fn remove(&self, owner: &Owner) {
        self.log.lock().unwrap().push(format!("remove {}@{}", owner.plugin_id, owner.generation));
        self.live.lock().unwrap().retain(|s| &s.owner != owner);
    }
}

impl RecordingSink {
    /// The live surface for contribution `id`. Panics when there is none.
    pub fn surface(&self, id: &str) -> Surface {
        self.live.lock().unwrap().iter().find(|s| s.contribution.id == id).unwrap_or_else(|| panic!("no live surface {id}")).clone()
    }

    /// Ids of the live surfaces, sorted.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.live.lock().unwrap().iter().map(|s| s.contribution.id.clone()).collect();
        ids.sort();
        ids
    }

    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

/// The `pixels` fixture. Entry numbers match `fixtures/pixels.wat`.
pub fn pixels_manifest(id: &str, version: &str) -> Value {
    json!({
        "schema": 1, "id": id, "name": id, "version": version,
        "abi": { "craft": 1 },
        "limits": { "memory_mb": 4, "stack_kb": 128 },
        "contributions": [
            { "kind": "effect", "id": "copy", "entry": 0, "process": { "sample": ["u8", "u16", "f32"] } },
            { "kind": "effect", "id": "vsum", "entry": 1, "process": { "overlap": 1, "sample": ["u8"] } },
            { "kind": "effect", "id": "vsum3", "entry": 1, "process": { "overlap": 3, "sample": ["u8"] } },
            { "kind": "effect", "id": "nan", "entry": 2, "process": { "sample": ["f32"] } },
            { "kind": "effect", "id": "x4", "entry": 3, "process": { "sample": ["f32"], "alpha": "straight" } },
            { "kind": "effect", "id": "trap", "entry": 4 },
            { "kind": "effect", "id": "err", "entry": 5 },
            { "kind": "effect", "id": "spin", "entry": 6 },
            { "kind": "effect", "id": "params", "entry": 7, "process": { "sample": ["u8"] }, "params": {
                "amount": { "type": "number", "min": 0, "max": 1, "default": 0.5 },
                "mode": { "type": "choice", "options": ["a", "b"], "default": "a" },
                "steps": { "type": "int", "min": 1, "max": 9 }
            } },
            { "kind": "effect", "id": "desc", "entry": 8, "process": { "overlap": 2 } },
            { "kind": "effect", "id": "big", "entry": 9, "process": { "sample": ["u16"] } },
            { "kind": "effect", "id": "trap-low", "entry": 10, "process": { "sample": ["u8"] } },
            { "kind": "filter", "id": "kind-filter", "entry": 0 },
            { "kind": "audio-effect", "id": "audio", "entry": 0 },
            { "kind": "object-filter", "id": "objects", "entry": 0 }
        ]
    })
}

pub fn pixels_package(id: &str, version: &str) -> Vec<u8> {
    package_from(&super::fixture_text("pixels"), &pixels_manifest(id, version))
}
