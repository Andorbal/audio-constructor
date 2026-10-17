//! Small helpers shared across the crate.

/// SplitMix64 step: a fast, well-distributed 64-bit generator used for ids
/// and seed derivation (Steele, Lea & Flood, "Fast Splittable Pseudorandom
/// Number Generators", OOPSLA 2014).
pub fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a 64-bit hash. Stable across platforms and releases, unlike
/// `std::hash`, so it can be stored in files (asset hashes) and used for seeds.
#[derive(Debug, Clone)]
pub struct Fnv64(u64);

impl Fnv64 {
    pub fn new() -> Self {
        Fnv64(0xcbf2_9ce4_8422_2325)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv64 {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash a byte slice with FNV-1a and format it the way asset references store it.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut h = Fnv64::new();
    h.write(bytes);
    format!("fnv1a64:{:016x}", h.finish())
}

/// Derive a per-node seed from the project seed and a node path hash.
pub fn derive_seed(project_seed: u64, path_hash: u64) -> u64 {
    let mut s = project_seed ^ path_hash.rotate_left(17);
    splitmix64(&mut s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_known_vector() {
        // FNV-1a 64 of "a" is af63dc4c8601ec8c.
        let mut h = Fnv64::new();
        h.write(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
    }
}
