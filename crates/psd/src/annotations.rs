// Reader primitives extracted from PhotoCraft annotations_map.rs at aa87012a.
struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }
    fn pascal(&mut self) -> Option<String> {
        let n = usize::from(self.u8()?);
        let s = self.take(n)?;
        if (n + 1) % 2 == 1 {
            self.take(1)?;
        }
        Some(s.iter().map(|&c| c as char).collect())
    }
}

fn pascal(out: &mut Vec<u8>, s: &str) {
    let bytes: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).take(255).collect();
    out.push(bytes.len() as u8);
    out.extend_from_slice(&bytes);
    if (bytes.len() + 1) % 2 == 1 {
        out.push(0);
    }
}

