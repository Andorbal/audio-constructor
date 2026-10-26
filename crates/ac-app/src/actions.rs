//! Every editor command, with its label and shortcut. Keyboard handling and
//! the menu bar both read this table, so every shortcut has a menu entry
//! and every menu entry shows its shortcut (UX.md principle 3).

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::i18n::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    New,
    Open,
    /// Import a block, a pack, or a shared library from a link.
    ImportFromLink,
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
    /// Turn wires into portals, or portals back into wires.
    Portal,

    /// Lay out the selection (or everything) in layers, left to right.
    AutoLayout,
    /// The align and distribute menu, at the pointer.
    ArrangeMenu,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignTop,
    AlignMiddle,
    AlignBottom,
    DistributeHorizontally,
    DistributeVertically,
    /// Wire two selected nodes ("lazy connect").
    LazyConnect,
    Find,

    Mute,
    Collapse,
    HideUnwired,
    NodeHelp,

    FitAll,
    FitSelection,
    ZoomIn,
    ZoomOut,
    /// The whole interface bigger or smaller (egui's zoom factor), or at 100 %.
    UiScaleUp,
    UiScaleDown,
    UiScaleReset,
    ToggleSidePanel,
    ShowTiming,
    Minimap,
    /// No fades, eases, or flowing wires (View ▸ Reduce Motion).
    ReduceMotion,

    Pin,
    TogglePanels,
    PerformanceMode,

    PlayStop,
    /// Stop, even with the transport already stopped: every generator's
    /// `stop` fires (D200).
    Stop,
    /// Stop everything, reset every node, and mute the outputs for a block
    /// (D201). `Escape` twice.
    Panic,
    PlayFromStart,
    Record,
    ToggleLoop,
    Rewind,
    /// The playhead to the song's end.
    GoToEnd,
    /// A marker at the playhead.
    AddMarker,
    /// The playhead to the marker before or after it.
    PreviousMarker,
    NextMarker,
    /// The transport's click on or off (D207).
    Metronome,
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
const SHIFT_ALT: Modifiers = Modifiers {
    alt: true,
    ctrl: false,
    shift: true,
    mac_cmd: false,
    command: false,
};

