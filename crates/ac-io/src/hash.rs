//! Content hashes for files that travel: block files and their previews in
//! a library index, and downloads checked against one (DECISIONS D191).
//! Assets inside projects keep their own FNV hashes (`ac_core::util`),
//! which only tell files apart; these have to stand up to someone else's
//! server.

use sha2::{Digest, Sha256};

/// SHA-256 of `bytes` as 64 lowercase hex digits.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        out.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(b & 15), 16).unwrap_or('0'));
    }
    out
}

/// Whether `s` looks like a SHA-256 written by [`sha256_hex`] (any case).
pub fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `bytes` hash to `expected` (compared without regard to case).
pub fn matches_sha256(bytes: &[u8], expected: &str) -> bool {
    sha256_hex(bytes).eq_ignore_ascii_case(expected.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // FIPS 180-2, appendix B.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn checks_ignore_case_and_reject_other_text() {
        let h = sha256_hex(b"abc");
        assert!(is_sha256(&h));
        assert!(matches_sha256(b"abc", &h.to_uppercase()));
        assert!(!matches_sha256(b"abd", &h));
        assert!(!is_sha256("abc"));
        assert!(!is_sha256(&"g".repeat(64)));
    }
}
