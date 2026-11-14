//! Single-pass average bitrate control at picture level, after the target bit allocation of the
//! MPEG-2 Test Model 5: each picture type keeps a complexity `X = bits · average quantiser`, the
//! bits left for the GOP are shared out by complexity, and a picture's quantiser is `X / target`.
//! There is no VBV model: the average converges on the target, single pictures may not.

pub(crate) struct RateCtl {
    bps: f64,
    fps: f64,
    /// Complexity of I and P pictures.
    xi: f64,
    xp: f64,
    /// Bits left for the current GOP (carries over between GOPs).
    remaining: f64,
    /// P pictures left in the current GOP.
    np: u32,
    /// Range of quantiser_scale values.
    q_min: f64,
    q_max: f64,
}

impl RateCtl {
    pub fn new(bps: f64, fps: f64, q_range: (f64, f64)) -> Self {
        // TM5's starting complexities
        Self { bps, fps, xi: 160.0 * bps / 115.0, xp: 60.0 * bps / 115.0, remaining: 0.0, np: 0, q_min: q_range.0, q_max: q_range.1 }
    }

    /// A GOP of `pictures` pictures (one I, the rest P) starts.
    pub fn start_gop(&mut self, pictures: u32) {
        self.remaining += self.bps * f64::from(pictures) / self.fps;
        self.np = pictures.saturating_sub(1);
    }

    /// quantiser_scale for the next picture.
    pub fn qscale(&self, intra: bool) -> f64 {
        let floor = self.bps / (8.0 * self.fps);
        let (target, x) = if intra {
            (self.remaining / (1.0 + f64::from(self.np) * self.xp / self.xi), self.xi)
        } else {
            (self.remaining / f64::from(self.np.max(1)), self.xp)
        };
        (x / target.max(floor)).clamp(self.q_min, self.q_max)
    }

    /// The picture used `bits` at average quantiser_scale `avg_q`.
    pub fn update(&mut self, intra: bool, bits: u64, avg_q: f64) {
        let x = bits as f64 * avg_q;
        if intra {
            self.xi = x.max(1.0);
        } else {
            self.xp = x.max(1.0);
            self.np = self.np.saturating_sub(1);
        }
        self.remaining -= bits as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture whose size is `complexity / q` (the model the controller assumes).
    fn run(complexity_i: f64, complexity_p: f64, bps: f64, gops: u32) -> f64 {
        let fps = 25.0;
        let mut rc = RateCtl::new(bps, fps, (1.0, 112.0));
        let mut total = 0.0;
        for _ in 0..gops {
            rc.start_gop(12);
            for k in 0..12 {
                let intra = k == 0;
                let q = rc.qscale(intra);
                let bits = (if intra { complexity_i } else { complexity_p }) / q;
                rc.update(intra, bits as u64, q);
                total += bits;
            }
        }
        total * fps / (12.0 * f64::from(gops))
    }

    #[test]
    fn converges_on_the_target() {
        // complexities whose target quantiser is inside the allowed range
        for (ci, cp) in [(4e7, 1e7), (8e7, 2e6), (1e7, 9e6)] {
            let got = run(ci, cp, 4e6, 20);
            assert!((got / 4e6 - 1.0).abs() < 0.03, "{ci} {cp}: {got}");
        }
    }

    #[test]
    fn the_quantiser_stays_in_range() {
        let mut rc = RateCtl::new(1e6, 25.0, (2.0, 62.0));
        rc.start_gop(2);
        // a picture far larger than its budget pushes the next quantiser to the top
        rc.update(true, 50_000_000, 10.0);
        rc.start_gop(2);
        assert_eq!(rc.qscale(true), 62.0);
        // and a tiny one to the bottom
        let mut rc = RateCtl::new(1e6, 25.0, (2.0, 62.0));
        rc.start_gop(2);
        rc.update(true, 10, 2.0);
        assert_eq!(rc.qscale(true), 2.0);
    }
}
