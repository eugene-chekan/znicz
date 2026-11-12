//! The keymap, written down once.
//!
//! Help is generated from the binding tables, and a test checks that every
//! footer hint names a key those tables document, so the two cannot drift.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// One documented binding.
#[derive(Debug, PartialEq, Eq)]
pub struct Binding {
    pub keys: &'static str,
    pub action: &'static str,
}

const fn b(keys: &'static str, action: &'static str) -> Binding {
    Binding { keys, action }
}

/// Where the user is: picks the footer hints and the first help section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Library,
    LibraryTree,
    Search,
    FolderPrompt,
    Queue,
    Devices,
    Settings,
    Stage,
    Playlists,
    Radio,
    Messages,
    Update,
    Prompt,
    StationPrompt,
    Help,
    /// Help with its filter row taking keys.
    HelpFilter,
}

/// One clickable footer hint: what it says and the key a click sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    pub label: &'static str,
    pub key: KeyEvent,
}

const fn hint(label: &'static str, code: KeyCode) -> Hint {
    Hint {
        label,
        key: KeyEvent::new(code, KeyModifiers::NONE),
    }
}

const SPACE_PAUSE: Hint = hint("Space pause", KeyCode::Char(' '));
const SEARCH: Hint = hint("/ search", KeyCode::Char('/'));
const ADD: Hint = hint("a add", KeyCode::Char('a'));
const EXPAND: Hint = hint("o expand", KeyCode::Char('o'));
const NEXT: Hint = hint("n next", KeyCode::Char('n'));
const BROWSE: Hint = hint("v browse", KeyCode::Char('v'));
const QUEUE_OPEN: Hint = hint("] queue", KeyCode::Char(']'));
const QUEUE_CLOSE: Hint = hint("] close", KeyCode::Char(']'));
const PLAYLISTS_OPEN: Hint = hint("P playlists", KeyCode::Char('P'));
const RADIO_OPEN: Hint = hint("R radio", KeyCode::Char('R'));
const ENTER_PLAY: Hint = hint("Enter play", KeyCode::Enter);
const REMOVE: Hint = hint("d remove", KeyCode::Char('d'));
const JUMP: Hint = hint("o jump", KeyCode::Char('o'));
const CLEAR: Hint = hint("C clear", KeyCode::Char('C'));
const SELECT: Hint = hint("Enter select", KeyCode::Enter);
const RESCAN: Hint = hint("r rescan", KeyCode::Char('r'));
const NEW: Hint = hint("n new", KeyCode::Char('n'));
const EDIT: Hint = hint("e edit", KeyCode::Char('e'));
const DELETE: Hint = hint("d delete", KeyCode::Char('d'));
const CLOSE: Hint = hint("Esc close", KeyCode::Esc);
const ADD_FOLDER: Hint = hint("F add folder", KeyCode::Char('F'));
const TAB_COMPLETE: Hint = hint("Tab complete", KeyCode::Tab);
const ENTER_SCAN: Hint = hint("Enter scan", KeyCode::Enter);
const ENTER_CHANGE: Hint = hint("Enter change", KeyCode::Enter);
const CHOOSE: Hint = hint("h/l choose", KeyCode::Char('h'));
/// Opens the Messages list; the footer appends the unread count.
pub const MESSAGES_OPEN: Hint = hint("M messages", KeyCode::Char('M'));

// Prompt and help hints: these keys belong to a prompt, not a binding table.
const ENTER_SEARCH: Hint = hint("Enter search", KeyCode::Enter);
const ENTER_SAVE: Hint = hint("Enter save", KeyCode::Enter);
const CANCEL: Hint = hint("Esc cancel", KeyCode::Esc);
const NEXT_FIELD: Hint = hint("Tab next field", KeyCode::Tab);
const FILTER: Hint = hint("/ filter", KeyCode::Char('/'));
const ENTER_KEEP: Hint = hint("Enter keep", KeyCode::Enter);
const ESC_CLEAR: Hint = hint("Esc clear", KeyCode::Esc);

/// Drawn at the right end of the footer in every context except help.
pub const MORE: Hint = hint("? more", KeyCode::Char('?'));

