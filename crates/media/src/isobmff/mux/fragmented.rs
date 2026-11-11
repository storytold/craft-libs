//! The fragmented writer: [`FragmentedWriter`].

use super::boxes::{write_ftyp, write_mvhd, write_trak, write_udta};
use super::*;

/// Buffered fragment sample: (data, duration, composition offset, sync).
type PendingSample = (Vec<u8>, u32, i32, bool);

/// Fragmented MP4 writer: an init segment (`ftyp` + `moov` with `mvex`) followed by
/// `moof`+`mdat` fragments. Samples are buffered until [`flush_fragment`](Self::flush_fragment).
pub struct FragmentedWriter<W: Write> {
    w: W,
    opts: WriterOptions,
    tracks: Vec<WTrack>,
    pending: Vec<Vec<PendingSample>>,
    next_dts: Vec<u64>,
    seq: u32,
    init_written: bool,
}

impl<W: Write> FragmentedWriter<W> {
    /// Creates a writer that writes to `w` with the given options. The init segment is written with the first fragment.
    pub fn new(w: W, opts: WriterOptions) -> Self {
        FragmentedWriter { w, opts, tracks: Vec::new(), pending: Vec::new(), next_dts: Vec::new(), seq: 0, init_written: false }
    }

    /// Add a track (before the first fragment). PCM is not supported in fragments.
    pub fn add_track(&mut self, cfg: TrackConfig) -> Result<usize> {
        if self.init_written {
            return Err(Error::Mux("tracks must be added before the first fragment".into()));
        }
        if validate_track(&cfg.entry, cfg.timescale)?.is_some() {
            return Err(Error::Mux("PCM is not supported by the fragmented writer".into()));
        }
        let handler = handler_of(&cfg.entry);
        self.tracks.push(WTrack {
            id: self.tracks.len() as u32 + 1,
            handler,
            pcm_frame_bytes: None,
            t: Tables::new(),
            cfg,
            tmcd_ref: None,
            tmcd_source: None,
            chap_ref: None,
        });
        self.pending.push(Vec::new());
        self.next_dts.push(0);
        Ok(self.tracks.len() - 1)
    }

    fn write_init(&mut self) -> Result<()> {
        let qt = self.opts.brand == Brand::Mov;
        let ts = self.opts.movie_timescale.max(1);
        let mut b = BoxBuf::new();
        write_ftyp(&mut b, self.opts.brand);
        let m = b.start(b"moov");
        write_mvhd(&mut b, ts, 0, self.tracks.len() as u32 + 1);
        for t in &self.tracks {
            write_trak(&mut b, t, ts, qt, 0, false)?;
        }
        let mv = b.start(b"mvex");
        for t in &self.tracks {
            let x = b.start_full(b"trex", 0, 0);
            b.u32(t.id);
            b.u32(1);
            b.u32(0);
            b.u32(0);
            b.u32(0);
            b.end(x);
        }
        b.end(mv);
        write_udta(&mut b, &self.opts.metadata, &normalize_chapters(&self.opts.chapters, None), ts);
        b.end(m);
        self.w.write_all(&b.buf)?;
        self.init_written = true;
        Ok(())
    }

    /// Buffer a sample for the next fragment.
    pub fn write_sample(&mut self, track: usize, s: WriteSample) -> Result<()> {
        let p = self.pending.get_mut(track).ok_or(Error::NoSuchTrack(track))?;
        if s.data.len() > u32::MAX as usize {
            return Err(Error::Mux("sample larger than 4 GiB".into()));
        }
        p.push((s.data.to_vec(), s.duration, s.composition_offset, s.is_sync));
        Ok(())
    }

    fn build_moof(&self, data_offsets: &[i32]) -> Vec<u8> {
        let mut b = BoxBuf::new();
        let m = b.start(b"moof");
        let h = b.start_full(b"mfhd", 0, 0);
        b.u32(self.seq);
        b.end(h);
        for (i, t) in self.tracks.iter().enumerate() {
            let p = &self.pending[i];
            if p.is_empty() {
                continue;
            }
            let tf = b.start(b"traf");
            let h = b.start_full(b"tfhd", 0, 0x20000);
            b.u32(t.id);
            b.end(h);
            let h = b.start_full(b"tfdt", 1, 0);
            b.u64(self.next_dts[i]);
            b.end(h);
            let neg = p.iter().any(|s| s.2 < 0);
            let h = b.start_full(b"trun", neg as u8, 0x1 | 0x100 | 0x200 | 0x400 | 0x800);
            b.u32(p.len() as u32);
            b.i32(data_offsets[i]);
            for (d, dur, cto, sync) in p {
                b.u32(*dur);
                b.u32(d.len() as u32);
                b.u32(if *sync { 0x0200_0000 } else { 0x0101_0000 });
                b.i32(*cto);
            }
            b.end(h);
            b.end(tf);
        }
        b.end(m);
        b.buf
    }

    /// Write all buffered samples as one `moof` + `mdat` fragment (no-op if nothing is buffered).
    pub fn flush_fragment(&mut self) -> Result<()> {
        if !self.init_written {
            self.write_init()?;
        }
        if self.pending.iter().all(|p| p.is_empty()) {
            return Ok(());
        }
        self.seq += 1;
        let probe = self.build_moof(&vec![0; self.tracks.len()]);
        let data_len: u64 = self.pending.iter().flatten().map(|s| s.0.len() as u64).sum();
        let large = data_len + 8 > u32::MAX as u64;
        let hdr = if large { 16 } else { 8 };
        let mut offs = Vec::with_capacity(self.tracks.len());
        let mut acc = probe.len() as u64 + hdr;
        for p in &self.pending {
            offs.push(i32::try_from(acc).map_err(|_| Error::Mux("fragment too large".into()))?);
            acc += p.iter().map(|s| s.0.len() as u64).sum::<u64>();
        }
        let moof = self.build_moof(&offs);
        self.w.write_all(&moof)?;
        if large {
            self.w.write_all(&1u32.to_be_bytes())?;
            self.w.write_all(b"mdat")?;
            self.w.write_all(&(data_len + 16).to_be_bytes())?;
        } else {
            self.w.write_all(&((data_len + 8) as u32).to_be_bytes())?;
            self.w.write_all(b"mdat")?;
        }
        for (i, p) in self.pending.iter_mut().enumerate() {
            for (d, dur, _, _) in p.drain(..) {
                self.w.write_all(&d)?;
                self.next_dts[i] += dur as u64;
            }
        }
        Ok(())
    }

    /// Flush remaining samples and return the writer.
    pub fn finish(mut self) -> Result<W> {
        self.flush_fragment()?;
        self.w.flush()?;
        Ok(self.w)
    }
}
