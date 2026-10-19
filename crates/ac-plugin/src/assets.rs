//! The plugin's audio assets: the bytes of every audio file the project's
//! nodes reference, kept in memory and saved inside the plugin state
//! (DECISIONS D120), so a host session carries its samples with it.
//!
//! Keys are the paths as node configs write them (relative to the folder
//! the project came from, or absolute), so a project loaded from a file
//! and the same project restored from a state resolve alike.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use ac_core::{AudioData, Project, Registry, Resources, referenced_assets, util::content_hash};

/// One audio file.
#[derive(Clone, Debug)]
pub struct Asset {
    /// The file as it was read, saved in the state as is.
    pub bytes: Arc<Vec<u8>>,
    /// [`content_hash`] of `bytes`.
    pub hash: String,
    audio: Option<Arc<AudioData>>,
    /// Why it can't be played, if it can't.
    pub problem: Option<String>,
}

/// Every asset the plugin holds, by config path.
#[derive(Clone, Debug, Default)]
pub struct AssetBank {
    entries: BTreeMap<String, Asset>,
}

impl AssetBank {
    /// Add (or replace) the file at config path `path`. The bytes are kept
    /// even when they don't decode, so a state round-trips what it was
    /// given; the problem is returned for the report.
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        let hash = content_hash(&bytes);
        let (audio, problem) = match decode_wav(&bytes) {
            Ok(a) => (Some(Arc::new(a)), None),
            Err(e) => (None, Some(e)),
        };
        let result = problem.clone().map_or(Ok(()), Err);
        self.entries.insert(
            path.to_string(),
            Asset {
                bytes: Arc::new(bytes),
                hash,
                audio,
                problem,
            },
        );
        result
    }

    pub fn get(&self, path: &str) -> Option<&Asset> {
        self.entries.get(path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Asset)> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forget assets `project` no longer references.
    pub fn retain_used(&mut self, project: &Project, registry: &Registry) {
        let used: Vec<String> = referenced_assets(project, registry)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        self.entries.retain(|k, _| used.contains(k));
    }

    /// Read every asset `project` references from disk, relative to
    /// `base_dir`. Returns a line for the report per file that is missing,
    /// unreadable, or changed since it was assigned (its stored hash).
    pub fn load_from_disk(
        &mut self,
        project: &Project,
        registry: &Registry,
        base_dir: &Path,
    ) -> Vec<String> {
        let mut problems = Vec::new();
        for (path, hash) in referenced_assets(project, registry) {
            let p = Path::new(&path);
            let full = if p.is_absolute() {
                p.to_path_buf()
            } else {
                base_dir.join(p)
            };
            match std::fs::read(&full) {
                Ok(bytes) => {
                    let changed = hash
                        .as_deref()
                        .is_some_and(|h| !h.is_empty() && h != content_hash(&bytes));
                    if changed {
                        problems.push(format!("{path} changed since it was added to the project"));
                    }
                    if let Err(e) = self.insert(&path, bytes) {
                        problems.push(format!("{path} can't be played: {e}"));
                    }
                }
                Err(e) => problems.push(format!("{path} is missing ({e}); it plays as silence")),
            }
        }
        problems
    }

    /// What the engine reads assets through.
    pub fn resources(&self) -> Arc<dyn Resources> {
        Arc::new(BankResources {
            audio: self
                .entries
                .iter()
                .filter_map(|(k, a)| a.audio.clone().map(|d| (k.clone(), d)))
                .collect(),
        })
    }
}

/// A snapshot of the decoded assets for one engine build.
struct BankResources {
    audio: BTreeMap<String, Arc<AudioData>>,
}

impl Resources for BankResources {
    fn audio_asset(&self, path: &str) -> Option<Arc<AudioData>> {
        self.audio.get(path).cloned()
    }
}

/// Decode a WAV file (PCM or float) into non-interleaved channels, as the
/// device layer's reader does.
pub fn decode_wav(bytes: &[u8]) -> Result<AudioData, String> {
    let mut reader =
        hound::WavReader::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let ch = usize::from(spec.channels.max(1));
    let frames = reader.duration() as usize;
    let mut channels: Vec<Vec<f32>> = (0..ch).map(|_| Vec::with_capacity(frames)).collect();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for (i, s) in reader.samples::<f32>().enumerate() {
                channels[i % ch].push(s.map_err(|e| e.to_string())?);
            }
        }
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            for (i, s) in reader.samples::<i32>().enumerate() {
                channels[i % ch].push(s.map_err(|e| e.to_string())? as f32 * scale);
            }
        }
    }
    Ok(AudioData {
        sample_rate: spec.sample_rate,
        channels,
    })
}

#[cfg(test)]
pub(crate) fn wav_bytes(rate: u32, channels: &[Vec<f32>]) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: channels.len() as u16,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cursor, spec).unwrap();
        let frames = channels.first().map_or(0, Vec::len);
        for i in 0..frames {
            for ch in channels {
                w.write_sample(ch[i]).unwrap();
            }
        }
        w.finalize().unwrap();
    }
    cursor.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wavs_decode_and_garbage_is_kept_with_a_problem() {
        let mut bank = AssetBank::default();
        let wav = wav_bytes(48_000, &[vec![0.5, -0.5, 0.25], vec![0.0, 1.0, 0.0]]);
        bank.insert("assets/a.wav", wav.clone()).unwrap();
        let a = bank.resources().audio_asset("assets/a.wav").unwrap();
        assert_eq!(a.sample_rate, 48_000);
        assert_eq!(a.channels, vec![vec![0.5, -0.5, 0.25], vec![0.0, 1.0, 0.0]]);
        assert_eq!(bank.get("assets/a.wav").unwrap().hash, content_hash(&wav));
        let err = bank.insert("b.wav", b"not a wav".to_vec());
        assert!(err.is_err());
        assert_eq!(bank.len(), 2, "kept, so the state round-trips it");
        assert!(bank.resources().audio_asset("b.wav").is_none());
    }
}
