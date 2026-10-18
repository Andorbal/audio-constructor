//! The optional audio backends, run for real where CI can: JACK against the
//! JACK server's dummy driver on Linux, and the ASIO host on Windows (which
//! has no ASIO drivers on CI, so only its listing and error paths run).
//!
//! JACK: `jackd -d dummy -r 48000 -p 256 &` then
//! `cargo test -p ac-io --features jack --test backends -- --ignored`.

#![cfg(any(feature = "jack", all(windows, feature = "asio")))]

#[cfg(feature = "jack")]
mod jack {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ac_core::{Endpoint, NodeId, NodeInstance, Project, Wire};
    use ac_engine::{Engine, EngineOptions};
    use ac_io::{AudioConfig, AudioHost, CpalAudio, InputChoice};

    fn tone() -> Project {
        let mut p = Project::default();
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "oscillator").with_param("freq", 440.0));
        p.graph.nodes.push(NodeInstance::new(NodeId(2), "audio_out"));
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in"),
        ));
        p
    }

    #[test]
    #[ignore = "needs a running JACK server (CI starts the dummy driver)"]
    fn jack_is_listed_and_runs_the_engine() {
        let hosts = ac_io::enumerate();
        let jack = hosts
            .iter()
            .find(|h| h.name == "JACK")
            .unwrap_or_else(|| panic!("JACK isn't among the hosts: {hosts:?}"));
        assert!(!jack.outputs.is_empty(), "JACK lists no outputs: {jack:?}");

        let host = CpalAudio::open(&AudioConfig {
            host: Some("jack".into()),
            input: InputChoice::Default,
            ..AudioConfig::default()
        })
        .expect("JACK opens");
        let info = host.info().clone();
        assert_eq!(info.host, "JACK");
        // JACK runs at the server's rate and period.
        assert_eq!(info.sample_rate, 48_000, "{info:?}");

        let (engine, _) = Engine::new(
            tone(),
            Arc::new(ac_nodes::registry()),
            Arc::new(ac_core::NoResources),
            EngineOptions {
                sample_rate: f64::from(info.sample_rate),
                max_block: info.max_block(),
                ..EngineOptions::default()
            },
        );
        let (mut handle, processor) = engine.split();
        let stream = Box::new(host).start(processor, None).expect("stream starts");
        handle.play();
        let t0 = Instant::now();
        while handle.position() < 24_000 && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
            handle.collect_garbage();
        }
        let stats = stream.stats();
        assert!(
            handle.position() >= 24_000,
            "half a second of audio in {:?}: {stats:?}",
            t0.elapsed()
        );
        assert!(stats.callbacks > 0 && stats.device_errors == 0, "{stats:?}");
        assert!(stream.stop().is_some(), "the processor comes back");
    }
}

#[cfg(all(windows, feature = "asio"))]
mod asio {
    use ac_io::{AudioConfig, CpalAudio};

    #[test]
    fn the_asio_host_is_built_in_and_fails_cleanly_without_drivers() {
        assert!(
            ac_io::host_names().iter().any(|h| h == "ASIO"),
            "{:?}",
            ac_io::host_names()
        );
        // Listing works with or without installed drivers.
        let hosts = ac_io::enumerate();
        assert!(hosts.iter().any(|h| h.name == "ASIO"));
        // Opening either works (a driver is installed) or explains why not.
        match CpalAudio::open(&AudioConfig {
            host: Some("ASIO".into()),
            ..AudioConfig::default()
        }) {
            Ok(a) => assert!(ac_io::AudioHost::info(&a).sample_rate > 0),
            Err(e) => assert!(!e.to_string().is_empty()),
        }
    }
}
