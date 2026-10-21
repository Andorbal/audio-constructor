//! WAV reading and writing (16/24-bit PCM, 32-bit float).

use std::fmt;
use std::path::Path;

use ac_core::AudioData;

/// Output sample format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BitDepth {
    Pcm16,
    #[default]
    Pcm24,
    Float32,
}

impl BitDepth {
    pub fn parse(s: &str) -> Option<BitDepth> {
        match s {
            "16" => Some(BitDepth::Pcm16),
            "24" => Some(BitDepth::Pcm24),
            "32f" | "32" | "float" => Some(BitDepth::Float32),
            _ => None,
        }
    }
}

impl fmt::Display for BitDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BitDepth::Pcm16 => "16",
            BitDepth::Pcm24 => "24",
            BitDepth::Float32 => "32f",
        })
    }
}

/// A WAV file that couldn't be read or written.
#[derive(Debug)]
pub struct WavError(pub String);

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WavError {}

impl From<hound::Error> for WavError {
    fn from(e: hound::Error) -> Self {
        WavError(e.to_string())
    }
}

/// Read a WAV file into non-interleaved `f32` channels.
pub fn read_wav(path: &Path) -> Result<AudioData, WavError> {
    let reader =
        hound::WavReader::open(path).map_err(|e| WavError(format!("{}: {e}", path.display())))?;
    read_from(reader)
}

/// Read WAV data from any reader (tests, embedded files).
pub fn read_wav_bytes(bytes: &[u8]) -> Result<AudioData, WavError> {
    read_from(hound::WavReader::new(std::io::Cursor::new(bytes))?)
}

fn read_from<R: std::io::Read>(mut reader: hound::WavReader<R>) -> Result<AudioData, WavError> {
    let spec = reader.spec();
    let ch = usize::from(spec.channels.max(1));
    let mut channels = vec![Vec::with_capacity(reader.duration() as usize); ch];
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for (i, s) in reader.samples::<f32>().enumerate() {
                channels[i % ch].push(s?);
            }
        }
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            for (i, s) in reader.samples::<i32>().enumerate() {
                channels[i % ch].push(s? as f32 * scale);
            }
        }
    }
    Ok(AudioData {
        sample_rate: spec.sample_rate,
        channels,
    })
}

/// Write non-interleaved channels as a WAV file. PCM output is clipped to
/// ±1 and rounded (no dither, so renders stay bit-reproducible).
pub fn write_wav(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    depth: BitDepth,
) -> Result<(), WavError> {
    let w = hound::WavWriter::create(path, spec(channels, sample_rate, depth))
        .map_err(|e| WavError(format!("{}: {e}", path.display())))?;
    write_samples(w, channels, depth)
}

/// A WAV file of non-interleaved channels, in memory (a host that saves
/// through something other than a path: the web build's downloads).
/// Written exactly as [`write_wav`] writes a file.
pub fn wav_bytes(
    channels: &[Vec<f32>],
    sample_rate: u32,
    depth: BitDepth,
) -> Result<Vec<u8>, WavError> {
    let mut out = std::io::Cursor::new(Vec::new());
    let w = hound::WavWriter::new(&mut out, spec(channels, sample_rate, depth))?;
    write_samples(w, channels, depth)?;
    Ok(out.into_inner())
}

fn spec(channels: &[Vec<f32>], sample_rate: u32, depth: BitDepth) -> hound::WavSpec {
    hound::WavSpec {
        channels: channels.len().max(1) as u16,
        sample_rate,
        bits_per_sample: match depth {
            BitDepth::Pcm16 => 16,
            BitDepth::Pcm24 => 24,
            BitDepth::Float32 => 32,
        },
        sample_format: match depth {
            BitDepth::Float32 => hound::SampleFormat::Float,
            _ => hound::SampleFormat::Int,
        },
    }
}

fn write_samples<W: std::io::Write + std::io::Seek>(
    mut w: hound::WavWriter<W>,
    channels: &[Vec<f32>],
    depth: BitDepth,
) -> Result<(), WavError> {
    let n_ch = channels.len().max(1);
    let frames = channels.first().map_or(0, Vec::len);
    for i in 0..frames {
        for c in 0..n_ch {
            let s = channels
                .get(c)
                .and_then(|ch| ch.get(i))
                .copied()
                .unwrap_or(0.0);
            let s = if s.is_finite() { s } else { 0.0 };
            match depth {
                BitDepth::Float32 => w.write_sample(s)?,
                // Scale by 2^(n-1), matching the reader, and clip the top code.
                BitDepth::Pcm16 => {
                    w.write_sample((s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16)?
                }
                BitDepth::Pcm24 => w.write_sample(
                    (s * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32,
                )?,
            }
        }
    }
    w.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_in_memory_are_the_file() {
        let dir = std::env::temp_dir().join(format!("ac-wav-bytes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chans = vec![
            (0..300)
                .map(|i| (i as f32 * 0.01).sin())
                .collect::<Vec<f32>>(),
        ];
        let p = dir.join("a.wav");
        write_wav(&p, &chans, 48_000, BitDepth::Pcm16).unwrap();
        let bytes = wav_bytes(&chans, 48_000, BitDepth::Pcm16).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
        assert_eq!(read_wav_bytes(&bytes).unwrap().channels[0].len(), 300);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn round_trip_all_depths() {
        let dir = std::env::temp_dir().join(format!("ac-wav-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sig: Vec<f32> = (0..1000).map(|i| ((i as f32) * 0.05).sin() * 0.8).collect();
        let chans = vec![sig.clone(), sig.iter().map(|v| -v).collect()];
        for (depth, tol) in [
            (BitDepth::Pcm16, 0.5 / 32768.0),
            (BitDepth::Pcm24, 0.5 / 8_388_608.0),
            (BitDepth::Float32, 0.0),
        ] {
            let p = dir.join(format!("t{depth}.wav"));
            write_wav(&p, &chans, 44_100, depth).unwrap();
            let back = read_wav(&p).unwrap();
            assert_eq!(back.sample_rate, 44_100);
            assert_eq!(back.channels.len(), 2);
            for (a, b) in back.channels[1].iter().zip(&chans[1]) {
                assert!((a - b).abs() <= tol + 1e-7, "{depth}: {a} vs {b}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bytes_match_the_file() {
        // Not `ac-wav-bytes-…`: that test removes its folder when it ends.
        let dir = std::env::temp_dir().join(format!("ac-wav-bytes24-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chans = vec![vec![0.25f32, -0.5, 1.0], vec![0.0, 0.125, -1.0]];
        let p = dir.join("t.wav");
        write_wav(&p, &chans, 44_100, BitDepth::Pcm24).unwrap();
        let bytes = wav_bytes(&chans, 44_100, BitDepth::Pcm24).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
        assert_eq!(read_wav_bytes(&bytes).unwrap().channels.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(read_wav(Path::new("/definitely/not/here.wav")).is_err());
        assert!(read_wav_bytes(b"garbage").is_err());
    }
}
