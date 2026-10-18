//! Musical scales for note quantizing, transposing, chords, and
//! arpeggios: a 12-bit mask of pitch classes above a root.

/// Built-in scales: name and pitch-class mask (bit `i` = `i` semitones
/// above the root).
pub const SCALES: &[(&str, u16)] = &[
    ("chromatic", 0b1111_1111_1111),
    ("major", mask(&[0, 2, 4, 5, 7, 9, 11])),
    ("minor", mask(&[0, 2, 3, 5, 7, 8, 10])),
    ("harmonic_minor", mask(&[0, 2, 3, 5, 7, 8, 11])),
    ("melodic_minor", mask(&[0, 2, 3, 5, 7, 9, 11])),
    ("dorian", mask(&[0, 2, 3, 5, 7, 9, 10])),
    ("phrygian", mask(&[0, 1, 3, 5, 7, 8, 10])),
    ("lydian", mask(&[0, 2, 4, 6, 7, 9, 11])),
    ("mixolydian", mask(&[0, 2, 4, 5, 7, 9, 10])),
    ("locrian", mask(&[0, 1, 3, 5, 6, 8, 10])),
    ("major_pentatonic", mask(&[0, 2, 4, 7, 9])),
    ("minor_pentatonic", mask(&[0, 3, 5, 7, 10])),
    ("blues", mask(&[0, 3, 5, 6, 7, 10])),
    ("whole_tone", mask(&[0, 2, 4, 6, 8, 10])),
];

/// Names of [`SCALES`] plus `custom`, for a choice socket.
pub const SCALE_NAMES: &[&str] = &[
    "chromatic",
    "major",
    "minor",
    "harmonic_minor",
    "melodic_minor",
    "dorian",
    "phrygian",
    "lydian",
    "mixolydian",
    "locrian",
    "major_pentatonic",
    "minor_pentatonic",
    "blues",
    "whole_tone",
    "custom",
];

/// Note names for a root choice socket, C = 0.
pub const ROOT_NAMES: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

const fn mask(steps: &[u8]) -> u16 {
    let mut m = 0u16;
    let mut i = 0;
    while i < steps.len() {
        m |= 1 << steps[i];
        i += 1;
    }
    m
}

/// A set of pitch classes above a root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    mask: u16,
}

impl Scale {
    /// From a mask; an empty mask becomes chromatic so quantizing always
    /// has somewhere to go.
    pub fn from_mask(mask: u16) -> Scale {
        let m = mask & 0xfff;
        Scale {
            mask: if m == 0 { 0xfff } else { m },
        }
    }

    /// Scale number `index` of [`SCALE_NAMES`]; the last (`custom`) uses
    /// `custom` as the mask.
    pub fn from_index(index: usize, custom: u16) -> Scale {
        match SCALES.get(index) {
            Some((_, m)) => Scale::from_mask(*m),
            None => Scale::from_mask(custom),
        }
    }

    pub fn mask(self) -> u16 {
        self.mask
    }

    /// Whether `note` (any octave) is in the scale over `root`.
    pub fn contains(self, note: i32, root: i32) -> bool {
        let pc = (note - root).rem_euclid(12);
        self.mask & (1 << pc) != 0
    }

    /// The nearest scale note to `note` (fractional MIDI note number). Ties
    /// go down.
    pub fn quantize(self, note: f64, root: i32) -> f64 {
        if !note.is_finite() {
            return 0.0;
        }
        let base = note.floor() as i64;
        let mut best = base as f64;
        let mut best_d = f64::INFINITY;
        for k in -12i64..=12 {
            let n = base + k;
            if self.contains(n as i32, root) {
                let d = (n as f64 - note).abs();
                if d < best_d - 1e-9 {
                    best_d = d;
                    best = n as f64;
                }
            }
        }
        best
    }

    /// Notes in one octave (the number of set bits).
    pub fn len(self) -> usize {
        self.mask.count_ones() as usize
    }

    pub fn is_empty(self) -> bool {
        self.mask == 0
    }

    /// Note number of scale degree `degree` (0 = the root, negative goes
    /// down), with `root` the root's note number.
    pub fn degree(self, degree: i32, root: i32) -> i32 {
        let n = self.len().max(1) as i32;
        let octave = degree.div_euclid(n);
        let step = degree.rem_euclid(n);
        let mut seen = 0;
        for pc in 0..12 {
            if self.mask & (1 << pc) != 0 {
                if seen == step {
                    return root + octave * 12 + pc;
                }
                seen += 1;
            }
        }
        root + octave * 12
    }

    /// Move `note` by `steps` scale degrees, after quantizing it.
    pub fn transpose(self, note: i32, steps: i32, root: i32) -> i32 {
        let q = self.quantize(f64::from(note), root) as i32;
        // The degree of q: count scale notes from the root up to it.
        let n = self.len().max(1) as i32;
        let rel = q - root;
        let octave = rel.div_euclid(12);
        let pc = rel.rem_euclid(12);
        let below = (0..pc).filter(|p| self.mask & (1 << p) != 0).count() as i32;
        self.degree(octave * n + below + steps, root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_snaps_to_the_nearest_scale_note() {
        let c_major = Scale::from_index(1, 0);
        assert_eq!(c_major.quantize(61.0, 0), 60.0, "C# ties down to C");
        assert_eq!(c_major.quantize(61.6, 0), 62.0);
        assert_eq!(c_major.quantize(66.0, 0), 65.0, "F# → F");
        // A minor pentatonic over A (root 9).
        let pent = Scale::from_index(11, 0);
        assert_eq!(pent.quantize(70.0, 9), 69.0);
        assert!(pent.contains(72, 9) && !pent.contains(71, 9));
        assert_eq!(Scale::from_mask(0).len(), 12, "empty becomes chromatic");
        assert_eq!(c_major.quantize(f64::NAN, 0), 0.0);
    }

    #[test]
    fn degrees_and_transposition_walk_the_scale() {
        let c_major = Scale::from_index(1, 0);
        assert_eq!(c_major.degree(0, 60), 60);
        assert_eq!(c_major.degree(2, 60), 64);
        assert_eq!(c_major.degree(7, 60), 72);
        assert_eq!(c_major.degree(-1, 60), 59);
        assert_eq!(c_major.transpose(60, 2, 0), 64, "C up a third is E");
        assert_eq!(c_major.transpose(64, 1, 0), 65);
        assert_eq!(c_major.transpose(71, 1, 0), 72);
        let custom = Scale::from_index(SCALE_NAMES.len() - 1, 0b1001_0001);
        assert_eq!(custom.len(), 3);
        assert_eq!(SCALE_NAMES.len(), SCALES.len() + 1);
    }
}