impl Action {
    pub const ALL: &'static [Action] = &[
        Action::New,
        Action::Open,
        Action::ImportFromLink,
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
        Action::Portal,
        Action::AutoLayout,
        Action::ArrangeMenu,
        Action::AlignLeft,
        Action::AlignCenter,
        Action::AlignRight,
        Action::AlignTop,
        Action::AlignMiddle,
        Action::AlignBottom,
        Action::DistributeHorizontally,
        Action::DistributeVertically,
        Action::LazyConnect,
        Action::Find,
        Action::Mute,
        Action::Collapse,
        Action::HideUnwired,
        Action::NodeHelp,
        Action::FitAll,
        Action::FitSelection,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::UiScaleUp,
        Action::UiScaleDown,
        Action::UiScaleReset,
        Action::ToggleSidePanel,
        Action::ShowTiming,
        Action::Minimap,
        Action::ReduceMotion,
        Action::Pin,
        Action::TogglePanels,
        Action::PerformanceMode,
        Action::PlayStop,
        Action::Stop,
        Action::Panic,
        Action::PlayFromStart,
        Action::Record,
        Action::ToggleLoop,
        Action::Rewind,
        Action::GoToEnd,
        Action::AddMarker,
        Action::PreviousMarker,
        Action::NextMarker,
        Action::Metronome,
        Action::PlayKeys,
        Action::Shortcuts,
        Action::About,
    ];

    /// The command's name, as menus and the shortcut list show it.
    pub fn label(self) -> String {
        match self {
            Action::New => tr("action-new"),
            Action::Open => tr("action-open"),
            Action::ImportFromLink => tr("action-import-from-link"),
            Action::Save => tr("action-save"),
            Action::SaveAs => tr("action-save-as"),
            Action::ExportWav => tr("action-export-wav"),
            Action::ExportPack => tr("action-export-pack"),
            Action::Quit => tr("action-quit"),
            Action::Undo => tr("action-undo"),
            Action::Redo => tr("action-redo"),
            Action::Copy => tr("action-copy"),
            Action::Paste => tr("action-paste"),
            Action::Delete => tr("action-delete"),
            Action::DeleteReconnect => tr("action-delete-reconnect"),
            Action::Duplicate => tr("action-duplicate"),
            Action::DuplicateWithInputs => tr("action-duplicate-with-inputs"),
            Action::SelectAll => tr("action-select-all"),
            Action::SelectNone => tr("action-select-none"),
            Action::SelectLinked => tr("action-select-linked"),
            Action::Grab => tr("action-grab"),
            Action::Rename => tr("action-rename"),
            Action::AddNode => tr("action-add-node"),
            Action::Frame => tr("action-frame"),
            Action::Group => tr("action-group"),
            Action::Ungroup => tr("action-ungroup"),
            Action::EnterLeaveGroup => tr("action-enter-leave-group"),
            Action::Portal => tr("action-portal"),
            Action::AutoLayout => tr("action-auto-layout"),
            Action::ArrangeMenu => tr("action-arrange-menu"),
            Action::AlignLeft => tr("action-align-left"),
            Action::AlignCenter => tr("action-align-center"),
            Action::AlignRight => tr("action-align-right"),
            Action::AlignTop => tr("action-align-top"),
            Action::AlignMiddle => tr("action-align-middle"),
            Action::AlignBottom => tr("action-align-bottom"),
            Action::DistributeHorizontally => tr("action-distribute-horizontally"),
            Action::DistributeVertically => tr("action-distribute-vertically"),
            Action::LazyConnect => tr("action-lazy-connect"),
            Action::Find => tr("action-find"),
            Action::Mute => tr("action-mute"),
            Action::Collapse => tr("action-collapse"),
            Action::HideUnwired => tr("action-hide-unwired"),
            Action::NodeHelp => tr("action-node-help"),
            Action::FitAll => tr("action-fit-all"),
            Action::FitSelection => tr("action-fit-selection"),
            Action::ZoomIn => tr("action-zoom-in"),
            Action::ZoomOut => tr("action-zoom-out"),
            Action::UiScaleUp => tr("action-ui-scale-up"),
            Action::UiScaleDown => tr("action-ui-scale-down"),
            Action::UiScaleReset => tr("action-ui-scale-reset"),
            Action::ToggleSidePanel => tr("action-toggle-side-panel"),
            Action::ShowTiming => tr("action-show-timing"),
            Action::Minimap => tr("action-minimap"),
            Action::ReduceMotion => tr("action-reduce-motion"),
            Action::Pin => tr("action-pin"),
            Action::TogglePanels => tr("action-toggle-panels"),
            Action::PerformanceMode => tr("action-performance-mode"),
            Action::PlayStop => tr("action-play-stop"),
            Action::Stop => tr("action-stop"),
            Action::Panic => tr("action-panic"),
            Action::PlayFromStart => tr("action-play-from-start"),
            Action::Record => tr("action-record"),
            Action::ToggleLoop => tr("action-toggle-loop"),
            Action::Rewind => tr("action-rewind"),
            Action::GoToEnd => tr("action-go-to-end"),
            Action::AddMarker => tr("action-add-marker"),
            Action::PreviousMarker => tr("action-previous-marker"),
            Action::NextMarker => tr("action-next-marker"),
            Action::Metronome => tr("action-metronome"),
            Action::PlayKeys => tr("action-play-keys"),
            Action::Shortcuts => tr("action-shortcuts"),
            Action::About => tr("action-about"),
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
            Action::ImportFromLink => None,
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
            // Tab moves the keyboard between cards (D183).
            Action::EnterLeaveGroup => sc(CMD, Key::Enter),
            Action::Portal => sc(SHIFT, Key::P),
            Action::AutoLayout => sc(CMD, Key::L),
            // `Alt+A` is Select None (D101).
            Action::ArrangeMenu => sc(SHIFT_ALT, Key::A),
            Action::AlignLeft
            | Action::AlignCenter
            | Action::AlignRight
            | Action::AlignTop
            | Action::AlignMiddle
            | Action::AlignBottom
            | Action::DistributeHorizontally
            | Action::DistributeVertically => None,
            Action::LazyConnect => sc(NONE, Key::F),
            Action::Find => sc(CMD, Key::F),
            Action::Mute => sc(NONE, Key::M),
            Action::Collapse => sc(NONE, Key::H),
            Action::HideUnwired => sc(CMD, Key::H),
            Action::NodeHelp => sc(NONE, Key::F1),
            Action::FitAll => sc(NONE, Key::Home),
            Action::FitSelection => sc(NONE, Key::Period),
            Action::ZoomIn => sc(NONE, Key::Plus),
            Action::ZoomOut => sc(NONE, Key::Minus),
            Action::UiScaleUp => sc(CMD, Key::Equals),
            Action::UiScaleDown => sc(CMD, Key::Minus),
            Action::UiScaleReset => sc(CMD, Key::Num0),
            Action::ToggleSidePanel => sc(NONE, Key::N),
            Action::ShowTiming => sc(CMD, Key::T),
            Action::Minimap => sc(SHIFT, Key::M),
            Action::ReduceMotion => None,
            Action::Pin => sc(NONE, Key::P),
            Action::TogglePanels => sc(CMD, Key::Tab),
            Action::PerformanceMode => sc(NONE, Key::F11),
            Action::PlayStop => sc(NONE, Key::Space),
            Action::Stop => None,
            // `Escape` twice, which the editor counts itself: one Escape
            // cancels whatever is under way.
            Action::Panic => None,
            Action::PlayFromStart => sc(SHIFT, Key::Space),
            Action::Record => sc(NONE, Key::R),
            Action::ToggleLoop => None,
            Action::Rewind => None,
            Action::GoToEnd => sc(NONE, Key::End),
            Action::AddMarker => sc(ALT, Key::M),
            Action::PreviousMarker => sc(ALT, Key::ArrowLeft),
            Action::NextMarker => sc(ALT, Key::ArrowRight),
            Action::Metronome => sc(SHIFT, Key::C),
            Action::PlayKeys => sc(CMD, Key::K),
            Action::Shortcuts => sc(CMD, Key::Slash),
            Action::About => None,
        }
    }

    /// The alignment an align or distribute command applies.
    pub fn align(self) -> Option<crate::edit::align::Align> {
        use crate::edit::align::Align;
        Some(match self {
            Action::AlignLeft => Align::Left,
            Action::AlignCenter => Align::Center,
            Action::AlignRight => Align::Right,
            Action::AlignTop => Align::Top,
            Action::AlignMiddle => Align::Middle,
            Action::AlignBottom => Align::Bottom,
            Action::DistributeHorizontally => Align::Horizontally,
            Action::DistributeVertically => Align::Vertically,
            _ => return None,
        })
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
                | Action::ImportFromLink
                | Action::Quit
                | Action::Undo
                | Action::Redo
                | Action::ToggleSidePanel
                | Action::TogglePanels
                | Action::PerformanceMode
                | Action::PlayStop
                | Action::Stop
                | Action::Panic
                | Action::PlayFromStart
                | Action::Record
                | Action::ToggleLoop
                | Action::Rewind
                | Action::GoToEnd
                | Action::AddMarker
                | Action::PreviousMarker
                | Action::NextMarker
                | Action::Metronome
                | Action::PlayKeys
                | Action::UiScaleUp
                | Action::UiScaleDown
                | Action::UiScaleReset
                | Action::ReduceMotion
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
                | Action::UiScaleUp
                | Action::UiScaleDown
                | Action::UiScaleReset
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
        assert_eq!(for_key(Key::P, Modifiers::SHIFT), Some(Action::Portal));
        assert_eq!(for_key(Key::M, Modifiers::NONE), Some(Action::Mute));
        assert_eq!(for_key(Key::M, Modifiers::SHIFT), Some(Action::Minimap));
        assert_eq!(
            for_key(Key::Tab, Modifiers::CTRL),
            Some(Action::TogglePanels)
        );
        // Tab is the canvas's focus; groups open with Ctrl+Enter.
        assert_eq!(for_key(Key::Tab, Modifiers::NONE), None);
        assert_eq!(
            for_key(Key::Enter, Modifiers::CTRL),
            Some(Action::EnterLeaveGroup)
        );
        assert_eq!(for_key(Key::Enter, Modifiers::NONE), None);
        assert_eq!(
            for_key(Key::F11, Modifiers::NONE),
            Some(Action::PerformanceMode)
        );
        // Alt+A stays Select None; the align menu adds Shift (D101).
        assert_eq!(for_key(Key::A, SHIFT_ALT), Some(Action::ArrangeMenu));
        assert_eq!(for_key(Key::L, Modifiers::CTRL), Some(Action::AutoLayout));
        assert_eq!(for_key(Key::L, Modifiers::NONE), Some(Action::SelectLinked));
        assert_eq!(for_key(Key::F, Modifiers::NONE), Some(Action::LazyConnect));
        assert_eq!(for_key(Key::F, Modifiers::CTRL), Some(Action::Find));
    }

    #[test]
    fn every_alignment_has_a_command() {
        for how in crate::edit::align::Align::ALL {
            assert!(
                Action::ALL.iter().any(|a| a.align() == Some(how)),
                "{how:?}"
            );
        }
    }

    #[test]
    fn ui_scale_keys_leave_canvas_zoom_alone() {
        assert_eq!(for_key(Key::Minus, Modifiers::NONE), Some(Action::ZoomOut));
        assert_eq!(
            for_key(Key::Minus, Modifiers::CTRL),
            Some(Action::UiScaleDown)
        );
        assert_eq!(
            for_key(Key::Equals, Modifiers::CTRL),
            Some(Action::UiScaleUp)
        );
        assert_eq!(
            for_key(Key::Num0, Modifiers::CTRL),
            Some(Action::UiScaleReset)
        );
        assert_eq!(for_key(Key::Num0, Modifiers::NONE), None);
        assert_eq!(for_key(Key::F1, Modifiers::NONE), Some(Action::NodeHelp));
    }
}
