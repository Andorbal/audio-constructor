//! Text as signals carry it: a short string kept inline, so the audio
//! thread can copy, compare, and write it without allocating.

use std::fmt;

/// The most bytes of UTF-8 a [`TextBuf`] holds. Longer text is cut at a
/// character boundary. OSC addresses fit with room to spare
/// ([`crate::control::OSC_ADDRESS_MAX`] is 64).
pub const TEXT_MAX: usize = 128;

/// A `Text` signal's value: up to [`TEXT_MAX`] bytes of UTF-8, inline.
///
/// Writing past the capacity drops the rest (never fails), so formatting
/// into one with `write!` is safe on the audio thread.
#[derive(Clone, Copy)]
pub struct TextBuf {
    bytes: [u8; TEXT_MAX],
    len: u8,
}

impl TextBuf {
    pub const EMPTY: TextBuf = TextBuf {
        bytes: [0; TEXT_MAX],
        len: 0,
    };

    /// `s`, cut to fit at a character boundary.
    pub fn new(s: &str) -> Self {
        let mut t = Self::EMPTY;
        t.push_str(s);
        t
    }

    pub fn as_str(&self) -> &str {
        // The bytes up to `len` always come from `&str` slices cut at
        // character boundaries, so they're valid UTF-8; the checked form
        // keeps this free of `unsafe` and only costs a scan of 128 bytes.
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Append as much of `s` as fits, cut at a character boundary. Returns
    /// whether all of it fit.
    pub fn push_str(&mut self, s: &str) -> bool {
        let room = TEXT_MAX - self.len as usize;
        let mut n = s.len().min(room);
        while n > 0 && !s.is_char_boundary(n) {
            n -= 1;
        }
        let at = self.len as usize;
        self.bytes[at..at + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len = (at + n) as u8;
        n == s.len()
    }

    /// Append one character if it fits.
    pub fn push(&mut self, c: char) -> bool {
        let mut buf = [0u8; 4];
        self.push_str(c.encode_utf8(&mut buf))
    }
}

impl Default for TextBuf {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl fmt::Write for TextBuf {
    /// Never fails: what doesn't fit is dropped, so a formatted number
    /// that overflows the buffer leaves a cut string rather than an error.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

impl PartialEq for TextBuf {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for TextBuf {}

impl PartialEq<str> for TextBuf {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for TextBuf {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl fmt::Debug for TextBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl fmt::Display for TextBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for TextBuf {
    fn from(s: &str) -> Self {
        TextBuf::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn holds_text_and_compares_by_content() {
        let a = TextBuf::new("/fader/1");
        assert_eq!(a.as_str(), "/fader/1");
        assert_eq!(a, "/fader/1");
        assert_eq!(a, TextBuf::new("/fader/1"));
        assert_ne!(a, TextBuf::new("/fader/2"));
        assert!(TextBuf::EMPTY.is_empty());
        assert_eq!(format!("{a}"), "/fader/1");
        assert_eq!(format!("{a:?}"), "\"/fader/1\"");
    }

    #[test]
    fn cuts_at_the_capacity_on_a_character_boundary() {
        let long = "a".repeat(200);
        let t = TextBuf::new(&long);
        assert_eq!(t.len(), TEXT_MAX);
        // 127 ASCII bytes then a 2-byte character: it doesn't fit.
        let s = format!("{}é", "b".repeat(TEXT_MAX - 1));
        let t = TextBuf::new(&s);
        assert_eq!(t.len(), TEXT_MAX - 1);
        assert_eq!(t.as_str(), "b".repeat(TEXT_MAX - 1));
        let mut t = TextBuf::new(&"c".repeat(TEXT_MAX - 1));
        assert!(!t.push('é'));
        assert!(t.push('d'));
        assert!(!t.push('e'));
    }

    #[test]
    fn formatting_writes_without_failing() {
        let mut t = TextBuf::EMPTY;
        write!(t, "/fader/{}", 3).unwrap();
        write!(t, "/{:.2}", 0.5f32).unwrap();
        assert_eq!(t, "/fader/3/0.50");
        let mut t = TextBuf::EMPTY;
        write!(t, "{}", "x".repeat(300)).unwrap();
        assert_eq!(t.len(), TEXT_MAX);
    }
}
