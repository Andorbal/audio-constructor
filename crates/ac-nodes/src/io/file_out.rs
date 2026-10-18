use ac_core::{
    Channels, Config, ConfigExt, FileRequest, Io, Node, NodeMeta, PrepareCtx, ProcessCtx,
    RecordSink, Signature, SocketSpec, node,
};

node! {
    kind: "file_out",
    name: "File Out",
    category: Io,
    keywords: ["export", "bounce", "stem", "render", "file", "save", "wav"],
    /// Renders its input to a file of its own when the project is rendered
    /// offline.
    ///
    /// `path` names the file, relative to the folder the main render goes
    /// to (`drums.wav`, `stems/bass.wav`); `.wav` is added if there's no
    /// extension. The file covers the same span as the render, at the same
    /// sample rate and bit depth. `channels` sets mono (1) or stereo (2, the
    /// default); a mono signal into a stereo File Out is written to both.
    ///
    /// One `audioc render` can produce many files this way: stems, a dry and
    /// a wet version, a click track. While the patch plays live, File Out
    /// does nothing; use `Record` to capture a performance.
    ///
    /// **Try this:** put a File Out after each instrument's last effect,
    /// named after the instrument, and render once to get a set of stems
    /// that line up sample for sample with the full mix.
    pub struct FileOut {
        sink: Option<Box<dyn RecordSink>>,
        writing: bool,
    }
}

const IN: usize = 0;

/// Files are mono or stereo, like `Audio Out`.
const MAX_CHANNELS: u64 = 2;

fn channel_count(config: &Config) -> u32 {
    config.get_u64_or("channels", 2).clamp(1, MAX_CHANNELS) as u32
}

impl Node for FileOut {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        Signature::new().input(
            SocketSpec::audio("in")
                .channels(Channels::Fixed(channel_count(config)))
                .doc("Signal to write."),
        )
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let path = config.get_str_or("path", "");
        let sink = if path.trim().is_empty() {
            None
        } else {
            ctx.resources.file_out(&FileRequest {
                node_key: ctx.node_key,
                name: ctx.name,
                path,
                channels: channel_count(config) as usize,
                sample_rate: ctx.sample_rate.round() as u32,
            })
        };
        FileOut {
            sink,
            writing: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let Some(sink) = self.sink.as_deref_mut() else {
            return;
        };
        match (self.writing, ctx.transport.playing) {
            (false, true) => {
                sink.start();
                self.writing = true;
            }
            (true, false) => {
                sink.stop();
                self.writing = false;
            }
            _ => {}
        }
        if self.writing {
            sink.write(&io.inputs.audio(IN));
        }
    }

    fn reset(&mut self) {}
}

impl Drop for FileOut {
    fn drop(&mut self) {
        if let (true, Some(sink)) = (self.writing, self.sink.as_deref_mut()) {
            sink.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{AudioIn, Resources};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// Per requested file: path, rate, channels written, and whether a take runs.
    type Log = Arc<Mutex<Vec<(String, usize, Vec<Vec<f32>>, bool)>>>;

    #[derive(Default)]
    struct Files(Log);

    struct Sink {
        k: usize,
        log: Log,
    }

    impl RecordSink for Sink {
        fn start(&mut self) {
            self.log.lock().unwrap()[self.k].3 = true;
        }
        fn write(&mut self, audio: &AudioIn<'_>) {
            let mut log = self.log.lock().unwrap();
            let chans = &mut log[self.k].2;
            for (c, ch) in chans.iter_mut().enumerate() {
                ch.extend_from_slice(audio.channel(c));
            }
        }
        fn stop(&mut self) {
            self.log.lock().unwrap()[self.k].3 = false;
        }
    }

    impl Resources for Files {
        fn file_out(&self, req: &FileRequest<'_>) -> Option<Box<dyn RecordSink>> {
            let mut log = self.0.lock().unwrap();
            log.push((
                req.path.to_string(),
                req.sample_rate as usize,
                vec![Vec::new(); req.channels],
                false,
            ));
            Some(Box::new(Sink {
                k: log.len() - 1,
                log: self.0.clone(),
            }))
        }
    }

    #[test]
    fn writes_its_input_while_playing() {
        let files = Files::default();
        let log = files.0.clone();
        let mut h = Harness::with_resources::<FileOut>(
            json!({"path": "drums.wav", "channels": 1}),
            48_000.0,
            64,
            &files,
        );
        h.set_audio("in", vec![(0..128).map(|i| i as f32).collect()]);
        h.stopped();
        h.run(64);
        assert!(log.lock().unwrap()[0].2[0].is_empty(), "not while stopped");
        h.play();
        h.run(64);
        let log = log.lock().unwrap();
        assert_eq!(log[0].0, "drums.wav");
        assert_eq!(log[0].1, 48_000);
        assert_eq!(log[0].2[0], (64..128).map(|i| i as f32).collect::<Vec<_>>());
        assert!(log[0].3, "started");
    }

    #[test]
    fn stops_when_the_transport_stops_or_the_node_goes() {
        let files = Files::default();
        let log = files.0.clone();
        let mut h =
            Harness::with_resources::<FileOut>(json!({"path": "a.wav"}), 48_000.0, 16, &files);
        h.run(16);
        assert!(log.lock().unwrap()[0].3);
        h.stop();
        h.run(16);
        assert!(!log.lock().unwrap()[0].3);
        h.play();
        h.run(16);
        assert!(log.lock().unwrap()[0].3);
        drop(h);
        assert!(!log.lock().unwrap()[0].3);
    }

    #[test]
    fn no_path_asks_for_nothing() {
        let files = Files::default();
        let log = files.0.clone();
        let mut h = Harness::with_resources::<FileOut>(json!({"path": " "}), 48_000.0, 16, &files);
        h.run(16);
        assert!(log.lock().unwrap().is_empty());
        // And no host gives a live patch a file.
        let mut h = Harness::new::<FileOut>(json!({"path": "x.wav"}));
        h.run(16);
    }
}
