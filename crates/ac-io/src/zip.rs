//! Minimal ZIP archives for project packs (`.acpack`): writing with deflate,
//! reading stored and deflated entries, which is what every common tool
//! produces. Format: PKWARE APPNOTE.TXT 6.3 (local headers, central
//! directory, end-of-central-directory record; no ZIP64, so entries and
//! archives stay under 4 GiB, and no encryption).
//!
//! Entry names are checked on read: absolute paths, drive letters, and `..`
//! are refused, so unpacking can never write outside its folder.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
const STORED: u16 = 0;
const DEFLATE: u16 = 8;
/// Version 2.0: deflate.
const VERSION: u16 = 20;
/// UTF-8 names (general purpose flag bit 11).
const UTF8: u16 = 1 << 11;
/// The largest file an entry may expand to (guards against zip bombs).
pub const MAX_ENTRY_BYTES: u64 = 2 << 30;

/// A file in an archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Forward-slash path inside the archive.
    pub name: String,
    pub data: Vec<u8>,
}

/// Why an archive couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZipError(pub String);

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ZipError {}

fn err(s: impl Into<String>) -> ZipError {
    ZipError(s.into())
}

/// Whether `name` stays inside the folder it's unpacked into.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.starts_with('\\')
        && !name.contains(':')
        && !name.contains('\0')
        && name
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "..")
}

/// Write `entries` as a ZIP archive, deflating each.
pub fn write(entries: &[Entry]) -> Result<Vec<u8>, ZipError> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in entries {
        if !safe_name(&e.name) {
            return Err(err(format!("unsafe entry name {:?}", e.name)));
        }
        let crc = crc32fast::hash(&e.data);
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&e.data)
            .map_err(|x| err(format!("compressing {}: {x}", e.name)))?;
        let deflated = enc
            .finish()
            .map_err(|x| err(format!("compressing {}: {x}", e.name)))?;
        // Keep incompressible data (audio, mostly) stored.
        let (method, body) = if deflated.len() < e.data.len() {
            (DEFLATE, deflated)
        } else {
            (STORED, e.data.clone())
        };
        let (csize, usize_) = (fits(body.len())?, fits(e.data.len())?);
        let offset = fits(out.len())?;
        let name = e.name.as_bytes();
        let name_len = u16::try_from(name.len()).map_err(|_| err("entry name too long"))?;
        // Local file header.
        put32(&mut out, LOCAL_SIG);
        put16(&mut out, VERSION);
        put16(&mut out, UTF8);
        put16(&mut out, method);
        put16(&mut out, 0); // time
        put16(&mut out, 0x21); // date: 1980-01-01
        put32(&mut out, crc);
        put32(&mut out, csize);
        put32(&mut out, usize_);
        put16(&mut out, name_len);
        put16(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(&body);
        // Central directory record.
        put32(&mut central, CENTRAL_SIG);
        put16(&mut central, VERSION);
        put16(&mut central, VERSION);
        put16(&mut central, UTF8);
        put16(&mut central, method);
        put16(&mut central, 0);
        put16(&mut central, 0x21);
        put32(&mut central, crc);
        put32(&mut central, csize);
        put32(&mut central, usize_);
        put16(&mut central, name_len);
        put16(&mut central, 0); // extra
        put16(&mut central, 0); // comment
        put16(&mut central, 0); // disk
        put16(&mut central, 0); // internal attributes
        put32(&mut central, 0); // external attributes
        put32(&mut central, offset);
        central.extend_from_slice(name);
    }
    let count = u16::try_from(entries.len()).map_err(|_| err("too many entries"))?;
    let cd_offset = fits(out.len())?;
    let cd_size = fits(central.len())?;
    out.extend_from_slice(&central);
    put32(&mut out, END_SIG);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, count);
    put16(&mut out, count);
    put32(&mut out, cd_size);
    put32(&mut out, cd_offset);
    put16(&mut out, 0);
    Ok(out)
}

