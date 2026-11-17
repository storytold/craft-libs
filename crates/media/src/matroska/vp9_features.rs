//! VP9 CodecPrivate feature records in Matroska/WebM.

/// The fields used to configure VP9 decoding and sample descriptions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vp9Features {
    pub profile: u8,
    pub level: u8,
    pub bit_depth: u8,
    pub chroma_subsampling: u8,
}

/// Read ID/length/value triples, retaining defaults when a trailing record is truncated.
pub fn vp9_features(private: &[u8]) -> Vp9Features {
    let mut result = Vp9Features { profile: 0, level: 0, bit_depth: 8, chroma_subsampling: 0 };
    let mut at = 0;
    while at + 2 <= private.len() {
        let (id, len) = (private[at], private[at + 1] as usize);
        let Some(value) = private.get(at + 2..at + 2 + len) else { break };
        let value = value.first().copied().unwrap_or(0);
        match id {
            1 => result.profile = value,
            2 => result.level = value,
            3 => result.bit_depth = value,
            4 => result.chroma_subsampling = value,
            _ => {}
        }
        at += 2 + len;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn features_keep_earlier_values_when_a_later_record_is_truncated() {
        let f = vp9_features(&[1, 1, 2, 3, 1, 10, 4, 2, 9]);
        assert_eq!(f, Vp9Features { profile: 2, level: 0, bit_depth: 10, chroma_subsampling: 0 });
        assert_eq!(vp9_features(&[3, 0]).bit_depth, 0);
        assert_eq!(vp9_features(&[]).bit_depth, 8);
    }
}
