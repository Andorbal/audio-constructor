//! VST3 plugins.

use std::path::Path;

use ac_core::{PluginDesc, PluginProcessor, PluginRequest};

pub(crate) fn describe_bundle(_path: &Path) -> Result<Vec<PluginDesc>, String> {
    Err("VST3 isn't supported yet".into())
}

pub(crate) fn instantiate(
    _path: &Path,
    _req: &PluginRequest<'_>,
    _state: Option<&[u8]>,
) -> Result<Box<dyn PluginProcessor>, String> {
    Err("VST3 isn't supported yet".into())
}
