//! Recording to disk (ARCHITECTURE §8 "Recording", §12).
//!
//! A [`DiskRecorder`] owns one disk thread, and the disk thread owns every
//! file. `Record` nodes get a [`RecordSink`] per node from
//! [`RecorderFactory::open`]; the sink only pushes into two lock-free rings
//! (commands, and interleaved samples with several seconds of room) and
//! never allocates, blocks, or touches a file. If the disk falls behind and
//! the sample ring fills, the block is dropped and counted.
//!
//! Commands carry the sample-ring position they apply at, so the disk thread
//! can split the sample stream into takes exactly even when it reads samples
//! and commands at different moments. Each take becomes
//! `<project>/assets/<name>-NNN.wav` (the first unused number) and is
//! reported through [`DiskRecorder::finished_takes`] once its file is
//! complete, so the editor can offer to turn it into a clip.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use ac_core::{AudioIn, RecordRequest, RecordSink};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::assets::RecorderFactory;
use crate::wav::BitDepth;

/// Seconds of audio a sink can buffer while the disk is busy.
const RING_SECONDS: usize = 5;
const CMD_RING: usize = 64;
/// How often the disk thread looks for work when idle.
const POLL: Duration = Duration::from_millis(10);
/// Files are numbered `-001` up to this.
const MAX_TAKE_NUMBER: u32 = 99_999;

/// A finished (or failed) take.
#[derive(Clone, Debug, PartialEq)]
pub struct Take {
    /// Where the file is.
    pub path: PathBuf,
    /// The path relative to the project directory (`assets/vox-001.wav`), as
    /// a node config would reference it.
    pub relative_path: String,
    /// The Record node's name and flattened path.
    pub name: String,
    pub node_key: String,
    pub channels: usize,
    pub sample_rate: u32,
    pub frames: u64,
    /// Why the take is incomplete or missing, if it is.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    /// A take begins at this sample-ring position.
    Start(u64),
    /// The take ends at this position.
    Stop(u64),
}

impl Cmd {
    fn at(self) -> u64 {
        match self {
            Cmd::Start(p) | Cmd::Stop(p) => p,
        }
    }
}

/// What `open` sends the disk thread.
struct Registration {
    name: String,
    node_key: String,
    channels: usize,
    sample_rate: u32,
    cmd_rx: Consumer<Cmd>,
    data_rx: Consumer<f32>,
}

struct Shared {
    project_dir: PathBuf,
    depth: BitDepth,
    stop: AtomicBool,
    finished: Mutex<Vec<Take>>,
}

/// Records `Record` nodes' takes into the project's `assets/` folder.
pub struct DiskRecorder {
    shared: Arc<Shared>,
    reg_tx: Mutex<Option<mpsc::Sender<Registration>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for DiskRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskRecorder")
            .field("project_dir", &self.shared.project_dir)
            .field("depth", &self.shared.depth)
            .finish_non_exhaustive()
    }
}

impl DiskRecorder {
    /// A recorder writing 32-bit float WAV files under `project_dir/assets`.
    pub fn new(project_dir: &Path) -> std::io::Result<DiskRecorder> {
        DiskRecorder::with_depth(project_dir, BitDepth::Float32)
    }

    /// A recorder with a chosen sample format.
    pub fn with_depth(project_dir: &Path, depth: BitDepth) -> std::io::Result<DiskRecorder> {
        let shared = Arc::new(Shared {
            project_dir: project_dir.to_path_buf(),
            depth,
            stop: AtomicBool::new(false),
            finished: Mutex::new(Vec::new()),
        });
        let (reg_tx, reg_rx) = mpsc::channel();
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("ac-disk".into())
            .spawn(move || disk_thread(&s, &reg_rx))?;
        Ok(DiskRecorder {
            shared,
            reg_tx: Mutex::new(Some(reg_tx)),
            thread: Mutex::new(Some(thread)),
        })
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.shared.project_dir.join("assets")
    }

    /// Takes completed since the last call, oldest first.
    pub fn finished_takes(&self) -> Vec<Take> {
        match self.shared.finished.lock() {
            Ok(mut v) => std::mem::take(&mut *v),
            Err(p) => std::mem::take(&mut *p.into_inner()),
        }
    }

