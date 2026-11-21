//! What one read may take. The values are VectorCraft's own; wasm32 gets half the memory limits.

use crate::Error;

/// The most a read may take, checked before every allocation and descent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of input.
    pub input: usize,
    /// Bytes inflated from compressed lists and package members, all of one file together.
    pub inflated: usize,
    /// Bytes of one package member.
    pub member: usize,
    /// Members of a package.
    pub members: usize,
    /// Nested lists below the file's own. Deeper content is left out with a warning.
    pub depth: usize,
    /// Chunks in the index, nested ones included, and the entries and styles of the style table;
    /// in a `.cmx` file, its chunks, table entries and page instructions.
    pub chunks: usize,
    /// Objects mapped (shapes, groups and guidelines).
    pub objects: usize,
    /// Points of all curves together.
    pub coordinates: usize,
    /// Arguments of one `loda` record.
    pub arguments: usize,
    /// Dashes of all outlines together, an outline's counted again for every shape that uses it.
    /// Past them a shape's outline is solid, with a warning.
    pub dashes: usize,
    /// Chunks walked inside clipped groups, all containers together, a group's counted again for
    /// every container that names it. Each search of an object's list for its `loda`, `flgs` and
    /// `trfd` runs once per read and counts every chunk it looks at. Past them a container's group
    /// is left out, with a warning.
    pub clipped_chunks: usize,
}

#[cfg(not(target_arch = "wasm32"))]
const MIB: usize = 1 << 20;

impl Default for Limits {
    #[cfg(not(target_arch = "wasm32"))]
    fn default() -> Self {
        Limits {
            input: 512 * MIB,
            inflated: 1024 * MIB,
            member: 512 * MIB,
            members: 4096,
            depth: 256,
            chunks: 2_000_000,
            objects: 1_000_000,
            coordinates: 16_000_000,
            arguments: 64,
            dashes: 16_000_000,
            clipped_chunks: 2_000_000,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn default() -> Self {
        Limits {
            input: 256 << 20,
            inflated: 512 << 20,
            member: 256 << 20,
            members: 4096,
            depth: 256,
            chunks: 2_000_000,
            objects: 1_000_000,
            coordinates: 16_000_000,
            arguments: 64,
            dashes: 8_000_000,
            clipped_chunks: 2_000_000,
        }
    }
}

/// The running counters of one read against its [`Limits`].
#[derive(Debug)]
pub(crate) struct Budget {
    pub(crate) limits: Limits,
    inflated: usize,
    chunks: usize,
    objects: usize,
    coordinates: usize,
    dashes: usize,
    clipped_chunks: usize,
    /// A request for coordinates was refused since [`Budget::coordinates_refused`] last asked.
    refused: bool,
}

impl Budget {
    pub(crate) fn new(limits: Limits) -> Self {
        Budget { limits, inflated: 0, chunks: 0, objects: 0, coordinates: 0, dashes: 0, clipped_chunks: 0, refused: false }
    }

    /// Whether a request for coordinates was refused since the last call, which tells a shape
    /// left out at the limit from one whose bytes don't parse.
    pub(crate) fn coordinates_refused(&mut self) -> bool {
        std::mem::take(&mut self.refused)
    }

    /// Bytes an inflate may still produce.
    pub(crate) fn inflate_left(&self) -> usize {
        self.limits.inflated.saturating_sub(self.inflated)
    }

    pub(crate) fn take_inflated(&mut self, n: usize) -> Result<(), Error> {
        self.inflated = self.inflated.checked_add(n).filter(|&t| t <= self.limits.inflated).ok_or(Error::Limit("inflated size"))?;
        Ok(())
    }

    pub(crate) fn take_chunk(&mut self) -> Result<(), Error> {
        self.take_chunks(1)
    }

    /// `n` more chunks or style table entries.
    pub(crate) fn take_chunks(&mut self, n: usize) -> Result<(), Error> {
        self.chunks = self.chunks.saturating_add(n);
        if self.chunks > self.limits.chunks { Err(Error::Limit("chunk count")) } else { Ok(()) }
    }

    /// One more object, or `false` when the drawing already holds as many as a read maps.
    pub(crate) fn take_object(&mut self) -> bool {
        if self.objects >= self.limits.objects {
            return false;
        }
        self.objects += 1;
        true
    }

    /// `n` more curve points, or `false` past the limit (nothing is taken then).
    pub(crate) fn take_coordinates(&mut self, n: usize) -> bool {
        match self.coordinates.checked_add(n).filter(|&t| t <= self.limits.coordinates) {
            Some(t) => {
                self.coordinates = t;
                true
            }
            None => {
                self.refused = true;
                false
            }
        }
    }

    /// Dashes the read may still take.
    pub(crate) fn dashes_left(&self) -> usize {
        self.limits.dashes.saturating_sub(self.dashes)
    }

    /// `n` more dashes, or `false` past the limit (nothing is taken then).
    pub(crate) fn take_dashes(&mut self, n: usize) -> bool {
        match self.dashes.checked_add(n).filter(|&t| t <= self.limits.dashes) {
            Some(t) => {
                self.dashes = t;
                true
            }
            None => false,
        }
    }

    /// One more chunk walked inside a clipped group, or `false` past the limit.
    pub(crate) fn take_clipped_chunk(&mut self) -> bool {
        if self.clipped_chunks >= self.limits.clipped_chunks {
            return false;
        }
        self.clipped_chunks += 1;
        true
    }
}