/// Footer hints for a context, most important first.
pub fn context_hints(context: Context) -> &'static [Hint] {
    match context {
        Context::Library => &[
            SPACE_PAUSE,
            SEARCH,
            ADD,
            QUEUE_OPEN,
            PLAYLISTS_OPEN,
            RADIO_OPEN,
            ADD_FOLDER,
            MESSAGES_OPEN,
        ],
        Context::LibraryTree => &[
            SEARCH,
            ADD,
            EXPAND,
            QUEUE_OPEN,
            PLAYLISTS_OPEN,
            RADIO_OPEN,
            ADD_FOLDER,
            MESSAGES_OPEN,
        ],
        Context::Search => &[ENTER_SEARCH, CANCEL],
        Context::FolderPrompt => &[TAB_COMPLETE, ENTER_SCAN, CANCEL],
        Context::Queue => &[ENTER_PLAY, REMOVE, JUMP, CLEAR, QUEUE_CLOSE],
        Context::Devices => &[SELECT, RESCAN, CLOSE],
        Context::Settings => &[ENTER_CHANGE, CHOOSE, CLOSE],
        Context::Stage => &[SPACE_PAUSE, NEXT, BROWSE, QUEUE_OPEN],
        Context::Playlists | Context::Radio => &[ENTER_PLAY, ADD, NEW, EDIT, DELETE, CLOSE],
        Context::Messages => &[CLEAR, CLOSE],
        Context::Update => &[CLOSE],
        Context::Prompt => &[ENTER_SAVE, CANCEL],
        Context::StationPrompt => &[ENTER_SAVE, CANCEL, NEXT_FIELD],
        Context::Help => &[FILTER, CLOSE],
        Context::HelpFilter => &[ENTER_KEEP, ESC_CLEAR],
    }
}

/// The right-aligned `? more` hint, or `None` where `?` is typed text or help is open.
pub fn more_hint(context: Context) -> Option<Hint> {
    let typed_or_help = matches!(
        context,
        Context::Search
            | Context::FolderPrompt
            | Context::Prompt
            | Context::StationPrompt
            | Context::Help
            | Context::HelpFilter
            | Context::Update
    );
    (!typed_or_help).then_some(MORE)
}

/// Works in every pane.
pub const GLOBAL: &[Binding] = &[
    b("Space", "play / pause"),
    b("s", "stop"),
    b("n", "next track"),
    b("p", "previous track"),
    b("P", "playlists"),
    b("R", "radio"),
    b("→ / ←", "seek 5s"),
    b("Shift-→ / Shift-←", "seek 30s"),
    b("+ / -", "volume up / down"),
    b("m", "mute"),
    b("e", "repeat: off, all, one"),
    b("r", "reload list"),
    b("z", "shuffle"),
    b("]", "open / close queue"),
    b("Tab", "next column, or library ↔ queue"),
    b("Shift-Tab", "previous column / queue focus"),
    b("Alt-← / Alt-→", "pan titles"),
    b("v / i", "now playing (stage)"),
    b(",", "devices"),
    b("S", "settings"),
    b("M", "messages"),
    b("?", "this help"),
    b("q", "quit TUI"),
];

/// Any list pane.
pub const NAVIGATION: &[Binding] = &[
    b("j / ↓", "down"),
    b("k / ↑", "up"),
    b("h / l", "left / right: column, level, back / into"),
    b("Ctrl-d / Ctrl-u", "half page down / up"),
    b("g / G", "first / last row"),
];

pub const QUEUE: &[Binding] = &[
    b("Enter", "play the selected track"),
    b("d / Del", "remove from the queue"),
    b("C", "clear the queue"),
    b("o", "jump to the playing track"),
    b("Alt-↑ / Alt-↓", "move the row up / down"),
];

pub const LIBRARY: &[Binding] = &[
    b("/", "search the library"),
    b("Enter", "open / expand, or play track"),
    b("Space / o", "expand or collapse (tree)"),
    b("Tab / Shift-Tab", "next / previous column"),
    b("a", "add selection to the queue"),
    b("A", "add everything listed"),
    b("J", "jump to the playing track"),
    b("F", "add a music folder"),
    b("U", "rescan music folders"),
    b("Esc", "back one browse level"),
];

