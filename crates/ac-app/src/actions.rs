//! Every editor command, with its label and shortcut. Keyboard handling and
//! the menu bar both read this table, so every shortcut has a menu entry
//! and every menu entry shows its shortcut (UX.md principle 3).

use egui::{Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    New,
    Open,
    Save,
    SaveAs,
    ExportWav,
    ExportPack,
    Quit,

    Undo,
    Redo,
    Copy,
    Paste,
    Delete,
    DeleteReconnect,
    Duplicate,
    DuplicateWithInputs,
    SelectAll,
    SelectNone,
    SelectLinked,
    Grab,
    Rename,

    AddNode,
    Frame,
    Group,
    Ungroup,
    EnterLeaveGroup,

    Mute,
    Collapse,
    HideUnwired,
    NodeHelp,

    FitAll,
    FitSelection,
    ZoomIn,
    ZoomOut,
    ToggleSidePanel,
    ShowTiming,

    Pin,
    TogglePanels,
    PerformanceMode,

    PlayStop,
    PlayFromStart,
    Record,
    ToggleLoop,
    Rewind,
    /// Computer keys go to the patch instead of running shortcuts.
    PlayKeys,

    Shortcuts,
    About,
}

const fn sc(modifiers: Modifiers, key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(modifiers, key))
}

const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;
const CMD_SHIFT: Modifiers = Modifiers {
    alt: false,
    ctrl: false,
    shift: true,
    mac_cmd: false,
    command: true,
};
const CMD_ALT: Modifiers = Modifiers {
    alt: true,
    ctrl: false,
    shift: false,
    mac_cmd: false,
    command: true,
};

impl Action {
    pub const ALL: &'static [Action] = &[
        Action::New,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::ExportWav,
        Action::ExportPack,
        Action::Quit,
        Action::Undo,
        Action::Redo,
        Action::Copy,
        Action::Paste,
        Action::Delete,
        Action::DeleteReconnect,
        Action::Duplicate,
        Action::DuplicateWithInputs,
        Action::SelectAll,
        Action::SelectNone,
        Action::SelectLinked,
        Action::Grab,
        Action::Rename,
        Action::AddNode,
        Action::Frame,
        Action::Group,
        Action::Ungroup,
        Action::EnterLeaveGroup,
        Action::Mute,
        Action::Collapse,
        Action::HideUnwired,
        Action::NodeHelp,
        Action::FitAll,
        Action::FitSelection,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ToggleSidePanel,
        Action::ShowTiming,
        Action::Pin,
        Action::TogglePanels,
        Action::PerformanceMode,
        Action::PlayStop,
        Action::PlayFromStart,
        Action::Record,
        Action::ToggleLoop,
        Action::Rewind,
        Action::PlayKeys,
        Action::Shortcuts,
        Action::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::New => "New",
            Action::Open => "Open…",
            Action::Save => "Save",
            Action::SaveAs => "Save As…",
            Action::ExportWav => "Export WAV…",
            Action::ExportPack => "Export Pack…",
            Action::Quit => "Quit",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::Copy => "Copy",
            Action::Paste => "Paste",
            Action::Delete => "Delete",
            Action::DeleteReconnect => "Delete and Reconnect",
            Action::Duplicate => "Duplicate",
            Action::DuplicateWithInputs => "Duplicate with Inputs",
            Action::SelectAll => "Select All",
            Action::SelectNone => "Select None",
            Action::SelectLinked => "Select Linked",
            Action::Grab => "Move",
            Action::Rename => "Rename",
            Action::AddNode => "Add Node…",
            Action::Frame => "Frame Selection",
            Action::Group => "Make Group",
            Action::Ungroup => "Ungroup",
            Action::EnterLeaveGroup => "Enter / Leave Group",
            Action::Mute => "Mute",
            Action::Collapse => "Collapse",
            Action::HideUnwired => "Hide Unwired Sockets",
            Action::NodeHelp => "Node Help",
            Action::FitAll => "Frame All",
            Action::FitSelection => "Frame Selected",
            Action::ZoomIn => "Zoom In",
            Action::ZoomOut => "Zoom Out",
            Action::ToggleSidePanel => "Side Panel",
            Action::ShowTiming => "Node Timing",
            Action::Pin => "Pin to Panel",
            Action::TogglePanels => "Canvas / Panels",
            Action::PerformanceMode => "Performance Mode",
            Action::PlayStop => "Play / Stop",
            Action::PlayFromStart => "Play from Start",
            Action::Record => "Record",
            Action::ToggleLoop => "Loop",
            Action::Rewind => "Go to Start",
            Action::PlayKeys => "Play Keys",
            Action::Shortcuts => "Keyboard Shortcuts",
            Action::About => "About Audio Constructor",
        }
    }

    /// The shortcut, if any (UX.md "Canvas interactions").
    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        match self {
            Action::New => sc(CMD, Key::N),
            Action::Open => sc(CMD, Key::O),
            Action::Save => sc(CMD, Key::S),
            Action::SaveAs => sc(CMD_SHIFT, Key::S),
            Action::ExportWav => sc(CMD, Key::E),
            Action::ExportPack => None,
            Action::Quit => sc(CMD, Key::Q),
            Action::Undo => sc(CMD, Key::Z),
            Action::Redo => sc(CMD_SHIFT, Key::Z),
            Action::Copy => sc(CMD, Key::C),
            Action::Paste => sc(CMD, Key::V),
            Action::Delete => sc(NONE, Key::X),
            Action::DeleteReconnect => sc(CMD, Key::X),
            Action::Duplicate => sc(SHIFT, Key::D),
            Action::DuplicateWithInputs => sc(CMD_SHIFT, Key::D),
            Action::SelectAll => sc(NONE, Key::A),
            Action::SelectNone => sc(ALT, Key::A),
            Action::SelectLinked => sc(NONE, Key::L),
            Action::Grab => sc(NONE, Key::G),
            Action::Rename => sc(NONE, Key::F2),
            Action::AddNode => sc(SHIFT, Key::A),
            Action::Frame => sc(CMD, Key::J),
            Action::Group => sc(CMD, Key::G),
            Action::Ungroup => sc(CMD_ALT, Key::G),
            Action::EnterLeaveGroup => sc(NONE, Key::Tab),
            Action::Mute => sc(NONE, Key::M),
            Action::Collapse => sc(NONE, Key::H),
            Action::HideUnwired => sc(CMD, Key::H),
            Action::NodeHelp => sc(NONE, Key::F1),
            Action::FitAll => sc(NONE, Key::Home),
            Action::FitSelection => sc(NONE, Key::Period),
            Action::ZoomIn => sc(NONE, Key::Plus),
            Action::ZoomOut => sc(NONE, Key::Minus),
            Action::ToggleSidePanel => sc(NONE, Key::N),
            Action::ShowTiming => sc(CMD, Key::T),
            Action::Pin => sc(NONE, Key::P),
            Action::TogglePanels => sc(CMD, Key::Tab),
            Action::PerformanceMode => sc(NONE, Key::F11),
            Action::PlayStop => sc(NONE, Key::Space),
            Action::PlayFromStart => sc(SHIFT, Key::Space),
            Action::Record => sc(NONE, Key::R),
            Action::ToggleLoop => None,
            Action::Rewind => None,
            Action::PlayKeys => sc(CMD, Key::K),
            Action::Shortcuts => sc(CMD, Key::Slash),
            Action::About => None,
        }
    }

    /// Commands that work while the panels are shown instead of the canvas
    /// (the rest act on the graph, which isn't on screen).
    pub fn works_in_panels(self) -> bool {
        matches!(
            self,
            Action::New
                | Action::Open
                | Action::Save
                | Action::SaveAs
                | Action::ExportWav
                | Action::ExportPack
                | Action::Quit
                | Action::Undo
                | Action::Redo
                | Action::ToggleSidePanel
                | Action::TogglePanels
                | Action::PerformanceMode
                | Action::PlayStop
                | Action::PlayFromStart
                | Action::Record
                | Action::ToggleLoop
                | Action::Rewind
                | Action::PlayKeys
                | Action::Shortcuts
                | Action::About
        )
    }

    /// Commands that work while typing in a text field.
    pub fn works_while_typing(self) -> bool {
        matches!(
            self,
            Action::Save
                | Action::SaveAs
                | Action::Open
                | Action::New
                | Action::Quit
                | Action::ExportWav
        )
    }
}

