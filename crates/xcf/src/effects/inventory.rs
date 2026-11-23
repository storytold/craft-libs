//! Source identities and ordered pointers, without effect/mask/pixel decoding.
use super::{Budget, Owner, OwnerKind, Reader, Result, invalid};

/// Original owner identity and on-disk effect order. Offsets are wire identities,
/// not a promise that an effect record or its auxiliary mask can be decoded.
#[derive(Debug, PartialEq, Eq)]
pub struct LayerInventory {
    pub owner: Owner,
    pub offsets: Vec<u64>,
}

impl crate::Xcf<'_> {
    /// Inventory one layer without re-parsing its properties or decoding pixels.
    /// Reuse `budget` across layers and repeated calls: traversal work is never
    /// refunded; failed optional pointer retention refunds only temporary storage.
    /// Files before XCF20 have no effect list and return an empty inventory.
    pub fn layer_effects(&self, index: usize, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<LayerInventory> {
        if !keep_going() {
            return Err(crate::Error::Cancelled.into());
        }
        // Price source-index and retained header lookups even when the list is empty.
        budget.work.charge(2 * std::mem::size_of::<usize>())?;
        let source = self.layer_sources.get(index).copied().ok_or_else(|| invalid("layer index is out of bounds"))?;
        let layer = self.image.layers.get(index).ok_or_else(|| invalid("layer index is out of bounds"))?;
        let owner =
            Owner { kind: OwnerKind::Layer, offset: u64::try_from(source).map_err(|_| invalid("layer source offset exceeds wire identity"))? };
        let offsets = if self.image.version < 20 {
            Vec::new()
        } else {
            let offset = u64::try_from(layer.effect_source).map_err(|_| invalid("effect list offset exceeds wire identity"))?;
            Reader::new(self.c.b, self.image.version).pointers(offset, budget, keep_going)?
        };
        Ok(LayerInventory { owner, offsets })
    }
}

#[cfg(test)]
mod tests;
