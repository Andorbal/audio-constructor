//! The Keyboard node's body: which keys it has and where each one sits, so
//! drawing and hit testing agree.
//!
//! White keys share the width evenly; each black key straddles the line
//! between its two white neighbors, as on a piano. A keyboard that starts
//! or ends on a black key gets half a black key of room at that edge.

use ac_core::{ConfigExt, NodeInstance, Registry};
use egui::{Pos2, Rect, pos2};

/// Width of a black key, as a fraction of a white key's.
const BLACK_WIDTH: f32 = 0.6;
/// Height of a black key, as a fraction of the keyboard's.
const BLACK_HEIGHT: f32 = 0.62;
/// Room per white key on a card that hasn't been sized by hand (canvas
/// units), so long keyboards get a wider card instead of thin keys.
pub const WHITE_KEY_WIDTH: f32 = 12.0;

/// One key of the drawn keyboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    /// Key number from the left: the node's `key_N` input.
    pub index: usize,
    /// MIDI note it plays (before the node's octave shift).
    pub note: u8,
    pub black: bool,
    pub rect: Rect,
}

pub fn is_black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// The input a key holds down.
pub fn key_socket(index: usize) -> String {
    format!("key_{index}")
}

/// The key number of a `key_N` input.
pub fn key_index(socket: &str) -> Option<usize> {
    socket.strip_prefix("key_")?.parse().ok()
}

/// A Keyboard node's lowest note and number of keys, as the node reads its
/// config: `low` clamped to MIDI's range, and one key per hidden `key_N`
/// input of its signature (which already holds the repaired count).
pub fn layout(registry: &Registry, node: &NodeInstance) -> (u8, usize) {
    let low = node.config.get_u64_or("low", 48).min(127) as u8;
    let keys = registry.get(&node.kind).map_or(0, |e| {
        (e.describe)(&node.config)
            .inputs
            .iter()
            .filter(|s| s.hidden && key_index(&s.name).is_some())
            .count()
    });
    (low, keys)
}

fn note_of(low: u8, k: usize) -> u8 {
    (usize::from(low) + k).min(127) as u8
}

/// Width of the keyboard in white keys, including the half black key of
/// room at an edge that ends on a black key.
pub fn width_in_white_keys(low: u8, keys: usize) -> f32 {
    if keys == 0 {
        return 0.0;
    }
    let whites = (0..keys).filter(|k| !is_black(note_of(low, *k))).count();
    let edge = |k: usize| {
        if is_black(note_of(low, k)) {
            BLACK_WIDTH / 2.0
        } else {
            0.0
        }
    };
    whites as f32 + edge(0) + edge(keys - 1)
}

/// Every key laid out in `rect`, white keys first (drawing order).
pub fn keys(low: u8, keys: usize, rect: Rect) -> Vec<Key> {
    let total = width_in_white_keys(low, keys);
    if total <= 0.0 {
        return Vec::new();
    }
    let unit = rect.width() / total;
    let lead = if is_black(low) {
        BLACK_WIDTH / 2.0
    } else {
        0.0
    };
    let any_white = (0..keys).any(|k| !is_black(note_of(low, k)));
    let black_h = if any_white {
        rect.height() * BLACK_HEIGHT
    } else {
        rect.height()
    };
    let x = |units: f32| rect.min.x + units * unit;
    let mut whites = Vec::new();
    let mut blacks = Vec::new();
    // White keys to the left of the current key.
    let mut w = 0.0f32;
    for index in 0..keys {
        let note = note_of(low, index);
        if is_black(note) {
            let c = lead + w;
            blacks.push(Key {
                index,
                note,
                black: true,
                rect: Rect::from_min_max(
                    pos2(x(c - BLACK_WIDTH / 2.0), rect.min.y),
                    pos2(x(c + BLACK_WIDTH / 2.0), rect.min.y + black_h),
                ),
            });
        } else {
            whites.push(Key {
                index,
                note,
                black: false,
                rect: Rect::from_min_max(
                    pos2(x(lead + w), rect.min.y),
                    pos2(x(lead + w + 1.0), rect.max.y),
                ),
            });
            w += 1.0;
        }
    }
    whites.extend(blacks);
    whites
}

/// The key under `p`: black keys sit on top of the white ones.
pub fn key_at(keys: &[Key], p: Pos2) -> Option<usize> {
    keys.iter()
        .filter(|k| k.black)
        .chain(keys.iter().filter(|k| !k.black))
        .find(|k| k.rect.contains(p))
        .map(|k| k.index)
}