/// Does a key press with these modifiers match the shortcut exactly?
/// (egui's own matching ignores Shift in some cases; editor shortcuts like
/// `A` vs `Shift+A` must not.)
pub fn matches(sc: &KeyboardShortcut, key: Key, m: Modifiers) -> bool {
    sc.logical_key == key
        && sc.modifiers.command == (m.command || m.ctrl)
        && sc.modifiers.shift == m.shift
        && sc.modifiers.alt == m.alt
}

/// The action bound to a key press, if any.
pub fn for_key(key: Key, m: Modifiers) -> Option<Action> {
    Action::ALL
        .iter()
        .copied()
        .find(|a| a.shortcut().is_some_and(|s| matches(&s, key, m)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for a in Action::ALL {
            if let Some(s) = a.shortcut() {
                assert!(
                    seen.insert((
                        s.logical_key,
                        s.modifiers.command,
                        s.modifiers.shift,
                        s.modifiers.alt
                    )),
                    "{a:?} shares a shortcut"
                );
            }
        }
    }

    #[test]
    fn modifiers_must_match_exactly() {
        assert_eq!(for_key(Key::A, Modifiers::NONE), Some(Action::SelectAll));
        assert_eq!(for_key(Key::A, Modifiers::SHIFT), Some(Action::AddNode));
        assert_eq!(for_key(Key::A, Modifiers::ALT), Some(Action::SelectNone));
        assert_eq!(for_key(Key::D, Modifiers::SHIFT), Some(Action::Duplicate));
        assert_eq!(
            for_key(Key::D, CMD_SHIFT),
            Some(Action::DuplicateWithInputs)
        );
        assert_eq!(for_key(Key::G, CMD_ALT), Some(Action::Ungroup));
        assert_eq!(
            for_key(Key::X, Modifiers::CTRL),
            Some(Action::DeleteReconnect)
        );
        assert_eq!(for_key(Key::Q, Modifiers::NONE), None);
        assert_eq!(for_key(Key::P, Modifiers::NONE), Some(Action::Pin));
        assert_eq!(
            for_key(Key::Tab, Modifiers::CTRL),
            Some(Action::TogglePanels)
        );
        assert_eq!(
            for_key(Key::Tab, Modifiers::NONE),
            Some(Action::EnterLeaveGroup)
        );
        assert_eq!(
            for_key(Key::F11, Modifiers::NONE),
            Some(Action::PerformanceMode)
        );
    }
}
