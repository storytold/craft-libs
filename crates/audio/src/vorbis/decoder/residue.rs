//! Residues (Vorbis I §8): the fine detail of the spectrum, coded as vectors in passes.

use super::Error;
use super::bits::{BitReader, Eop};
use super::codebook::Codebook;

pub(super) struct Residue {
    kind: u32,
    begin: usize,
    end: usize,
    partition_size: usize,
    classifications: usize,
    classbook: usize,
    /// `books[class][pass]`.
    books: Vec<[Option<usize>; 8]>,
}

fn eop(_: Eop) -> Error {
    Error::Setup("setup header ends inside a residue")
}

impl Residue {
    pub(super) fn read(kind: u32, r: &mut BitReader<'_>, books: &[Codebook]) -> Result<Self, Error> {
        let begin = r.read(24).map_err(eop)? as usize;
        let end = r.read(24).map_err(eop)? as usize;
        let partition_size = r.read(24).map_err(eop)? as usize + 1;
        let classifications = r.read(6).map_err(eop)? as usize + 1;
        let classbook = r.read(8).map_err(eop)? as usize;
        let Some(cb) = books.get(classbook) else { return Err(Error::Setup("residue class book out of range")) };
        // classifications ^ dimensions must not exceed the book's entries.
        let mut reach = 1u128;
        for _ in 0..cb.dimensions {
            reach = reach.saturating_mul(classifications as u128);
            if reach > cb.entries as u128 {
                return Err(Error::Setup("residue class book has too few entries for its classifications"));
            }
        }
        let mut cascade = Vec::with_capacity(classifications);
        for _ in 0..classifications {
            let low = r.read(3).map_err(eop)?;
            let high = if r.bit().map_err(eop)? { r.read(5).map_err(eop)? } else { 0 };
            cascade.push(high * 8 + low);
        }
        let mut list = Vec::with_capacity(classifications);
        for &c in &cascade {
            let mut row = [None; 8];
            for (j, slot) in row.iter_mut().enumerate() {
                if c & (1 << j) != 0 {
                    let b = r.read(8).map_err(eop)? as usize;
                    match books.get(b) {
                        Some(book) if book.has_vectors() => *slot = Some(b),
                        _ => return Err(Error::Setup("residue names a missing codebook or one without vectors")),
                    }
                }
            }
            list.push(row);
        }
        Ok(Self { kind, begin, end, partition_size, classifications, classbook, books: list })
    }

    /// Decode a bundle of vectors, each `n` long (half the block size), into `out`; a vector marked
    /// in `skip` is left zero. End of packet ends decoding where it is (a nominal event).
    pub(super) fn decode(&self, r: &mut BitReader<'_>, books: &[Codebook], n: usize, skip: &[bool], out: &mut [Vec<f32>]) {
        let ch = out.len();
        if skip.iter().all(|&s| s) {
            return;
        }
        if self.kind == 2 {
            let mut flat = vec![vec![0f32; n * ch]];
            let _ = self.decode_vectors(r, books, n * ch, &[false], &mut flat);
            for i in 0..n {
                for (j, v) in out.iter_mut().enumerate() {
                    v[i] = flat[0][i * ch + j];
                }
            }
        } else {
            let _ = self.decode_vectors(r, books, n, skip, out);
        }
    }

    fn decode_vectors(&self, r: &mut BitReader<'_>, books: &[Codebook], actual: usize, skip: &[bool], out: &mut [Vec<f32>]) -> Result<(), Eop> {
        let ch = out.len();
        let begin = self.begin.min(actual);
        let end = self.end.min(actual);
        if end <= begin {
            return Ok(());
        }
        let classbook = &books[self.classbook];
        let cpw = classbook.dimensions;
        let partitions = (end - begin) / self.partition_size;
        // Classbook dimensions may greatly exceed the actual partition count.
        // Retain only digits that this block can use, not padding per channel.
        let mut classes = vec![vec![0usize; partitions]; ch];
        for pass in 0..8 {
            let mut count = 0usize;
            while count < partitions {
                if pass == 0 {
                    for j in 0..ch {
                        if skip[j] {
                            continue;
                        }
                        let mut temp = classbook.scalar(r)? as usize;
                        for i in (0..cpw).rev() {
                            if i + count < partitions {
                                classes[j][i + count] = temp % self.classifications;
                            }
                            temp /= self.classifications;
                        }
                    }
                }
                let mut i = 0;
                while i < cpw && count < partitions {
                    for j in 0..ch {
                        if skip[j] {
                            continue;
                        }
                        let Some(book) = self.books[classes[j][count]][pass] else { continue };
                        let book = &books[book];
                        let offset = begin + count * self.partition_size;
                        self.partition(r, book, &mut out[j][offset..offset + self.partition_size])?;
                    }
                    count += 1;
                    i += 1;
                }
            }
        }
        Ok(())
    }

    /// One partition into `v` (the slice of the vector it covers).
    fn partition(&self, r: &mut BitReader<'_>, book: &Codebook, v: &mut [f32]) -> Result<(), Eop> {
        let n = v.len();
        let dims = book.dimensions;
        let step = n / dims;
        if self.kind == 0 {
            for i in 0..step {
                let Some(entry) = book.vector(r)? else { return Ok(()) };
                for (j, &e) in entry.iter().enumerate() {
                    v[i + j * step] += e;
                }
            }
        } else {
            let mut i = 0;
            for _ in 0..step {
                let Some(entry) = book.vector(r)? else { return Ok(()) };
                for &e in entry {
                    v[i] += e;
                    i += 1;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_class_dimensions_keep_only_actual_partitions() {
        let mut data = Vec::<u8>::new();
        let mut bits = 0usize;
        let mut put = |value: u32, n: usize| {
            for i in 0..n {
                if bits.is_multiple_of(8) {
                    data.push(0);
                }
                let last = data.len() - 1;
                data[last] |= (((value >> i) & 1) as u8) << (bits % 8);
                bits += 1;
            }
        };
        put(0x56_4342, 24);
        put(65535, 16);
        put(1, 24);
        put(0, 1); // unordered, not sparse, one length-1 entry
        put(0, 1);
        put(0, 5);
        put(0, 4); // scalar-only classbook
        let book = Codebook::read(&mut BitReader::new(&data), &mut (1 << 20)).expect("classbook");
        let books = [book];
        for kind in [0, 1] {
            let residue = Residue { kind, begin: 0, end: 32, partition_size: 1, classifications: 1, classbook: 0, books: vec![[None; 8]] };
            let mut out = vec![vec![0.0; 32]; 255];
            let packet = [0u8; 32];
            let mut r = BitReader::new(&packet);
            residue.decode(&mut r, &books, 32, &[false; 255], &mut out);
            assert_eq!(r.remaining(), 1); // one classword per channel, not 32
            assert!(out.iter().flatten().all(|&x| x == 0.0));
            let mut r = BitReader::new(&packet);
            residue.decode(&mut r, &books, 32, &[true; 255], &mut out);
            assert_eq!(r.remaining(), 256); // skipped channels consume nothing
        }
    }
}
