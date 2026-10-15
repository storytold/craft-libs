use crate::{DeviceCaps, GpuFault};

/// Split `data` into contiguous slices no larger than [`DeviceCaps::max_storage_buffer_binding_size`].
pub fn chunk_buffer<'a>(caps: &DeviceCaps, data: &'a [u8]) -> Result<Vec<&'a [u8]>, GpuFault> {
    let max = usize::try_from(caps.max_storage_buffer_binding_size)
        .map_err(|_| GpuFault::Operation("max_storage_buffer_binding_size exceeds host address space".into()))?;
    if max == 0 {
        return Err(GpuFault::Operation("max_storage_buffer_binding_size must be non-zero".into()));
    }
    Ok(data.chunks(max).collect())
}
