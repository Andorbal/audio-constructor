//! Smoothing for control values that arrive once per block.

/// Glides linearly from the previous block's value to this block's, so a
/// block-rate control (a slider, a transport value) doesn't step at block
/// boundaries. The first value it sees is taken as-is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockRamp {
    current: f64,
    primed: bool,
}

impl BlockRamp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a block heading to `target` over `frames`. Returns `(start,
    /// step)`: frame `i` has the value `start + step * (i + 1)`, so the last
    /// frame lands exactly on `target`.
    pub fn begin(&mut self, target: f64, frames: usize) -> (f64, f64) {
        if !self.primed || !self.current.is_finite() {
            self.current = target;
            self.primed = true;
        }
        let start = self.current;
        let step = (target - start) / frames.max(1) as f64;
        self.current = target;
        (start, step)
    }

    /// Jump to `value` (when the control is followed per sample instead).
    pub fn set(&mut self, value: f64) {
        self.current = value;
        self.primed = true;
    }

    /// Forget the last value; the next block starts without a glide.
    pub fn reset(&mut self) {
        self.primed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_block_is_constant_then_glides() {
        let mut r = BlockRamp::new();
        assert_eq!(r.begin(10.0, 4), (10.0, 0.0));
        let (start, step) = r.begin(18.0, 4);
        assert_eq!((start, step), (10.0, 2.0));
        assert_eq!(start + step * 4.0, 18.0);
        r.reset();
        assert_eq!(r.begin(3.0, 8), (3.0, 0.0));
    }

    #[test]
    fn set_moves_the_glide_origin() {
        let mut r = BlockRamp::new();
        r.set(5.0);
        assert_eq!(r.begin(6.0, 1), (5.0, 1.0));
        assert_eq!(r.begin(6.0, 0), (6.0, 0.0));
    }
}
