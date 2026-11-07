use craft_gpu::GpuJobParams;

pub(crate) fn parse_u32(params: &GpuJobParams, prefix: &[u8]) -> Option<u32> {
    let bytes = params.0.as_slice();
    let start = bytes.windows(prefix.len()).position(|w| w == prefix)? + prefix.len();
    let rest = &bytes[start..];
    let end = rest.iter().position(|&b| b == b',' || b == b';').unwrap_or(rest.len());
    std::str::from_utf8(&rest[..end]).ok()?.parse().ok()
}

pub(crate) fn params_contain(params: &GpuJobParams, needle: &[u8]) -> bool {
    params.0.windows(needle.len()).any(|w| w == needle)
}
