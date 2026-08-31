//! The keymap, written down once.
//!
//! The help overlay and the footer hints are both generated from these tables,
//! so the documented keys and the shown keys cannot drift apart.

/// One documented binding.
pub struct Binding {
    pub keys: &'static str,
    pub action: &'static str,
}

const fn b(keys: &'static str, action: &'static str) -> Binding {
    Binding { keys, action }
}

/// Works in every pane.
pub const GLOBAL: &[Binding] = &[
    b("Space", "play / pause"),
    b("s", "stop"),
    b("n", "next track"),
    b("N / p", "previous track"),
    b("P", "playlists"),
    b("→ / l", "seek forward 5s"),
    b("← / h", "seek back 5s"),
    b("L / H", "seek 30s"),
    b("+ / -", "volume up / down"),
    b("m", "mute"),
    b("r", "repeat: off, all, one"),
    b("z", "shuffle"),
    b("]", "open / close queue"),
    b("Tab", "library ↔ queue"),
    b("Alt-← / Alt-→", "pan titles"),
    b("i", "signal inspector"),
    b(",", "devices"),
    b("?", "this help"),
    b("q", "quit"),
];

/// Any list pane.
pub const NAVIGATION: &[Binding] = &[
    b("j / ↓", "down"),
    b("k / ↑", "up"),
    b("Ctrl-d / Ctrl-u", "half page down / up"),
    b("g / G", "first / last row"),
];

pub const QUEUE: &[Binding] = &[
    b("Enter", "play the selected track"),
    b("d / Del", "remove from the queue"),
    b("C", "clear the queue"),
    b("o", "jump to the playing track"),
];

pub const LIBRARY: &[Binding] = &[
    b("/", "search the library"),
    b("Enter", "open album, or play track"),
    b("a", "add selection to the queue"),
    b("A", "add everything listed"),
    b("Esc", "back to the album list"),
    b("R", "reload after a scan"),
];

pub const DEVICES: &[Binding] = &[
    b("Enter", "use this output device"),
    b("R", "rescan devices"),
    b("Esc", "close"),
];

pub const PLAYLISTS: &[Binding] = &[
    b("Enter", "clear the queue and play"),
    b("a", "add to the queue"),
    b("w", "save the queue"),
    b("Esc", "close"),
];

/// Short hints for the footer, per pane.
pub fn hints(pane: &str) -> &'static str {
    match pane {
        "Queue" => {
            "Enter play · d remove · C clear · o jump · Alt-← / Alt-→ pan · ] close · ? help"
        }
        "Library" => {
            "/ search · a add · ] queue · i inspect · P · Alt-← / Alt-→ pan · , devices · ? help"
        }
        "Devices" => "Enter select · R rescan · Esc close · ? help",
        "Inspector" => "i / Esc close · Space pause · ? help",
        "Playlists" => "Enter play · a add · w save · Esc close · ? help",
        _ => "] queue · ? help · q quit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_documents_both_a_key_and_an_action() {
        let tables = [GLOBAL, NAVIGATION, QUEUE, LIBRARY, DEVICES, PLAYLISTS];
        for table in tables {
            for binding in table {
                assert!(!binding.keys.is_empty());
                assert!(!binding.action.is_empty());
            }
        }
    }

    #[test]
    fn each_pane_has_its_own_hint_line() {
        for pane in ["Queue", "Library", "Devices"] {
            assert!(
                hints(pane).contains("?"),
                "{pane} hints should mention help"
            );
        }
        assert!(hints("Queue") != hints("Library"));
        assert!(
            hints("Library").contains("Alt-→"),
            "library hints should show Alt-arrow pan, got {}",
            hints("Library")
        );
        assert!(
            GLOBAL.iter().any(|b| b.keys.contains("Alt-→")),
            "global help should document Alt-arrow pan"
        );
        assert!(
            GLOBAL.iter().any(|b| b.keys == "i"),
            "global help should document the signal inspector"
        );
        assert!(
            GLOBAL.iter().any(|b| b.keys == "P"),
            "global help should document playlists as P, not p"
        );
        assert!(
            GLOBAL
                .iter()
                .any(|b| b.keys.contains("p") && b.action.contains("previous")),
            "global help should document p as previous"
        );
        assert!(
            hints("Library").contains("i inspect"),
            "library hints should mention inspect, got {}",
            hints("Library")
        );
        assert!(
            hints("Library").contains('P'),
            "library hints should mention P, got {}",
            hints("Library")
        );
        assert!(
            GLOBAL
                .iter()
                .all(|b| !b.keys.contains('<') && !b.keys.contains('>')),
            "< and > must not appear in the keymap"
        );
    }
}
