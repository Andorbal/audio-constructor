//! What the page keeps besides the editor: the link to the worklet, the
//! browser's devices, and results of file pickers and downloads waiting
//! for the next frame. Shared (on the page's one thread) between the
//! frame, the browser's callbacks, and the script-facing handle.

use std::path::PathBuf;

use ac_app::{AudioPurpose, SaveKind};
use ac_core::Project;
use wasm_bindgen::JsValue;

use super::devices::Devices;
use super::shared::JsShared;
use crate::link::PageLink;

/// Something an asynchronous browser call finished, for the next frame.
pub enum Pending {
    /// A project, pack, or block file the person picked.
    Opened { name: String, bytes: Vec<u8> },
    /// A WAV file the person picked.
    Audio {
        purpose: AudioPurpose,
        name: String,
        bytes: Vec<u8>,
    },
    /// A fetched example and its audio files.
    Example {
        path: PathBuf,
        text: String,
        assets: Vec<(String, Vec<u8>)>,
    },
    /// The site's examples, and the one to open first.
    Examples {
        list: Vec<(PathBuf, Project)>,
        open: Option<PathBuf>,
    },
    Saved {
        kind: SaveKind,
        name: String,
        revision: u64,
    },
    /// Not saved: why, or `None` when the person cancelled.
    NotSaved(Option<String>),
    /// Something to tell the person.
    Say { text: String, warning: bool },
}

pub struct PageState {
    pub link: PageLink<JsShared>,
    pub devices: Devices,
    pub pending: Vec<Pending>,
    pub ctx: Option<egui::Context>,
    /// Where the document was opened from or last saved to, for Save.
    pub file_handle: JsValue,
    pub sample_rate: u32,
    /// The document has unsaved changes (leaving the page asks first).
    pub dirty: bool,
    /// The page's title as last set.
    pub title: String,
}

impl PageState {
    pub fn new(link: PageLink<JsShared>, sample_rate: u32) -> PageState {
        PageState {
            link,
            devices: Devices::default(),
            pending: Vec::new(),
            ctx: None,
            file_handle: JsValue::NULL,
            sample_rate,
            dirty: false,
            title: String::new(),
        }
    }

    /// Ask for a frame (after a callback changed something).
    pub fn repaint(&self) {
        if let Some(c) = &self.ctx {
            c.request_repaint();
        }
    }

    /// Hand a result to the next frame.
    pub fn finish(&mut self, p: Pending) {
        self.pending.push(p);
        self.repaint();
    }
}
