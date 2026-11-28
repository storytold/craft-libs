//! Huffman trees and run-length bitstring decoding (Section 6.4.4, 7.2, 7.5.1).

use super::Error;
use craft_media::bitstream::BitReader;

/// A node in a binary Huffman tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HuffmanNode {
    /// Leaf node storing a token value (0..=31).
    Leaf(u8),
    /// Branch node with 0-child and 1-child.
    Branch(Box<HuffmanNode>, Box<HuffmanNode>),
}

/// A decoded Huffman tree for DCT tokens (up to 32 entries, max depth 32).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HuffmanTree {
    pub root: HuffmanNode,
    /// Code and length for each token 0..=31: `(bits, len)`.
    pub codes: [(u32, u8); 32],
}

impl HuffmanTree {
    /// Parse a single Huffman tree from a bitstream (Section 6.4.4).
    pub fn parse(r: &mut BitReader) -> Result<Self, Error> {
        let mut count = 0;
        let root = Self::parse_node(r, 0, &mut count)?;
        let mut codes = [(0u32, 0u8); 32];
        Self::build_codes(&root, 0, 0, &mut codes);
        Ok(Self { root, codes })
    }

    fn parse_node(r: &mut BitReader, depth: usize, count: &mut usize) -> Result<HuffmanNode, Error> {
        if depth > 32 {
            return Err(Error::Corrupt("Huffman tree depth exceeds 32 bits"));
        }
        let is_leaf = r.read_bits(1).map_err(|_| Error::Truncated("Huffman tree"))? != 0;
        if is_leaf {
            if *count >= 32 {
                return Err(Error::Corrupt("Huffman tree exceeds 32 entries"));
            }
            *count += 1;
            let token = r.read_bits(5).map_err(|_| Error::Truncated("Huffman leaf token"))? as u8;
            Ok(HuffmanNode::Leaf(token))
        } else {
            let left = Self::parse_node(r, depth + 1, count)?;
            let right = Self::parse_node(r, depth + 1, count)?;
            Ok(HuffmanNode::Branch(Box::new(left), Box::new(right)))
        }
    }

    fn build_codes(node: &HuffmanNode, current_bits: u32, len: u8, codes: &mut [(u32, u8); 32]) {
        match node {
            HuffmanNode::Leaf(token) => {
                let idx = *token as usize;
                if idx < 32 {
                    codes[idx] = (current_bits, len);
                }
            }
            HuffmanNode::Branch(left, right) => {
                Self::build_codes(left, current_bits << 1, len + 1, codes);
                Self::build_codes(right, (current_bits << 1) | 1, len + 1, codes);
            }
        }
    }

    /// Decode a single token from the bitstream.
    #[inline]
    pub fn decode_token(&self, r: &mut BitReader) -> Result<u8, Error> {
        let mut curr = &self.root;
        loop {
            match curr {
                HuffmanNode::Leaf(token) => return Ok(*token),
                HuffmanNode::Branch(left, right) => {
                    let bit = r.read_bits(1).map_err(|_| Error::Truncated("Huffman token"))?;
                    curr = if bit == 0 { left } else { right };
                }
            }
        }
    }
}

/// Decode a long-run encoded bit string of `nbits` bits (Section 7.2.1).
pub fn decode_long_run(r: &mut BitReader, nbits: usize) -> Result<Vec<bool>, Error> {
    let mut bits = Vec::with_capacity(nbits);
    if nbits == 0 {
        return Ok(bits);
    }

    let mut current_bit = r.read_bits(1).map_err(|_| Error::Truncated("long run initial bit"))? != 0;

    while bits.len() < nbits {
        // Decode Huffman run length per Table 7.7
        let (rstart, rbits_count) = decode_long_run_length_header(r)?;
        let roffs = if rbits_count > 0 { r.read_bits(rbits_count).map_err(|_| Error::Truncated("long run offset"))? as usize } else { 0 };
        let rlen = rstart + roffs;

        let needed = nbits - bits.len();
        let to_add = rlen.min(needed);
        bits.resize(bits.len() + to_add, current_bit);

        if bits.len() == nbits {
            break;
        }

        if rlen == 4129 {
            // Read new bit after max run of 4129
            current_bit = r.read_bits(1).map_err(|_| Error::Truncated("long run toggle bit"))? != 0;
        } else {
            current_bit = !current_bit;
        }
    }

    Ok(bits)
}

#[inline]
fn decode_long_run_length_header(r: &mut BitReader) -> Result<(usize, u32), Error> {
    // Table 7.7
    // b0 -> rstart 1, rbits 0
    // b10 -> rstart 2, rbits 1
    // b110 -> rstart 4, rbits 1
    // b1110 -> rstart 6, rbits 2
    // b11110 -> rstart 10, rbits 3
    // b111110 -> rstart 18, rbits 4
    // b111111 -> rstart 34, rbits 12
    let b0 = r.read_bits(1).map_err(|_| Error::Truncated("long run header"))?;
    if b0 == 0 {
        return Ok((1, 0));
    }
    let b1 = r.read_bits(1).map_err(|_| Error::Truncated("long run header"))?;
    if b1 == 0 {
        return Ok((2, 1));
    }
    let b2 = r.read_bits(1).map_err(|_| Error::Truncated("long run header"))?;
    if b2 == 0 {
        return Ok((4, 1));
    }
    let b3 = r.read_bits(1).map_err(|_| Error::Truncated("long run header"))?;
    if b3 == 0 {
        return Ok((6, 2));
    }
    let b4 = r.read_bits(1).map_err(|_| Error::Truncated("long run header"))?;
    if b4 == 0 {
        return Ok((10, 3));
    }
    let b5 = r.read_bits(1).map_err(|_| Error::Truncated("long run header"))?;
    if b5 == 0 { Ok((18, 4)) } else { Ok((34, 12)) }
}

