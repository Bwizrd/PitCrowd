//! A small deterministic generator (SplitMix64), so the same seed gives the same market on
//! every machine and every run, with no dependency.

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// A generator for one (stream, index) pair: minute `m` of symbol `s` always draws the
    /// same numbers, however it is reached.
    pub fn keyed(seed: u64, stream: u64, index: u64) -> Self {
        let mut r = Rng(seed
            ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ index.wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal (Box-Muller; one of the pair is thrown away to keep draws keyed).
    pub fn normal(&mut self) -> f64 {
        let u1 = self.unit().max(1e-300);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Poisson count with mean `lambda` (normal approximation above 30).
    pub fn poisson(&mut self, lambda: f64) -> u32 {
        if lambda <= 0.0 {
            return 0;
        }
        if lambda > 30.0 {
            return (lambda + lambda.sqrt() * self.normal()).round().max(0.0) as u32;
        }
        let l = (-lambda).exp();
        let mut k = 0u32;
        let mut p = 1.0;
        loop {
            p *= self.unit();
            if p <= l {
                return k;
            }
            k += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keyed_stream_is_the_same_however_it_is_reached() {
        let a: Vec<u64> = (0..4).map(|_| Rng::keyed(7, 3, 99).next_u64()).collect();
        assert!(a.windows(2).all(|w| w[0] == w[1]));
        assert_ne!(
            Rng::keyed(7, 3, 99).next_u64(),
            Rng::keyed(7, 3, 100).next_u64()
        );
    }

    #[test]
    fn normals_have_unit_spread() {
        let mut r = Rng::new(1);
        let xs: Vec<f64> = (0..20000).map(|_| r.normal()).collect();
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / xs.len() as f64;
        assert!(
            mean.abs() < 0.03 && (var - 1.0).abs() < 0.05,
            "mean {mean} var {var}"
        );
    }
}
