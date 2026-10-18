//! The data a `Spectrum` wire carries for one block: the FFT frames that
//! completed during the block, each stamped with its sample offset.
//!
//! Storage is allocated once, when the graph is compiled, for the most frames
//! a block can hold; pushing past that drops the frame instead of growing, so
//! nothing on the audio thread allocates.

use crate::signal::SpectrumShape;

/// Frames of `shape.bins()` complex bins, interleaved `re, im`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpectrumBuf {
    shape: Option<SpectrumShape>,
    offsets: Vec<u32>,
    data: Vec<f32>,
}

impl SpectrumBuf {
    /// Storage for up to `max_frames` frames of `shape`.
    pub fn new(shape: SpectrumShape, max_frames: usize) -> Self {
        SpectrumBuf {
            shape: Some(shape),
            offsets: Vec::with_capacity(max_frames),
            data: Vec::with_capacity(max_frames * shape.bins() * 2),
        }
    }

    /// The frame layout, or `None` for a buffer that holds nothing (an
    /// unwired input).
    pub fn shape(&self) -> Option<SpectrumShape> {
        self.shape
    }

    /// Bins per frame (0 without a shape).
    pub fn bins(&self) -> usize {
        self.shape.map_or(0, SpectrumShape::bins)
    }

    /// Frames in this block.
    pub fn frames(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Frame `k`: its sample offset in the block and its bins (`re, im`
    /// pairs).
    pub fn frame(&self, k: usize) -> Option<(u32, &[f32])> {
        let offset = *self.offsets.get(k)?;
        let n = self.bins() * 2;
        let bins = self.data.get(k * n..(k + 1) * n)?;
        Some((offset, bins))
    }

    /// Iterate over `(offset, bins)` in time order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &[f32])> {
        (0..self.frames()).filter_map(|k| self.frame(k))
    }

    /// Append a frame completing at `offset` and return its bins to fill
    /// (zeroed). Returns `None`, dropping the frame, if the preallocated
    /// storage is full or the offset would go backwards.
    pub fn push(&mut self, offset: u32) -> Option<&mut [f32]> {
        let n = self.bins() * 2;
        if n == 0
            || self.offsets.len() == self.offsets.capacity()
            || self.data.len() + n > self.data.capacity()
            || self.offsets.last().is_some_and(|&o| o > offset)
        {
            return None;
        }
        self.offsets.push(offset);
        let start = self.data.len();
        self.data.resize(start + n, 0.0);
        self.data.get_mut(start..)
    }

    /// Append a copy of a frame (bins beyond this buffer's size are dropped,
    /// missing ones are zero).
    pub fn push_copy(&mut self, offset: u32, bins: &[f32]) -> bool {
        match self.push(offset) {
            Some(dst) => {
                let n = dst.len().min(bins.len());
                dst[..n].copy_from_slice(&bins[..n]);
                true
            }
            None => false,
        }
    }

    /// Remove every frame (keeps the storage).
    pub fn clear(&mut self) {
        self.offsets.clear();
        self.data.clear();
    }
}

/// An empty spectrum, for unwired inputs.
pub static EMPTY_SPECTRUM: SpectrumBuf = SpectrumBuf {
    shape: None,
    offsets: Vec::new(),
    data: Vec::new(),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fill_preallocated_storage_and_then_drop() {
        let shape = SpectrumShape::new(16, 8);
        let mut s = SpectrumBuf::new(shape, 2);
        assert_eq!(s.bins(), 9);
        s.push(3).unwrap()[0] = 1.0;
        assert!(s.push_copy(7, &[2.0; 18]));
        assert!(s.push(9).is_none(), "full");
        assert_eq!(s.frames(), 2);
        let (o, b) = s.frame(0).unwrap();
        assert_eq!((o, b[0], b.len()), (3, 1.0, 18));
        assert_eq!(s.frame(1).unwrap().1[17], 2.0);
        s.clear();
        assert!(s.is_empty());
        assert!(s.push(1).is_some());
    }

    #[test]
    fn offsets_never_go_backwards() {
        let mut s = SpectrumBuf::new(SpectrumShape::new(16, 4), 4);
        assert!(s.push(5).is_some());
        assert!(s.push(2).is_none());
        assert!(s.push(5).is_some());
    }

    #[test]
    fn empty_spectrum_has_no_frames() {
        assert_eq!(EMPTY_SPECTRUM.frames(), 0);
        assert_eq!(EMPTY_SPECTRUM.bins(), 0);
        assert!(EMPTY_SPECTRUM.frame(0).is_none());
    }
}
