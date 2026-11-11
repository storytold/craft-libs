//! Movie fragments: `moof` parsing into samples.

use super::*;

pub(super) fn parse_moof(
    moof_off: u64,
    moof: &[u8],
    tracks: &mut [Track],
    track_index: &HashMap<u32, usize>,
    trex: &HashMap<u32, Trex>,
    next_dts: &mut [i64],
    budget: &mut SampleBudget,
) -> Result<()> {
    // End of data of the previous traf, for the base-offset default of subsequent trafs.
    let mut prev_end: Option<u64> = None;
    for b in boxes(moof) {
        let b = b?;
        if b.kind != b"traf" {
            continue;
        }
        let mut tfhd = None;
        let mut tfdt = None;
        let mut truns = Vec::new();
        for c in boxes(b.payload) {
            let c = c?;
            match &c.kind.0 {
                b"tfhd" => tfhd = Some(c.payload),
                b"tfdt" => tfdt = Some(c.payload),
                b"trun" => truns.push(c.payload),
                _ => {}
            }
        }
        let tfhd = tfhd.ok_or_else(|| Error::Invalid("traf without tfhd".into()))?;
        let mut c = Cur::labeled(tfhd, "tfhd");
        let (_, fl) = c.full_header()?;
        let id = c.u32()?;
        let Some(&ti) = track_index.get(&id) else { continue };
        let d = trex.get(&id).copied().unwrap_or_default();
        let base = if fl & 1 != 0 {
            c.u64()?
        } else if fl & 0x20000 != 0 {
            moof_off
        } else {
            prev_end.unwrap_or(moof_off)
        };
        let sdi = if fl & 2 != 0 { c.u32()? } else { d.description_index };
        let def_dur = if fl & 8 != 0 { c.u32()? } else { d.duration };
        let def_size = if fl & 0x10 != 0 { c.u32()? } else { d.size };
        let def_flags = if fl & 0x20 != 0 { c.u32()? } else { d.flags };
        if let Some(p) = tfdt {
            let mut c = Cur::labeled(p, "tfdt");
            let (v, _) = c.full_header()?;
            let t = if v == 1 { c.u64()? } else { c.u32()? as u64 };
            next_dts[ti] = t.min(i64::MAX as u64) as i64;
        }
        let mut data_pos = base;
        for tr in truns {
            let mut c = Cur::labeled(tr, "trun");
            let (v, tf) = c.full_header()?;
            let per = [0x100u32, 0x200, 0x400, 0x800].iter().filter(|&&f| tf & f != 0).count() * 4;
            let n = c.u32()? as usize;
            if per > 0 {
                c.check_count(n, per)?;
            } else {
                let implied = (n as u64).saturating_mul(def_size.max(1) as u64);
                budget.implied_bytes = budget.implied_bytes.checked_sub(implied).ok_or(Error::TooLarge("trun sample count"))?;
            }
            budget.take(n)?;
            if tf & 1 != 0 {
                data_pos = base.wrapping_add_signed(c.i32()? as i64);
            }
            let first_flags = if tf & 4 != 0 { Some(c.u32()?) } else { None };
            let t = &mut tracks[ti];
            t.samples.reserve(n);
            for i in 0..n {
                let dur = if tf & 0x100 != 0 { c.u32()? } else { def_dur };
                let size = if tf & 0x200 != 0 { c.u32()? } else { def_size };
                let flags = if tf & 0x400 != 0 { c.u32()? } else { def_flags };
                // first_sample_flags overrides the flags of the first sample only.
                let flags = if i == 0 { first_flags.unwrap_or(flags) } else { flags };
                let cto = if tf & 0x800 != 0 {
                    let raw = c.u32()?;
                    if v == 0 { raw as i64 } else { raw as i32 as i64 }
                } else {
                    0
                };
                let dts = next_dts[ti];
                t.samples.push(Sample {
                    offset: data_pos,
                    size,
                    dts,
                    pts: dts.saturating_add(cto),
                    duration: dur,
                    is_sync: flags & 0x1_0000 == 0,
                    description_index: sdi,
                });
                data_pos = data_pos.saturating_add(size as u64);
                next_dts[ti] = dts.saturating_add(dur as i64);
            }
        }
        prev_end = Some(data_pos);
    }
    Ok(())
}