/// Decode a short-run encoded bit string of `nbits` bits (Section 7.2.2).
pub fn decode_short_run(r: &mut BitReader, nbits: usize) -> Result<Vec<bool>, Error> {
    let mut bits = Vec::with_capacity(nbits);
    if nbits == 0 {
        return Ok(bits);
    }

    let mut current_bit = r.read_bits(1).map_err(|_| Error::Truncated("short run initial bit"))? != 0;

    while bits.len() < nbits {
        // Decode short run length per Table 7.11
        let (rstart, rbits_count) = decode_short_run_length_header(r)?;
        let roffs = r.read_bits(rbits_count).map_err(|_| Error::Truncated("short run offset"))? as usize;
        let rlen = rstart + roffs;

        let needed = nbits - bits.len();
        let to_add = rlen.min(needed);
        bits.resize(bits.len() + to_add, current_bit);

        if bits.len() == nbits {
            break;
        }

        current_bit = !current_bit;
    }

    Ok(bits)
}

#[inline]
fn decode_short_run_length_header(r: &mut BitReader) -> Result<(usize, u32), Error> {
    // Table 7.11
    // b0 -> rstart 1, rbits 1 (1..2)
    // b10 -> rstart 3, rbits 1 (3..4)
    // b110 -> rstart 5, rbits 1 (5..6)
    // b1110 -> rstart 7, rbits 2 (7..10)
    // b11110 -> rstart 11, rbits 2 (11..14)
    // b11111 -> rstart 15, rbits 4 (15..30)
    let b0 = r.read_bits(1).map_err(|_| Error::Truncated("short run header"))?;
    if b0 == 0 {
        return Ok((1, 1));
    }
    let b1 = r.read_bits(1).map_err(|_| Error::Truncated("short run header"))?;
    if b1 == 0 {
        return Ok((3, 1));
    }
    let b2 = r.read_bits(1).map_err(|_| Error::Truncated("short run header"))?;
    if b2 == 0 {
        return Ok((5, 1));
    }
    let b3 = r.read_bits(1).map_err(|_| Error::Truncated("short run header"))?;
    if b3 == 0 {
        return Ok((7, 2));
    }
    let b4 = r.read_bits(1).map_err(|_| Error::Truncated("short run header"))?;
    if b4 == 0 { Ok((11, 2)) } else { Ok((15, 4)) }
}

/// Decode a single motion vector component using Table 7.23.
pub fn decode_mv_component_huffman(r: &mut BitReader) -> Result<i32, Error> {
    let b0 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
    if b0 == 0 {
        let b1 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
        let b2 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
        match (b1, b2) {
            (0, 0) => Ok(0),
            (0, 1) => Ok(1),
            (1, 0) => Ok(-1),
            _ => {
                // b011x
                let b3 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
                if b3 == 0 { Ok(2) } else { Ok(-2) }
            }
        }
    } else {
        // starts with 1
        let b1 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
        if b1 == 0 {
            // 10...
            let b2 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
            if b2 == 0 {
                // 100x -> +/-3
                let b3 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
                if b3 == 0 { Ok(3) } else { Ok(-3) }
            } else {
                // 1010xx or 1011xx
                let b3 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
                let b4 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
                let b5 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
                match (b3, b4) {
                    (0, 0) => {
                        if b5 == 0 {
                            Ok(4)
                        } else {
                            Ok(-4)
                        }
                    }
                    (0, 1) => {
                        if b5 == 0 {
                            Ok(5)
                        } else {
                            Ok(-5)
                        }
                    }
                    (1, 0) => {
                        if b5 == 0 {
                            Ok(6)
                        } else {
                            Ok(-6)
                        }
                    }
                    _ => {
                        if b5 == 0 {
                            Ok(7)
                        } else {
                            Ok(-7)
                        }
                    }
                }
            }
        } else {
            // 11...
            let b2 = r.read_bits(1).map_err(|_| Error::Truncated("mv"))?;
            if b2 == 0 {
                // 110xxxx (7 bits) -> +/-8 .. +/-15
                let val = r.read_bits(4).map_err(|_| Error::Truncated("mv"))? as i32;
                let mag = 8 + (val >> 1);
                let sign = val & 1;
                if sign == 0 { Ok(mag) } else { Ok(-mag) }
            } else {
                // 111xxxxx (8 bits) -> +/-16 .. +/-31
                let val = r.read_bits(5).map_err(|_| Error::Truncated("mv"))? as i32;
                let mag = 16 + (val >> 1);
                let sign = val & 1;
                if sign == 0 { Ok(mag) } else { Ok(-mag) }
            }
        }
    }
}

/// Decode a single motion vector `(MVX, MVY)` (Section 7.5.1).
pub fn decode_mv(r: &mut BitReader, mvmode: u8) -> Result<(i32, i32), Error> {
    if mvmode == 0 {
        let mvx = decode_mv_component_huffman(r)?;
        let mvy = decode_mv_component_huffman(r)?;
        Ok((mvx, mvy))
    } else {
        let mag_x = r.read_bits(5).map_err(|_| Error::Truncated("mv mag"))? as i32;
        let sign_x = r.read_bits(1).map_err(|_| Error::Truncated("mv sign"))?;
        let mvx = if sign_x != 0 { -mag_x } else { mag_x };

        let mag_y = r.read_bits(5).map_err(|_| Error::Truncated("mv mag"))? as i32;
        let sign_y = r.read_bits(1).map_err(|_| Error::Truncated("mv sign"))?;
        let mvy = if sign_y != 0 { -mag_y } else { mag_y };

        Ok((mvx, mvy))
    }
}