/// Read every file in `bytes` (directories are skipped). Fails on damaged
/// archives, unsupported methods, bad checksums, and unsafe names.
pub fn read(bytes: &[u8]) -> Result<Vec<Entry>, ZipError> {
    // The end record is the last 22 bytes plus a comment of up to 64 KiB.
    let min = bytes.len().saturating_sub(22 + 65_535);
    let end = (min..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&i| get32(bytes, i) == Some(END_SIG))
        .ok_or_else(|| err("not a ZIP archive"))?;
    let count = get16(bytes, end + 10).ok_or_else(|| err("truncated archive"))? as usize;
    let mut at = get32(bytes, end + 16).ok_or_else(|| err("truncated archive"))? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if get32(bytes, at) != Some(CENTRAL_SIG) {
            return Err(err("damaged central directory"));
        }
        let field16 = |o: usize| get16(bytes, at + o).ok_or_else(|| err("truncated archive"));
        let field32 = |o: usize| get32(bytes, at + o).ok_or_else(|| err("truncated archive"));
        let flags = field16(8)?;
        let method = field16(10)?;
        let crc = field32(16)?;
        let csize = field32(20)? as usize;
        let usize_ = u64::from(field32(24)?);
        let name_len = field16(28)? as usize;
        let extra_len = field16(30)? as usize;
        let comment_len = field16(32)? as usize;
        let local = field32(42)? as usize;
        let name_bytes = bytes
            .get(at + 46..at + 46 + name_len)
            .ok_or_else(|| err("truncated archive"))?;
        let name = String::from_utf8_lossy(name_bytes).replace('\\', "/");
        at += 46 + name_len + extra_len + comment_len;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err(err(format!("{name} is encrypted")));
        }
        if !safe_name(&name) {
            return Err(err(format!(
                "refusing entry {name:?}: it would leave the folder"
            )));
        }
        if usize_ > MAX_ENTRY_BYTES {
            return Err(err(format!("{name} is too large")));
        }
        if get32(bytes, local) != Some(LOCAL_SIG) {
            return Err(err(format!("damaged entry {name}")));
        }
        let lname = get16(bytes, local + 26).ok_or_else(|| err("truncated archive"))? as usize;
        let lextra = get16(bytes, local + 28).ok_or_else(|| err("truncated archive"))? as usize;
        let start = local + 30 + lname + lextra;
        let body = bytes
            .get(start..start + csize)
            .ok_or_else(|| err(format!("truncated entry {name}")))?;
        let data = match method {
            STORED => body.to_vec(),
            DEFLATE => {
                let mut d = Vec::with_capacity(usize_.min(64 << 20) as usize);
                DeflateDecoder::new(body)
                    .take(MAX_ENTRY_BYTES + 1)
                    .read_to_end(&mut d)
                    .map_err(|x| err(format!("damaged entry {name}: {x}")))?;
                d
            }
            m => {
                return Err(err(format!(
                    "{name}: compression method {m} isn't supported"
                )));
            }
        };
        if data.len() as u64 != usize_ || crc32fast::hash(&data) != crc {
            return Err(err(format!("{name} is damaged (checksum mismatch)")));
        }
        entries.push(Entry { name, data });
    }
    Ok(entries)
}

fn fits(n: usize) -> Result<u32, ZipError> {
    u32::try_from(n).map_err(|_| err("archive larger than 4 GiB"))
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn get16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn get32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, data: &[u8]) -> Entry {
        Entry {
            name: name.into(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn round_trips_text_and_binary() {
        let text = "{\"a\": 1}\n".repeat(200);
        let noise: Vec<u8> = (0..5000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let entries = vec![
            entry("song.acproj", text.as_bytes()),
            entry("assets/kick.wav", &noise),
            entry("empty.txt", b""),
        ];
        let z = write(&entries).unwrap();
        assert!(z.len() < text.len() + noise.len(), "text is deflated");
        assert_eq!(read(&z).unwrap(), entries);
    }

    #[test]
    fn reads_an_archive_from_another_tool() {
        // Made with Python's zipfile (deflate), with a directory entry.
        let z: &[u8] = include_bytes!("../tests/fixtures/other-tool.zip");
        let entries = read(z).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["hello.txt", "dir/inner.txt"]);
        assert_eq!(entries[0].data, b"hello hello hello hello\n");
        assert_eq!(entries[1].data, b"inner\n");
    }

    #[test]
    fn refuses_names_that_leave_the_folder() {
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "C:/x",
            "\\\\server\\x",
            "a//b",
            "",
        ] {
            assert!(!safe_name(bad), "{bad}");
            assert!(write(&[entry(bad, b"x")]).is_err(), "{bad}");
        }
        for good in ["x", "assets/kick.wav", "a/b/c.txt"] {
            assert!(safe_name(good), "{good}");
        }
        // An archive naming ../x (built by hand) is refused on read.
        let z = write(&[entry("aa/x", b"evil")]).unwrap();
        let mut hacked = z.clone();
        let at: Vec<usize> = (0..z.len().saturating_sub(4))
            .filter(|&i| &z[i..i + 4] == b"aa/x")
            .collect();
        assert_eq!(at.len(), 2, "local and central names");
        for i in at {
            hacked[i..i + 4].copy_from_slice(b"../x");
        }
        assert!(read(&hacked).unwrap_err().0.contains("leave"));
    }

    #[test]
    fn damage_is_reported_not_panicked_on() {
        let z = write(&[entry("a.txt", &b"abc".repeat(100))]).unwrap();
        assert!(read(&z[..z.len() - 3]).is_err());
        assert!(read(b"not a zip").is_err());
        let mut bad = z.clone();
        // Flip a byte in the deflated body.
        bad[40] ^= 0xff;
        assert!(read(&bad).is_err());
        for cut in [0, 10, 30, 60] {
            let _ = read(&z[cut..]);
        }
    }
}
