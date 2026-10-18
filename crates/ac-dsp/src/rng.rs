//! The seeded PRNG every random node uses (ARCHITECTURE §10: all randomness
//! comes from a generator seeded from the project seed and the node's path,
//! `PrepareCtx::seed`).
//!
//! xoshiro256++ (Blackman & Vigna, "Scrambled linear pseudorandom number
//! generators", 2018), seeded through SplitMix64 so any 64-bit seed,
//! including 0, gives a good state.

use ac_core::util::splitmix64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    s: [u64; 4],
    /// The second value of the last Box-Muller pair.
    spare: Option<u64>,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut sm = seed;
        Rng {
            s: [
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
            ],
            spare: None,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, 1)`, with 53 bits of precision.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[-1, 1)`.
    pub fn bipolar(&mut self) -> f64 {
        self.uniform() * 2.0 - 1.0
    }

    /// Uniform in `[lo, hi)` (or `lo` when the range is empty).
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    /// An integer in `0..n` (0 when `n` is 0), without modulo bias.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        // Lemire's multiply-shift with rejection.
        loop {
            let x = self.next_u64();
            let m = u128::from(x) * u128::from(n);
            let low = m as u64;
            if low >= n || low >= n.wrapping_neg() % n {
                return (m >> 64) as u64;
            }
        }
    }

    /// Standard normal (mean 0, deviation 1), Box-Muller.
    pub fn normal(&mut self) -> f64 {
        if let Some(bits) = self.spare.take() {
            return f64::from_bits(bits);
        }
        let u1 = self.uniform().max(f64::MIN_POSITIVE);
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        let (s, c) = (std::f64::consts::TAU * u2).sin_cos();
        self.spare = Some((r * s).to_bits());
        r * c
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.uniform() < p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream_and_seeds_differ() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let mut c = Rng::new(43);
        let xs: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        let zs: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(xs, ys);
        assert_ne!(xs, zs);
        // Seed 0 is fine too.
        assert_ne!(Rng::new(0).next_u64(), 0);
    }

    #[test]
    fn distributions_look_right() {
        let mut r = Rng::new(7);
        let n = 100_000;
        let mean: f64 = (0..n).map(|_| r.uniform()).sum::<f64>() / n as f64;
        assert!((mean - 0.5).abs() < 0.01, "{mean}");
        let xs: Vec<f64> = (0..n).map(|_| r.normal()).collect();
        let m = xs.iter().sum::<f64>() / n as f64;
        let var = xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n as f64;
        assert!(m.abs() < 0.02 && (var - 1.0).abs() < 0.03, "{m} {var}");
        let mut counts = [0u32; 5];
        for _ in 0..n {
            counts[r.below(5) as usize] += 1;
        }
        assert!(
            counts
                .iter()
                .all(|&c| (c as f64 / n as f64 - 0.2).abs() < 0.01)
        );
        assert_eq!(r.below(0), 0);
        assert!((0..1000).all(|_| (-1.0..1.0).contains(&r.bipolar())));
    }
}
