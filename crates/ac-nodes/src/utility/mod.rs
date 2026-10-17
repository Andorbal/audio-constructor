//! Utility nodes: routing, channels, feedback, and canvas helpers.

use ac_core::{Config, ConfigExt, Registry};

pub mod comment;
pub mod feedback;
pub mod group;
pub mod merge_channels;
pub mod reroute;
pub mod split_channels;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<feedback::Feedback>();
    r.register::<reroute::Reroute>();
    r.register::<group::Group>();
    r.register::<split_channels::SplitChannels>();
    r.register::<merge_channels::MergeChannels>();
    r.register::<comment::Comment>();
}

/// Most channels a channel-count config accepts; bounds the sockets and
/// buffers a typo in a project file can ask for.
pub(crate) const MAX_CHANNELS: u64 = 32;

/// The `channels` config, clamped to 1..=[`MAX_CHANNELS`].
pub(crate) fn channel_count(config: &Config, default: u64) -> usize {
    config
        .get_u64_or("channels", default)
        .clamp(1, MAX_CHANNELS) as usize
}

/// Copy `src` into `dst`, zero-filling whatever `src` doesn't cover.
pub(crate) fn copy_into(dst: &mut [f32], src: &[f32]) {
    let (head, tail) = dst.split_at_mut(src.len().min(dst.len()));
    for (d, s) in head.iter_mut().zip(src) {
        *d = *s;
    }
    tail.fill(0.0);
}
