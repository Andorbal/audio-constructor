//! Offline export: exactly what `audioc render` does with default
//! arguments, so a WAV exported from the app is bit-identical to one
//! rendered on the command line.

use std::path::Path;
use std::sync::Arc;

use ac_core::{Project, Registry, Resources};
use ac_engine::{CompileReport, RenderOptions, render};
use ac_io::{BitDepth, write_wav};

/// What an export produced.
#[derive(Clone, Debug)]
pub struct ExportSummary {
    pub frames: usize,
    pub channels: usize,
    pub sample_rate: u32,
    pub report: CompileReport,
}

/// Render the project over its length at its own rate and block size and
/// write a 24-bit WAV.
pub fn export_wav(
    project: &Project,
    registry: Arc<Registry>,
    resources: Arc<dyn Resources>,
    path: &Path,
) -> Result<ExportSummary, String> {
    let out = render(project, registry, resources, &RenderOptions::default());
    write_wav(path, &out.channels, out.sample_rate, BitDepth::Pcm24).map_err(|e| e.to_string())?;
    Ok(ExportSummary {
        frames: out.frames(),
        channels: out.channels.len(),
        sample_rate: out.sample_rate,
        report: out.report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{Endpoint, NodeId, NodeInstance, Wire};

    fn project() -> Project {
        let mut p = Project {
            name: "export".into(),
            ..Project::default()
        };
        p.settings.length = 0.25;
        // A constant through a gain: a known level from nodes that exist today.
        p.graph.nodes.push(
            NodeInstance::new(NodeId(1), "constant")
                .with_param("value", 0.5)
                .at(0.0, 0.0),
        );
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(2), "gain").with_param("gain", -6.0));
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(3), "audio_out").at(400.0, 0.0));
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in"),
        ));
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(2), "out"),
            Endpoint::new(NodeId(3), "in"),
        ));
        p
    }

    #[test]
    fn export_matches_a_direct_render_bit_for_bit() {
        let dir = std::env::temp_dir().join(format!("ac-app-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let registry = Arc::new(ac_nodes::registry());
        let resources: Arc<dyn Resources> = Arc::new(ac_core::NoResources);
        let p = project();

        let a = dir.join("app.wav");
        let summary = export_wav(&p, registry.clone(), resources.clone(), &a).unwrap();
        assert_eq!(summary.sample_rate, 48_000);
        assert_eq!(summary.frames, 12_000);

        // What `audioc render project -o b.wav` does with default arguments.
        let direct = render(&p, registry, resources, &RenderOptions::default());
        let b = dir.join("cli.wav");
        write_wav(&b, &direct.channels, direct.sample_rate, BitDepth::Pcm24).unwrap();

        let bytes_a = std::fs::read(&a).unwrap();
        let bytes_b = std::fs::read(&b).unwrap();
        assert_eq!(bytes_a, bytes_b);
        // And it isn't silence: 0.5 at -6 dB.
        let back = ac_io::read_wav(&a).unwrap();
        let peak = back.channels[0].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.25).abs() < 0.01, "peak {peak}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