/// "C4" for the C keys (middle C, MIDI note 60, is C4), so the keyboard
/// can be read at a glance. `note` may be shifted by the node's octave, so
/// it can fall outside MIDI's range, where there's no name.
pub fn c_label(note: i32) -> Option<String> {
    ((0..=127).contains(&note) && note.rem_euclid(12) == 0)
        .then(|| format!("C{}", note.div_euclid(12) - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;
    use serde_json::json;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), vec2(150.0, 60.0))
    }

    #[test]
    fn two_octaves_from_c_lay_out_like_a_piano() {
        // 25 keys from C3: 15 white keys, each 10 wide in a 150 wide rect.
        assert_eq!(width_in_white_keys(48, 25), 15.0);
        let ks = keys(48, 25, rect());
        assert_eq!(ks.len(), 25);
        let key = |i: usize| *ks.iter().find(|k| k.index == i).unwrap();
        assert!(!key(0).black && key(1).black && !key(2).black);
        assert_eq!(key(0).rect.min.x, 100.0);
        assert_eq!(key(2).rect.min.x, 110.0);
        assert_eq!(key(24).rect.max.x, 250.0);
        // C# straddles the line between C and D, and is shorter.
        assert!((key(1).rect.center().x - 110.0).abs() < 1e-4);
        assert!(key(1).rect.height() < key(0).rect.height());
        // E and F are neighbors with no black key between them.
        assert_eq!(key(4).rect.max.x, key(5).rect.min.x);
        // White keys come first so black keys draw on top.
        assert!(ks[..15].iter().all(|k| !k.black));
    }

    #[test]
    fn hit_testing_prefers_black_keys() {
        let ks = keys(48, 25, rect());
        // Near the top at the C/D line: C#.
        assert_eq!(key_at(&ks, pos2(110.0, 55.0)), Some(1));
        // The same line low down: a white key either side.
        assert_eq!(key_at(&ks, pos2(108.0, 105.0)), Some(0));
        assert_eq!(key_at(&ks, pos2(112.0, 105.0)), Some(2));
        // Between E and F there's no black key, even at the top.
        assert_eq!(key_at(&ks, pos2(129.0, 55.0)), Some(4));
        assert_eq!(key_at(&ks, pos2(131.0, 55.0)), Some(5));
        // Outside the keyboard: nothing.
        assert_eq!(key_at(&ks, pos2(99.0, 80.0)), None);
        assert_eq!(key_at(&ks, pos2(150.0, 111.0)), None);
    }

    #[test]
    fn black_edges_get_room() {
        // C#..A#: starts and ends on a black key.
        let w = width_in_white_keys(49, 10);
        assert!((w - (5.0 + BLACK_WIDTH)).abs() < 1e-6, "{w}");
        let ks = keys(49, 10, rect());
        let first = ks.iter().find(|k| k.index == 0).unwrap();
        assert!(first.black);
        assert!((first.rect.min.x - 100.0).abs() < 1e-3);
        let last = ks.iter().find(|k| k.index == 9).unwrap();
        assert!((last.rect.max.x - 250.0).abs() < 1e-3);
        // A lone black key fills the whole height.
        let one = keys(61, 1, rect());
        assert_eq!(one[0].rect.height(), 60.0);
        assert_eq!(key_at(&one, pos2(170.0, 105.0)), Some(0));
        assert!(keys(48, 0, rect()).is_empty());
    }

    #[test]
    fn layout_follows_the_nodes_config() {
        let r = ac_nodes::registry();
        let node = |config| {
            let mut n = NodeInstance::new(ac_core::NodeId(1), "keyboard");
            n.config = config;
            n
        };
        assert_eq!(layout(&r, &node(json!({}))), (48, 25));
        assert_eq!(layout(&r, &node(json!({"keys": 200}))), (48, 80));
        assert_eq!(layout(&r, &node(json!({"low": 120}))), (120, 8));
        assert_eq!(layout(&r, &node(json!({"low": 21, "keys": 88}))), (21, 88));
    }

    #[test]
    fn names_and_sockets() {
        assert_eq!(c_label(60).as_deref(), Some("C4"));
        assert_eq!(c_label(0).as_deref(), Some("C-1"));
        assert_eq!(c_label(61), None);
        assert_eq!(c_label(-12), None);
        assert_eq!(c_label(132), None);
        assert_eq!(key_socket(12), "key_12");
        assert_eq!(key_index("key_12"), Some(12));
        assert_eq!(key_index("velocity"), None);
    }
}