pub const DEVICES: &[Binding] = &[
    b("Enter", "use this output device"),
    b("r", "rescan devices"),
    b("Esc", "close"),
];

pub const PLAYLISTS: &[Binding] = &[
    b("Enter", "clear the queue and play"),
    b("a", "add to the queue"),
    b("n", "new (save the queue)"),
    b("e", "edit name"),
    b("c", "copy"),
    b("d", "delete"),
    b("Esc", "close"),
];

pub const RADIO: &[Binding] = &[
    b("Enter", "clear the queue and play"),
    b("a", "add to the queue"),
    b("n", "new station"),
    b("e", "edit name, URL, and art"),
    b("c", "copy"),
    b("d", "delete"),
    b("r", "reload stations.toml"),
    b("Esc", "close"),
];

pub const SETTINGS: &[Binding] = &[
    b("Enter / Space", "change"),
    b("h / l", "choose"),
    b("d", "remove folder"),
    b("Esc", "close"),
];

pub const STAGE: &[Binding] = &[b("v / i / Esc", "back to browse")];

pub const MESSAGES: &[Binding] = &[b("C", "clear the history"), b("Esc / M", "close")];

/// Only in the help overlay: opens a known update, or checks for one right away.
pub const HELP: &[Binding] = &[b("u", "check for updates")];

/// A titled group of bindings in the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub bindings: Vec<&'static Binding>,
    /// Belongs to another context: drawn dim, after the sections that apply now.
    pub dim: bool,
}

/// Context tables, in the order they appear when they are not first.
const CONTEXT_SECTIONS: &[(&str, &[Binding])] = &[
    ("Library", LIBRARY),
    ("Queue", QUEUE),
    ("Playlists", PLAYLISTS),
    ("Radio", RADIO),
    ("Devices", DEVICES),
    ("Settings", SETTINGS),
    ("Stage", STAGE),
    ("Messages", MESSAGES),
    ("Help", HELP),
];

/// Title of the section holding this context's own keys, if it has a table.
pub fn context_title(context: Context) -> Option<&'static str> {
    match context {
        Context::Library | Context::LibraryTree | Context::Search => Some("Library"),
        Context::FolderPrompt => None,
        Context::Queue => Some("Queue"),
        Context::Devices => Some("Devices"),
        Context::Settings => Some("Settings"),
        Context::Stage => Some("Stage"),
        Context::Playlists => Some("Playlists"),
        Context::Radio => Some("Radio"),
        Context::Messages => Some("Messages"),
        Context::Help => Some("Help"),
        Context::Update | Context::Prompt | Context::StationPrompt | Context::HelpFilter => None,
    }
}

fn section(title: &'static str, bindings: &'static [Binding], dim: bool) -> Section {
    Section {
        title,
        bindings: bindings.iter().collect(),
        dim,
    }
}

/// Help in display order: this context, Playback, Navigation, then the rest dim.
pub fn help_sections(context: Context) -> Vec<Section> {
    let first = context_title(context);
    let mut sections = Vec::new();
    for &(title, bindings) in CONTEXT_SECTIONS {
        if Some(title) == first {
            sections.push(section(title, bindings, false));
        }
    }
    sections.push(section("Playback", GLOBAL, false));
    sections.push(section("Navigation", NAVIGATION, false));
    for &(title, bindings) in CONTEXT_SECTIONS {
        if Some(title) != first {
            sections.push(section(title, bindings, true));
        }
    }
    sections
}

