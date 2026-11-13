//! Single-pass average bitrate control at picture level, after the target bit allocation of the
//! MPEG-2 Test Model 5: each picture type keeps a complexity `X = bits · average quantiser`, the
//! bits left for the GOP are shared out by complexity (B pictures weighted down by `KB`), and a
//! picture's quantiser is `X / target`. There is no VBV model: the average converges on the
//! target, single pictures may not.

use crate::mpeg2::headers::PictureType;

/// TM5's quantiser ratio of B to P pictures (K_b / K_p).
const KB: f64 = 1.4;

pub(crate) struct RateCtl {
    bps: f64,
    fps: f64,
    /// Complexity of I, P and B pictures.
    xi: f64,
    xp: f64,
    xb: f64,
    /// Bits left for the current GOP (carries over between GOPs).
    remaining: f64,
    /// P and B pictures left in the current GOP.
    np: u32,
    nb: u32,
    /// Range of quantiser_scale values.
    q_min: f64,
    q_max: f64,
}

impl RateCtl {
    pub fn new(bps: f64, fps: f64, q_range: (f64, f64)) -> Self {
        // TM5's starting complexities
        Self {
            bps,
            fps,
            xi: 160.0 * bps / 115.0,
            xp: 60.0 * bps / 115.0,
            xb: 42.0 * bps / 115.0,
            remaining: 0.0,
            np: 0,
            nb: 0,
            q_min: q_range.0,
            q_max: q_range.1,
        }
    }

    /// A GOP of one I, `np` P and `nb` B pictures starts.
    pub fn start_gop(&mut self, np: u32, nb: u32) {
        self.remaining += self.bps * f64::from(1 + np + nb) / self.fps;
        self.np = np;
        self.nb = nb;
    }

    /// quantiser_scale for the next picture of type `ptype`.
    pub fn qscale(&self, ptype: PictureType) -> f64 {
        let floor = self.bps / (8.0 * self.fps);
        let (np, nb) = (f64::from(self.np), f64::from(self.nb));
        let (target, x) = match ptype {
            PictureType::I => (self.remaining / (1.0 + np * self.xp / self.xi + nb * self.xb / (self.xi * KB)), self.xi),
            PictureType::B => (self.remaining / (nb.max(1.0) + np * KB * self.xp / self.xb), self.xb),
            _ => (self.remaining / (np.max(1.0) + nb * self.xb / (KB * self.xp)), self.xp),
        };
        (x / target.max(floor)).clamp(self.q_min, self.q_max)
    }

    /// A picture of type `ptype` used `bits` at average quantiser_scale `avg_q`.
    pub fn update(&mut self, ptype: PictureType, bits: u64, avg_q: f64) {
        let x = (bits as f64 * avg_q).max(1.0);
        match ptype {
            PictureType::I => self.xi = x,
            PictureType::B => {
                self.xb = x;
                self.nb = self.nb.saturating_sub(1);
            }
            _ => {
                self.xp = x;
                self.np = self.np.saturating_sub(1);
            }
        }
        self.remaining -= bits as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture whose size is `complexity / q` (the model the controller assumes), in GOPs of
    /// 12 pictures with `b` B pictures between anchors.
    fn run(c: [f64; 3], b: usize, bps: f64, gops: u32) -> f64 {
        let fps = 25.0;
        let types: Vec<PictureType> = (0..12)
            .map(|k| match k {
                0 => PictureType::I,
                k if k % (b + 1) == 0 => PictureType::P,
                _ => PictureType::B,
            })
            .collect();
        let np = types.iter().filter(|&&t| t == PictureType::P).count() as u32;
        let mut rc = RateCtl::new(bps, fps, (1.0, 112.0));
        let mut total = 0.0;
        for _ in 0..gops {
            rc.start_gop(np, 11 - np);
            for &t in &types {
                let q = rc.qscale(t);
                let bits = c[t as usize] / q;
                rc.update(t, bits as u64, q);
                total += bits;
            }
        }
        total * fps / (12.0 * f64::from(gops))
    }

    #[test]
    fn converges_on_the_target() {
        // complexities whose target quantiser is inside the allowed range
        for (c, b) in [([4e7, 1e7, 7e6], 0), ([8e7, 2e6, 1e6], 0), ([1e7, 9e6, 8e6], 0), ([4e7, 1e7, 7e6], 2), ([6e7, 2e7, 5e6], 1)] {
            let got = run(c, b, 4e6, 20);
            assert!((got / 4e6 - 1.0).abs() < 0.03, "{c:?} b {b}: {got}");
        }
    }

    #[test]
    fn the_quantiser_stays_in_range() {
        let mut rc = RateCtl::new(1e6, 25.0, (2.0, 62.0));
        rc.start_gop(1, 0);
        // a picture far larger than its budget pushes the next quantiser to the top
        rc.update(PictureType::I, 50_000_000, 10.0);
        rc.start_gop(1, 0);
        assert_eq!(rc.qscale(PictureType::I), 62.0);
        // and a tiny one to the bottom
        let mut rc = RateCtl::new(1e6, 25.0, (2.0, 62.0));
        rc.start_gop(1, 0);
        rc.update(PictureType::I, 10, 2.0);
        assert_eq!(rc.qscale(PictureType::I), 2.0);
    }
}
