//! Bounded conversion of unsigned file stacking orders to Float's zero-is-unset model.

/// A file-order mapping. Ordinary files retain their exact one-more encoding.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileOrders {
    gap: Option<u32>,
}

impl FileOrders {
    /// A file order adjusted to leave room for its one-more model representation.
    pub fn file_order(self, raw: u32) -> u32 {
        raw - u32::from(self.gap.is_some_and(|gap| raw > gap))
    }

    /// An absent order remains unset; a present file order remains present.
    pub fn model_order(self, raw: Option<u32>) -> u32 {
        raw.map_or(0, |raw| self.file_order(raw).saturating_add(1))
    }
}

/// Collect every raw order before decoding any. No document or callback is replayed.
///
/// The upper window has more slots than the source can contain distinct ten-digit
/// orders. Only this window can be shifted. XML callers must count all source bytes,
/// decode attributes exactly as their reader does, and never expand custom entities.
pub struct FileOrderScan {
    seen: Vec<u8>,
    slots: usize,
}

impl FileOrderScan {
    /// Most source bytes scanned. This also caps the bitmap below 26 MiB.
    pub const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

    /// Allocate fallibly; reject an unbounded source rather than truncating the scan.
    pub fn new(source_bytes: u64) -> Result<Self, String> {
        if source_bytes > Self::MAX_SOURCE_BYTES {
            return Err("stacking-order source exceeds 2 GiB".into());
        }
        let slots = (source_bytes / 10 + 1) as usize;
        let size = slots.div_ceil(8);
        let mut seen = Vec::new();
        seen.try_reserve_exact(size).map_err(|_| "stacking-order bitmap allocation failed")?;
        seen.resize(size, 0);
        Ok(Self { seen, slots })
    }

    /// Observe a decoded unsigned order. Repeated values cost no extra memory.
    pub fn observe(&mut self, raw: u32) {
        let distance = (u32::MAX - raw) as usize;
        if distance < self.slots
            && let Some(byte) = self.seen.get_mut(distance / 8)
        {
            *byte |= 1 << (distance % 8);
        }
    }

    /// Finish after the complete source traversal, before constructing model floats.
    pub fn finish(self) -> Result<FileOrders, String> {
        if self.seen.first().is_none_or(|b| b & 1 == 0) {
            return Ok(FileOrders::default());
        }
        for (i, byte) in self.seen.into_iter().enumerate() {
            if byte == u8::MAX {
                continue;
            }
            let distance = i * 8 + (!byte).trailing_zeros() as usize;
            if distance < self.slots {
                return Ok(FileOrders { gap: Some(u32::MAX - distance as u32) });
            }
        }
        Err("stacking-order scan did not cover its source".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upper_tail_is_injective_and_leaves_ordinary_orders_alone() {
        let raw = [0, 1, 251658240, u32::MAX - 2, u32::MAX - 1, u32::MAX, u32::MAX];
        let mut scan = FileOrderScan::new(raw.len() as u64 * 10).unwrap();
        for raw in raw {
            scan.observe(raw);
        }
        let map = scan.finish().unwrap();
        assert_eq!(raw.map(|raw| map.model_order(Some(raw))), [1, 2, 251658241, u32::MAX - 2, u32::MAX - 1, u32::MAX, u32::MAX]);
        assert_eq!(map.model_order(None), 0);
    }

    #[test]
    fn no_max_means_no_renumbering_and_source_limits_fail_closed() {
        let mut scan = FileOrderScan::new(100).unwrap();
        scan.observe(u32::MAX - 1);
        assert_eq!(scan.finish().unwrap().model_order(Some(u32::MAX - 1)), u32::MAX);
        assert!(FileOrderScan::new(FileOrderScan::MAX_SOURCE_BYTES + 1).is_err());
        let mut incomplete = FileOrderScan::new(0).unwrap();
        incomplete.observe(u32::MAX);
        assert!(incomplete.finish().is_err());
    }
}