    /// Drain every ring, finish every open take, and stop the disk thread.
    /// Sinks still in use afterwards record nothing. Idempotent.
    pub fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Ok(mut tx) = self.reg_tx.lock() {
            tx.take();
        }
        let handle = self.thread.lock().ok().and_then(|mut t| t.take());
        if let Some(h) = handle {
            let _ = h.join();
        }
    }
}

impl Drop for DiskRecorder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl RecorderFactory for DiskRecorder {
    fn open(&self, req: &RecordRequest<'_>) -> Option<Box<dyn RecordSink>> {
        let channels = req.channels.clamp(1, 64);
        let rate = req.sample_rate.clamp(1_000, 768_000) as usize;
        let (data_tx, data_rx) = RingBuffer::new(rate * channels * RING_SECONDS);
        let (cmd_tx, cmd_rx) = RingBuffer::new(CMD_RING);
        let reg = Registration {
            name: req.name.to_string(),
            node_key: req.node_key.to_string(),
            channels,
            sample_rate: req.sample_rate,
            cmd_rx,
            data_rx,
        };
        let sent = self
            .reg_tx
            .lock()
            .ok()
            .and_then(|tx| tx.as_ref().map(|tx| tx.send(reg).is_ok()))
            .unwrap_or(false);
        sent.then(|| {
            Box::new(DiskSink {
                channels,
                cmd_tx,
                data_tx,
                pushed: 0,
                state: SinkState::Idle,
                pending_stop: None,
                dropped: 0,
            }) as Box<dyn RecordSink>
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SinkState {
    Idle,
    Recording,
    /// The start command couldn't be queued; the take's samples are dropped.
    Failed,
}

/// The audio-thread end of a recording. Wait-free and allocation-free.
struct DiskSink {
    channels: usize,
    cmd_tx: Producer<Cmd>,
    data_tx: Producer<f32>,
    /// Samples pushed into the data ring so far.
    pushed: u64,
    state: SinkState,
    /// A stop that didn't fit in the command ring yet.
    pending_stop: Option<u64>,
    dropped: u64,
}

impl DiskSink {
    fn flush_pending_stop(&mut self) -> bool {
        if let Some(at) = self.pending_stop {
            if self.cmd_tx.push(Cmd::Stop(at)).is_err() {
                return false;
            }
            self.pending_stop = None;
        }
        true
    }
}

impl RecordSink for DiskSink {
    fn start(&mut self) {
        if self.state != SinkState::Idle {
            return;
        }
        self.state =
            if self.flush_pending_stop() && self.cmd_tx.push(Cmd::Start(self.pushed)).is_ok() {
                SinkState::Recording
            } else {
                SinkState::Failed
            };
    }

    fn write(&mut self, audio: &AudioIn<'_>) {
        let frames = audio.frames();
        let n = frames * self.channels;
        match self.state {
            SinkState::Idle => return,
            SinkState::Failed => {
                self.dropped += n as u64;
                return;
            }
            SinkState::Recording => {}
        }
        let ch = self.channels;
        match self.data_tx.write_chunk_uninit(n) {
            Ok(chunk) => {
                let samples = (0..frames).flat_map(move |i| {
                    (0..ch).map(move |c| {
                        let s = audio.channel(c).get(i).copied().unwrap_or(0.0);
                        if s.is_finite() { s } else { 0.0 }
                    })
                });
                let written = chunk.fill_from_iter(samples);
                self.pushed += written as u64;
            }
            Err(_) => self.dropped += n as u64,
        }
    }

    fn stop(&mut self) {
        let was = self.state;
        self.state = SinkState::Idle;
        if was == SinkState::Recording && self.cmd_tx.push(Cmd::Stop(self.pushed)).is_err() {
            self.pending_stop = Some(self.pushed);
        }
    }

    fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// An open WAV file.
struct OpenTake {
    writer: Option<hound::WavWriter<BufWriter<File>>>,
    take: Take,
    samples: u64,
}

/// The disk thread's view of one sink.
struct Track {
    reg: Registration,
    read_pos: u64,
    cmds: VecDeque<Cmd>,
    open: Option<OpenTake>,
    abandoned_seen: bool,
}

impl Track {
    /// Move everything available to disk. Returns whether there was work.
    fn service(&mut self, shared: &Shared) -> bool {
        // Snapshot the samples first, then the commands: a command pushed
        // before a sample is then always seen with it (see the module docs).
        let mut avail = self.reg.data_rx.slots() as u64;
        let mut busy = avail > 0;
        while let Ok(c) = self.reg.cmd_rx.pop() {
            self.cmds.push_back(c);
            busy = true;
        }
        loop {
            while let Some(&c) = self.cmds.front()
                && c.at() <= self.read_pos
            {
                self.cmds.pop_front();
                match c {
                    Cmd::Start(_) => {
                        self.finish(shared);
                        self.open = Some(open_take(shared, &self.reg));
                    }
                    Cmd::Stop(_) => self.finish(shared),
                }
            }
            if avail == 0 {
                break;
            }
            let until_cmd = self
                .cmds
                .front()
                .map_or(avail, |c| c.at().saturating_sub(self.read_pos));
            let n = avail.min(until_cmd);
            if n == 0 {
                break;
            }
            self.write_samples(n as usize, shared.depth);
            self.read_pos += n;
            avail -= n;
        }
        busy
    }

    fn write_samples(&mut self, n: usize, depth: BitDepth) {
        let Ok(chunk) = self.reg.data_rx.read_chunk(n) else {
            return;
        };
        if let Some(open) = self.open.as_mut() {
            let (a, b) = chunk.as_slices();
            if let Some(w) = open.writer.as_mut() {
                let mut result = Ok(());
                for &s in a.iter().chain(b) {
                    result = match depth {
                        BitDepth::Float32 => w.write_sample(s),
                        BitDepth::Pcm24 => w.write_sample(
                            (s * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32,
                        ),
                        BitDepth::Pcm16 => {
                            w.write_sample((s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16)
                        }
                    };
                    if result.is_err() {
                        break;
                    }
                }
                match result {
                    Ok(()) => open.samples += n as u64,
                    Err(e) => {
                        open.take.error = Some(format!("write failed: {e}"));
                        open.writer = None;
                    }
                }
            }
        }
        chunk.commit_all();
    }

    /// Finalize the open take, if any, and report it.
    fn finish(&mut self, shared: &Shared) {
        let Some(mut open) = self.open.take() else {
            return;
        };
        if let Some(w) = open.writer.take()
            && let Err(e) = w.finalize()
        {
            open.take
                .error
                .get_or_insert(format!("couldn't finish file: {e}"));
        }
        open.take.frames = open.samples / self.reg.channels.max(1) as u64;
        if let Ok(mut v) = shared.finished.lock() {
            v.push(open.take);
        }
    }
}

fn disk_thread(shared: &Shared, reg_rx: &mpsc::Receiver<Registration>) {
    let mut tracks: Vec<Track> = Vec::new();
    loop {
        let stopping = shared.stop.load(Ordering::Acquire);
        while let Ok(reg) = reg_rx.try_recv() {
            tracks.push(Track {
                reg,
                read_pos: 0,
                cmds: VecDeque::new(),
                open: None,
                abandoned_seen: false,
            });
        }
        let mut any_busy = false;
        // A sink whose node is gone: once it has been seen abandoned and a
        // later pass found nothing more, its take (if any) is complete.
        tracks.retain_mut(|t| {
            let abandoned = t.reg.cmd_rx.is_abandoned() && t.reg.data_rx.is_abandoned();
            let busy = t.service(shared);
            any_busy |= busy;
            if t.abandoned_seen && !busy {
                t.finish(shared);
                return false;
            }
            t.abandoned_seen = abandoned;
            true
        });
        if stopping {
            for t in &mut tracks {
                t.service(shared);
                t.finish(shared);
            }
            return;
        }
        if !any_busy {
            std::thread::sleep(POLL);
        }
    }
}

/// Keep letters, digits, `-` and `_`; everything else becomes `-`.
pub fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.trim().chars() {
        let c = if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            ch
        } else {
            '-'
        };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "take".to_string()
    } else {
        out
    }
}

fn open_take(shared: &Shared, reg: &Registration) -> OpenTake {
    let base = sanitize_name(&reg.name);
    let dir = shared.project_dir.join("assets");
    let mut take = Take {
        path: dir.join(format!("{base}-001.wav")),
        relative_path: format!("assets/{base}-001.wav"),
        name: reg.name.clone(),
        node_key: reg.node_key.clone(),
        channels: reg.channels,
        sample_rate: reg.sample_rate,
        frames: 0,
        error: None,
    };
    let spec = hound::WavSpec {
        channels: reg.channels as u16,
        sample_rate: reg.sample_rate,
        bits_per_sample: match shared.depth {
            BitDepth::Pcm16 => 16,
            BitDepth::Pcm24 => 24,
            BitDepth::Float32 => 32,
        },
        sample_format: match shared.depth {
            BitDepth::Float32 => hound::SampleFormat::Float,
            _ => hound::SampleFormat::Int,
        },
    };
    let file = std::fs::create_dir_all(&dir)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            // `create_new` claims the number atomically, so two takes (or two
            // processes) never write the same file.
            for n in 1..=MAX_TAKE_NUMBER {
                let name = format!("{base}-{n:03}.wav");
                let path = dir.join(&name);
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(f) => {
                        take.path = path;
                        take.relative_path = format!("assets/{name}");
                        return Ok(f);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(format!("{}: {e}", path.display())),
                }
            }
            Err(format!("too many takes named {base} in {}", dir.display()))
        });
    let writer = file
        .and_then(|f| hound::WavWriter::new(BufWriter::new(f), spec).map_err(|e| e.to_string()));
    match writer {
        Ok(w) => OpenTake {
            writer: Some(w),
            take,
            samples: 0,
        },
        Err(e) => {
            take.error = Some(format!("couldn't create take: {e}"));
            OpenTake {
                writer: None,
                take,
                samples: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::read_wav;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ac-record-{tag}-{}-{}",
            std::process::id(),
            crate::clock::now_ns()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn request<'a>(name: &'a str, channels: usize) -> RecordRequest<'a> {
        RecordRequest {
            node_key: "n1",
            name,
            channels,
            sample_rate: 48_000,
        }
    }

    /// Feed `frames` of a two-channel ramp through a sink in blocks.
    fn write_ramp(sink: &mut dyn RecordSink, start: usize, frames: usize, block: usize) {
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
            let l: Vec<f32> = (0..n).map(|i| (start + done + i) as f32 * 1e-4).collect();
            let r: Vec<f32> = l.iter().map(|v| -v).collect();
            let bufs = vec![l, r];
            let chans = [0u32, 1];
            sink.write(&AudioIn::new(&bufs, &chans, n));
            done += n;
        }
    }

    fn wait_for_takes(rec: &DiskRecorder, n: usize) -> Vec<Take> {
        let mut takes = Vec::new();
        for _ in 0..500 {
            takes.extend(rec.finished_takes());
            if takes.len() >= n {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        takes
    }

    #[test]
    fn takes_land_in_assets_and_read_back() {
        let dir = temp_dir("basic");
        let rec = DiskRecorder::new(&dir).unwrap();
        let mut sink = rec.open(&request("Lead Vox", 2)).unwrap();
        sink.start();
        write_ramp(sink.as_mut(), 0, 10_000, 256);
        sink.stop();
        // Written between takes: ignored.
        write_ramp(sink.as_mut(), 0, 100, 100);
        sink.start();
        write_ramp(sink.as_mut(), 50_000, 3_000, 128);
        sink.stop();

        let takes = wait_for_takes(&rec, 2);
        assert_eq!(takes.len(), 2, "{takes:?}");
        assert_eq!(takes[0].relative_path, "assets/Lead-Vox-001.wav");
        assert_eq!(takes[1].relative_path, "assets/Lead-Vox-002.wav");
        assert_eq!(takes[0].frames, 10_000);
        assert_eq!(takes[1].frames, 3_000);
        assert!(takes.iter().all(|t| t.error.is_none() && t.channels == 2));

        let a = read_wav(&takes[0].path).unwrap();
        assert_eq!(a.sample_rate, 48_000);
        assert_eq!(a.channels.len(), 2);
        assert_eq!(a.frames(), 10_000);
        for (i, (l, r)) in a.channels[0].iter().zip(&a.channels[1]).enumerate() {
            assert_eq!(*l, i as f32 * 1e-4);
            assert_eq!(*r, -(i as f32 * 1e-4));
        }
        let b = read_wav(&takes[1].path).unwrap();
        assert_eq!(b.channels[0][0], 50_000.0 * 1e-4);
        assert_eq!(sink.dropped(), 0);
        drop(sink);
        rec.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn numbering_skips_existing_files() {
        let dir = temp_dir("numbering");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/take-001.wav"), b"x").unwrap();
        let rec = DiskRecorder::new(&dir).unwrap();
        let mut sink = rec.open(&request("", 1)).unwrap();
        sink.start();
        sink.write(&AudioIn::silent(64));
        sink.stop();
        let takes = wait_for_takes(&rec, 1);
        assert_eq!(takes[0].relative_path, "assets/take-002.wav");
        assert_eq!(takes[0].frames, 64);
        rec.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dropping_a_sink_mid_take_finishes_the_file() {
        let dir = temp_dir("abandon");
        let rec = DiskRecorder::with_depth(&dir, BitDepth::Pcm24).unwrap();
        let mut sink = rec.open(&request("loop", 2)).unwrap();
        sink.start();
        write_ramp(sink.as_mut(), 0, 4_800, 480);
        drop(sink);
        let takes = wait_for_takes(&rec, 1);
        assert_eq!(takes.len(), 1);
        assert_eq!(takes[0].frames, 4_800);
        let back = read_wav(&takes[0].path).unwrap();
        assert_eq!(back.frames(), 4_800);
        assert!((back.channels[0][4_000] - 0.4).abs() < 1e-6);
        rec.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shutdown_finishes_open_takes() {
        let dir = temp_dir("shutdown");
        let rec = DiskRecorder::new(&dir).unwrap();
        let mut sink = rec.open(&request("x", 2)).unwrap();
        sink.start();
        write_ramp(sink.as_mut(), 0, 1_000, 100);
        rec.shutdown();
        let takes = rec.finished_takes();
        assert_eq!(takes.len(), 1);
        assert_eq!(takes[0].frames, 1_000);
        // After shutdown, sinks keep working but go nowhere, and new ones
        // aren't offered.
        write_ramp(sink.as_mut(), 0, 100, 100);
        sink.stop();
        assert!(rec.open(&request("y", 1)).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_full_ring_drops_and_counts() {
        let dir = temp_dir("full");
        let rec = DiskRecorder::new(&dir).unwrap();
        // Stop the disk thread so nothing drains, then overfill.
        let mut sink = rec.open(&request("x", 1)).unwrap();
        rec.shutdown();
        sink.start();
        let block = 4_800;
        let blocks = 48_000 * RING_SECONDS / block + 3;
        for _ in 0..blocks {
            sink.write(&AudioIn::silent(block));
        }
        assert_eq!(sink.dropped(), 3 * block as u64);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unwritable_project_dir_reports_an_error() {
        let dir = temp_dir("unwritable");
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let rec = DiskRecorder::new(&file).unwrap();
        let mut sink = rec.open(&request("x", 1)).unwrap();
        sink.start();
        sink.write(&AudioIn::silent(32));
        sink.stop();
        let takes = wait_for_takes(&rec, 1);
        assert_eq!(takes.len(), 1);
        assert!(takes[0].error.is_some(), "{takes:?}");
        rec.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("Lead Vox"), "Lead-Vox");
        assert_eq!(sanitize_name("  a/b\\c:d  "), "a-b-c-d");
        assert_eq!(sanitize_name("../.."), "take");
        assert_eq!(sanitize_name(""), "take");
        assert_eq!(sanitize_name("drums_1"), "drums_1");
    }
}