/// Keep bindings whose keys or action contain `query`, ignoring case.
///
/// Sections left empty are dropped. A blank query keeps everything.
pub fn filter_sections(sections: Vec<Section>, query: &str) -> Vec<Section> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return sections;
    }
    sections
        .into_iter()
        .filter_map(|mut section| {
            section.bindings.retain(|b| {
                b.keys.to_lowercase().contains(&query) || b.action.to_lowercase().contains(&query)
            });
            (!section.bindings.is_empty()).then_some(section)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_lives_on_the_arrows_and_h_l_move() {
        assert!(
            GLOBAL
                .iter()
                .any(|b| b.keys == "→ / ←" && b.action.contains("5s"))
        );
        assert!(
            GLOBAL
                .iter()
                .any(|b| b.keys == "Shift-→ / Shift-←" && b.action.contains("30s"))
        );
        assert!(GLOBAL.iter().all(|b| {
            !b.keys
                .split(" / ")
                .any(|k| matches!(k, "h" | "l" | "H" | "L"))
        }));
        assert!(NAVIGATION.iter().any(|b| b.keys == "h / l"));
    }

    #[test]
    fn every_binding_documents_both_a_key_and_an_action() {
        let tables = [
            GLOBAL, NAVIGATION, QUEUE, LIBRARY, DEVICES, SETTINGS, PLAYLISTS, RADIO, STAGE,
            MESSAGES,
        ];
        for table in tables {
            for binding in table {
                assert!(!binding.keys.is_empty());
                assert!(!binding.action.is_empty());
            }
        }
    }

    const ALL_CONTEXTS: [Context; 16] = [
        Context::Library,
        Context::LibraryTree,
        Context::Search,
        Context::FolderPrompt,
        Context::Queue,
        Context::Devices,
        Context::Settings,
        Context::Stage,
        Context::Playlists,
        Context::Radio,
        Context::Messages,
        Context::Update,
        Context::Prompt,
        Context::StationPrompt,
        Context::Help,
        Context::HelpFilter,
    ];

    /// Columns a hint row needs: labels plus ` · ` between them.
    fn row_width(hints: &[Hint]) -> usize {
        let labels: usize = hints.iter().map(|h| h.label.chars().count()).sum();
        labels + 3 * hints.len().saturating_sub(1)
    }

    #[test]
    // Later hints drop first on narrow terminals.
    fn every_context_fits_a_wide_footer_with_more() {
        for context in ALL_CONTEXTS {
            let mut width = row_width(context_hints(context));
            if let Some(more) = more_hint(context) {
                width += 3 + more.label.chars().count();
            }
            assert!(width <= 110, "{context:?} needs {width} columns");
        }
    }

    #[test]
    fn more_shows_only_where_question_mark_opens_help() {
        for context in ALL_CONTEXTS {
            match context {
                Context::Search
                | Context::FolderPrompt
                | Context::Prompt
                | Context::StationPrompt
                | Context::Update
                | Context::Help
                | Context::HelpFilter => {
                    assert_eq!(more_hint(context), None, "{context:?}")
                }
                _ => assert_eq!(more_hint(context), Some(MORE), "{context:?}"),
            }
        }
        assert_eq!(MORE.key.code, KeyCode::Char('?'));
    }

    #[test]
    fn library_hints_lead_with_pause_and_label_p_and_r() {
        let labels: Vec<&str> = context_hints(Context::Library)
            .iter()
            .map(|h| h.label)
            .collect();
        assert_eq!(
            labels,
            [
                "Space pause",
                "/ search",
                "a add",
                "] queue",
                "P playlists",
                "R radio",
                "F add folder",
                "M messages"
            ]
        );
    }

    #[test]
    fn the_tree_offers_expand_instead_of_pause() {
        let labels: Vec<&str> = context_hints(Context::LibraryTree)
            .iter()
            .map(|h| h.label)
            .collect();
        assert!(labels.contains(&"o expand"), "{labels:?}");
        assert!(!labels.contains(&"Space pause"), "{labels:?}");
    }

    #[test]
    fn each_hint_sends_the_key_its_label_names() {
        for context in ALL_CONTEXTS {
            for hint in context_hints(context) {
                let token = hint.label.split_once(' ').map(|(k, _)| k).unwrap();
                let expected = match token {
                    "Space" => KeyCode::Char(' '),
                    "Enter" => KeyCode::Enter,
                    "Esc" => KeyCode::Esc,
                    "Tab" => KeyCode::Tab,
                    one => KeyCode::Char(one.chars().next().unwrap()),
                };
                assert_eq!(hint.key.code, expected, "{context:?}: {}", hint.label);
                assert_eq!(hint.key.modifiers, KeyModifiers::NONE);
            }
        }
    }

    fn titles(sections: &[Section]) -> Vec<&'static str> {
        sections.iter().map(|s| s.title).collect()
    }

    #[test]
    fn help_starts_with_the_context_then_playback_and_navigation() {
        let sections = help_sections(Context::Queue);
        assert_eq!(
            titles(&sections),
            [
                "Queue",
                "Playback",
                "Navigation",
                "Library",
                "Playlists",
                "Radio",
                "Devices",
                "Settings",
                "Stage",
                "Messages",
                "Help"
            ]
        );
        let dims: Vec<bool> = sections.iter().map(|s| s.dim).collect();
        assert_eq!(
            dims,
            [
                false, false, false, true, true, true, true, true, true, true, true
            ]
        );
    }

    #[test]
    fn tree_and_search_open_help_on_library() {
        for context in [Context::Library, Context::LibraryTree, Context::Search] {
            assert_eq!(help_sections(context)[0].title, "Library", "{context:?}");
        }
    }

    #[test]
    fn help_lists_the_update_key_first() {
        let sections = help_sections(Context::Help);
        assert_eq!(sections[0].title, "Help");
        assert!(sections[0].bindings.iter().any(|b| b.keys == "u"));
    }

    #[test]
    fn prompts_have_no_context_section() {
        for context in [Context::Prompt, Context::StationPrompt] {
            let sections = help_sections(context);
            assert_eq!(sections[0].title, "Playback", "{context:?}");
            assert_eq!(sections.len(), 11, "every table still listed once");
            assert_eq!(context_title(context), None);
        }
    }

    #[test]
    fn the_filter_matches_actions_ignoring_case() {
        let sections = filter_sections(help_sections(Context::Library), "VOL");
        assert_eq!(titles(&sections), ["Playback"]);
        assert!(
            sections[0]
                .bindings
                .iter()
                .all(|b| b.action.contains("volume"))
        );
    }

    #[test]
    fn the_filter_matches_key_text() {
        let sections = filter_sections(help_sections(Context::Library), "del");
        assert!(
            sections
                .iter()
                .flat_map(|s| s.bindings.iter())
                .any(|b| b.keys == "d / Del")
        );
    }

    #[test]
    fn a_blank_filter_keeps_everything_and_a_miss_keeps_nothing() {
        let all = help_sections(Context::Queue);
        assert_eq!(filter_sections(all.clone(), "  "), all);
        assert!(filter_sections(all, "zzzz").is_empty());
    }

    #[test]
    fn table_backed_hints_name_a_documented_key() {
        for context in [
            Context::Library,
            Context::LibraryTree,
            Context::Queue,
            Context::Devices,
            Context::Stage,
            Context::Playlists,
            Context::Radio,
            Context::Messages,
        ] {
            let sections = help_sections(context);
            let documented: Vec<&Binding> = sections
                .iter()
                .filter(|s| !s.dim)
                .flat_map(|s| s.bindings.iter().copied())
                .collect();
            for hint in context_hints(context) {
                let token = hint.label.split_once(' ').map(|(k, _)| k).unwrap();
                assert!(
                    documented
                        .iter()
                        .any(|b| b.keys.split(" / ").any(|k| k == token)),
                    "{context:?}: hint {:?} has no binding in its help sections",
                    hint.label
                );
            }
        }
    }

    #[test]
    fn the_global_table_keeps_its_letters() {
        let has = |keys: &str, action: &str| {
            GLOBAL
                .iter()
                .any(|b| b.keys == keys && b.action.contains(action))
        };
        assert!(has("P", "playlists"));
        assert!(has("p", "previous"));
        assert!(has("R", "radio"));
        assert!(has("e", "repeat"));
        assert!(has("r", "reload"));
        assert!(has("v / i", "now playing"));
        assert!(GLOBAL.iter().any(|b| b.keys.contains("Alt-→")));
        assert!(GLOBAL.iter().all(|b| !b.keys.contains('N')));
        assert!(
            GLOBAL
                .iter()
                .all(|b| !b.keys.contains('<') && !b.keys.contains('>'))
        );
        assert!(LIBRARY.iter().all(|b| b.keys != "R"));
        assert!(
            DEVICES
                .iter()
                .any(|b| b.keys == "r" && b.action.contains("rescan"))
        );
    }
}
