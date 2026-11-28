//! The parts of MaterialCraft's `crates/render/src/ibl.rs` at 4ff5af9af7 that differ from
//! TextureCraft's copy. The rest of that file (directions, procedural environments, lookup,
//! halving, the prefilter, SH9 and the basis) is the same text as `super::texturecraft` apart
//! from the glam import path and `hammersley` being `pub(crate)`, so the differential tests run
//! that one copy for both apps.

/// Built-in environments, in menu order.
pub const ENVIRONMENTS: [&str; 6] = ["Studio Soft", "Studio Contrast", "Overcast", "Sunset", "Warehouse", "Night"];

/// The environment for a 3D view id (`studio`, `sunset`, `overcast`, `night`).
pub fn by_id(id: &str) -> &'static str {
    match id {
        "sunset" => "Sunset",
        "overcast" => "Overcast",
        "night" => "Night",
        "contrast" => "Studio Contrast",
        "warehouse" => "Warehouse",
        _ => "Studio Soft",
    }
}
