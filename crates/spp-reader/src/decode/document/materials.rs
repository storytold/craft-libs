//! [`MaterialRecord`]: what `document.bin` says about each texture set (`DataMaterial` objects under
//! `DataDocument.materials`), read with one streaming walk.
//!
//! Per material (Verified on the corpus): `sceneMaterialName` (= mesh material name), `sceneMaterialUID`
//! (= the `<set id>` of `/texture-cache/<set id>/…`), `resolutionLog2`, `additionalMaps` (baked maps as
//! `DataAdditionalMap { urlToBitmapRes, usage }`), `normalMapBitmap` (the baked normal map) and
//! `stacks[].channels` (`DataChannel { type = channel bit, format, userName }`).

use super::context::Context;
use super::header::DocumentHeader;
use super::tag::ValueTag;
use super::value::Scalar;
use super::walker::{Visitor, walk};
use crate::decode::DecodeError;

const MATERIAL: &str = "DataMaterial";

/// A channel declared by a texture set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelRecord {
    /// Channel bit, the same as in texture-cache paths (1 = base colour, …).
    pub type_bit: u64,
    /// The reference app's storage format enum: 0 sRGB8, 1 L8, 5 L16F, 6 RGB16F for the channels seen so far.
    pub format: u64,
    /// Name of a user channel; empty for standard ones.
    pub user_name: String,
}

/// A map baked from the mesh, referenced by URL (`/<name>?version=<resource id>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BitmapRef {
    pub url: String,
    /// `usage` enum of `DataAdditionalMap`; `None` for the material's `normalMapBitmap`.
    pub usage: Option<u64>,
}

/// One texture set as recorded in the document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterialRecord {
    pub name: String,
    /// `sceneMaterialUID`, the texture-cache set id.
    pub scene_uid: Option<u64>,
    /// `resolutionLog2` (width, height).
    pub resolution_log2: Option<[i32; 2]>,
    pub channels: Vec<ChannelRecord>,
    pub baked: Vec<BitmapRef>,
    /// Number of `DataLayer*` objects (layers and groups) in the material's stacks.
    pub layer_count: usize,
    /// Number of `DataAction*` objects (fills, paint, filters, …).
    pub action_count: usize,
}

/// All texture sets in document order.
pub struct DocumentMaterials;

impl DocumentMaterials {
    pub fn scan(data: &[u8]) -> Result<Vec<MaterialRecord>, DecodeError> {
        DocumentHeader::parse(data)?;
        let mut scan = MaterialScan::default();
        walk(data, &mut scan)?;
        Ok(scan.done)
    }
}

#[derive(Default)]
struct MaterialScan {
    context: Context,
    /// Context depth of the open material, and the record being filled.
    open: Option<(usize, MaterialRecord)>,
    done: Vec<MaterialRecord>,
}

impl MaterialScan {
    fn material(&mut self) -> Option<&mut MaterialRecord> {
        self.open.as_mut().map(|(_, m)| m)
    }

    /// Distance from the innermost object to the open material (0 = the material itself).
    fn below_material(&self) -> Option<usize> {
        let (depth, _) = self.open.as_ref()?;
        self.context.depth().checked_sub(*depth)
    }

    /// The innermost object is a baked-map reference of the material (an additional map or the normal map).
    fn is_bitmap_ref(&self) -> bool {
        self.context.frame(0).is_some_and(|f| f.class == "DataAdditionalMap" || (f.class == "DataBitmap" && f.via == "normalMapBitmap"))
    }

    fn material_field(&mut self, field: &str, value: Scalar) {
        let Some(m) = self.material() else { return };
        match (field, value) {
            ("sceneMaterialName", Scalar::String(s)) => m.name = s,
            ("sceneMaterialUID", v) => m.scene_uid = v.as_u64(),
            ("resolutionLog2", Scalar::IVec2(v)) => m.resolution_log2 = Some(v),
            _ => {}
        }
    }

    fn channel_field(&mut self, field: &str, value: Scalar) {
        let Some(m) = self.material() else { return };
        let Some(c) = m.channels.last_mut() else { return };
        match (field, value) {
            ("type", v) => c.type_bit = v.channel_bit().unwrap_or(0),
            ("format", v) => c.format = v.as_u64().unwrap_or(0),
            ("userName", Scalar::String(s)) => c.user_name = s,
            _ => {}
        }
    }

    fn bitmap_field(&mut self, field: &str, value: Scalar) {
        let Some(m) = self.material() else { return };
        let Some(b) = m.baked.last_mut() else { return };
        match (field, value) {
            ("urlToBitmapRes", Scalar::String(s)) => b.url = s,
            ("usage", v) => b.usage = v.as_u64(),
            _ => {}
        }
    }
}

impl Visitor for MaterialScan {
    fn object(&mut self, class: &str) {
        self.context.enter(class);
        if class == MATERIAL && self.open.is_none() {
            self.open = Some((self.context.depth(), MaterialRecord::default()));
            return;
        }
        let below = self.below_material();
        let is_bitmap_ref = self.is_bitmap_ref();
        let Some(m) = self.material() else { return };
        if class.starts_with("DataLayer") {
            m.layer_count += 1;
        } else if class.starts_with("DataAction") {
            m.action_count += 1;
        }
        match (below, class) {
            (Some(2), "DataChannel") => m.channels.push(ChannelRecord::default()),
            (Some(1), _) if is_bitmap_ref => m.baked.push(BitmapRef::default()),
            _ => {}
        }
    }

    fn end_object(&mut self) {
        if self.below_material() == Some(0)
            && let Some((_, m)) = self.open.take()
        {
            self.done.push(m);
        }
        self.context.leave();
    }

    fn field(&mut self, name: &[u8]) {
        self.context.field(name);
    }

    fn scalar(&mut self, tag: ValueTag, payload: &[u8]) {
        let Some(value) = Scalar::decode(tag, payload) else {
            return;
        };
        let field = self.context.current_field().to_string();
        match self.below_material() {
            Some(0) => self.material_field(&field, value),
            Some(2) if self.context.is(&["DataChannel", "DataMaterialStack"]) => self.channel_field(&field, value),
            Some(1) if self.is_bitmap_ref() => self.bitmap_field(&field, value),
            _ => {}
        }
    }

    fn unknown_tag(&mut self, _tag: u32, _offset: usize) {}
}
