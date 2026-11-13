//! The Settings screen's rows: which rows exist, what they are called, what
//! they show and how a choice steps. Pure data; no rendering, no App.

use std::path::PathBuf;

use crate::folder_complete;
use crate::tui_config::{CoverProtocol, CoverSize, IconSet, LibraryLayout, TuiConfig};

/// One row of the Settings list. Headings group the rows and cannot be selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Heading(&'static str),
    ShowCover,
    CoverSize,
    CoverProtocol,
    CoverSearch,
    LibraryLayout,
    Marquee,
    Icons,
    DefaultDevice,
    BitPerfect,
    Folder(PathBuf),
    AddFolder,
    RescanAll,
    UpdateCheck,
}

/// How a row is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Switch,
    Choice,
    Action,
    Folder,
    Heading,
}

/// Every row in display order: the headings, then the folders under Library.
pub fn rows(folders: &[PathBuf]) -> Vec<Row> {
    let mut rows = vec![
        Row::Heading("Display"),
        Row::ShowCover,
        Row::CoverSize,
        Row::CoverProtocol,
        Row::CoverSearch,
        Row::LibraryLayout,
        Row::Marquee,
        Row::Icons,
        Row::Heading("Audio"),
        Row::DefaultDevice,
        Row::BitPerfect,
        Row::Heading("Library"),
    ];
    rows.extend(folders.iter().cloned().map(Row::Folder));
    rows.extend([Row::AddFolder, Row::RescanAll]);
    rows.extend([Row::Heading("Updates"), Row::UpdateCheck]);
    rows
}

impl Row {
    pub fn kind(&self) -> Kind {
        match self {
            Row::Heading(_) => Kind::Heading,
            Row::ShowCover
            | Row::CoverSearch
            | Row::Marquee
            | Row::BitPerfect
            | Row::UpdateCheck => Kind::Switch,
            Row::CoverSize | Row::CoverProtocol | Row::LibraryLayout | Row::Icons => Kind::Choice,
            Row::DefaultDevice | Row::AddFolder | Row::RescanAll => Kind::Action,
            Row::Folder(_) => Kind::Folder,
        }
    }

    /// False only for headings.
    pub fn selectable(&self) -> bool {
        self.kind() != Kind::Heading
    }

    /// The row's name. A folder shows its path with the home folder as `~`.
    pub fn label(&self) -> String {
        match self {
            Row::Heading(text) => text.to_string(),
            Row::ShowCover => "Show cover".into(),
            Row::CoverSize => "Cover size".into(),
            Row::CoverProtocol => "Cover protocol".into(),
            Row::CoverSearch => "Cover search for streams".into(),
            Row::LibraryLayout => "Library layout".into(),
            Row::Marquee => "Scroll long titles".into(),
            Row::Icons => "Icons".into(),
            Row::DefaultDevice => "Default output device".into(),
            Row::BitPerfect => "Bit-perfect".into(),
            Row::Folder(path) => match dirs::home_dir() {
                Some(home) => folder_complete::tilde(path, &home),
                None => path.display().to_string(),
            },
            Row::AddFolder => "+ Add a folder…".into(),
            Row::RescanAll => "Rescan all folders".into(),
            Row::UpdateCheck => "Check for updates daily".into(),
        }
    }

    /// The text shown in the explanation strip for the highlighted row.
    pub fn explanation(&self) -> &'static str {
        match self {
            Row::Heading(_) => "",
            Row::ShowCover => "Show album art on the Stage. Off gives the details the whole width.",
            Row::CoverSize => {
                "How big the album art is on the Stage, as a share of the largest size that fits."
            }
            Row::CoverProtocol => {
                "How the art is drawn. Auto asks the terminal; halfblocks works everywhere but looks coarse; off shows the logo."
            }
            Row::CoverSearch => {
                "Look up radio cover art by artist and title on Deezer and MusicBrainz."
            }
            Row::LibraryLayout => {
                "Columns shows artists, albums and tracks side by side (when wide enough); tree nests them in one list."
            }
            Row::Marquee => "Scroll now-playing titles that don't fit. Off cuts them with \"…\".",
            Row::Icons => {
                "Symbols used for icons such as the volume marker. Nerd needs a Nerd Font set as the terminal font (README: Nerd Font)."
            }
            Row::DefaultDevice => {
                "The output Znicz starts with. Choosing one also switches to it now."
            }
            Row::BitPerfect => {
                "Open the device at each file's own sample rate and channels, so samples reach it unchanged."
            }
            Row::Folder(_) => {
                "A folder Znicz indexes. d removes it and its tracks from the library (files are not touched)."
            }
            Row::AddFolder => "Index another folder.",
            Row::RescanAll => "Look for new, changed and deleted files in every folder.",
            Row::UpdateCheck => "Ask GitHub once a day whether a new Znicz is out.",
        }
    }

    /// The `[section] key` the row is saved under, when it has one.
    pub fn config_key(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Row::ShowCover => Some(("tui", "show_cover")),
            Row::CoverSize => Some(("tui", "cover_size")),
            Row::CoverProtocol => Some(("tui", "cover_protocol")),
            Row::CoverSearch => Some(("covers", "search")),
            Row::LibraryLayout => Some(("tui", "library_layout")),
            Row::Marquee => Some(("tui", "marquee")),
            Row::Icons => Some(("tui", "icons")),
            Row::DefaultDevice => Some(("audio", "device")),
            Row::BitPerfect => Some(("audio", "bit_perfect")),
            Row::UpdateCheck => Some(("updates", "check")),
            Row::Heading(_) | Row::Folder(_) | Row::AddFolder | Row::RescanAll => None,
        }
    }
}

/// What the rows display. Borrowed from the App so the model stays pure.
pub struct Values<'a> {
    pub tui: &'a TuiConfig,
    /// Display name of the default output device; `None` means System default.
    pub default_device: Option<&'a str>,
    pub bit_perfect: bool,
    pub update_check: bool,
    /// A protocol change could not take effect until the next start.
    pub protocol_needs_restart: bool,
    /// Bit-perfect was changed this run, so it applies at the next player start.
    pub bit_perfect_changed: bool,
    /// The configured protocol is one the terminal did not report.
    pub protocol_unsupported: bool,
    /// The cover settings the Stage behind the modal still shows; a cover row
    /// whose value differs applies when the modal closes.
    pub stage_shows: Option<&'a TuiConfig>,
}

fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.into()
}

/// The config string for a cover protocol.
pub fn protocol_name(protocol: CoverProtocol) -> &'static str {
    match protocol {
        CoverProtocol::Auto => "auto",
        CoverProtocol::Kitty => "kitty",
        CoverProtocol::Sixel => "sixel",
        CoverProtocol::Halfblocks => "halfblocks",
        CoverProtocol::Off => "off",
    }
}

/// The config string for a library layout.
pub fn layout_name(layout: LibraryLayout) -> &'static str {
    match layout {
        LibraryLayout::Columns => "columns",
        LibraryLayout::Tree => "tree",
    }
}

/// The config string for an icon set.
pub fn icons_name(icons: IconSet) -> &'static str {
    match icons {
        IconSet::Unicode => "unicode",
        IconSet::Emoji => "emoji",
        IconSet::Nerd => "nerd",
    }
}

/// The text shown after the row's label.
pub fn value_text(row: &Row, v: &Values) -> String {
    match row {
        Row::ShowCover => on_off(v.tui.show_cover),
        Row::CoverSize => format!("{}%", v.tui.cover_size.percent()),
        Row::CoverProtocol => protocol_name(v.tui.cover_protocol).into(),
        Row::CoverSearch => on_off(v.tui.cover_search),
        Row::LibraryLayout => layout_name(v.tui.library_layout).into(),
        Row::Marquee => on_off(v.tui.marquee),
        Row::Icons => icons_name(v.tui.icons).into(),
        Row::DefaultDevice => v.default_device.unwrap_or("System default").into(),
        Row::BitPerfect => on_off(v.bit_perfect),
        Row::UpdateCheck => on_off(v.update_check),
        Row::Heading(_) | Row::Folder(_) | Row::AddFolder | Row::RescanAll => String::new(),
    }
}

/// The hint shown at the end of the row, when it has one.
pub fn hint_text(row: &Row, v: &Values) -> Option<&'static str> {
    match row {
        Row::DefaultDevice => Some("↵ choose"),
        Row::BitPerfect => v.bit_perfect_changed.then_some("on restart"),
        Row::CoverProtocol if v.protocol_unsupported => Some("not supported"),
        Row::CoverProtocol if v.protocol_needs_restart => Some("on restart"),
        Row::ShowCover | Row::CoverSize | Row::CoverProtocol | Row::CoverSearch => {
            applies_on_close(row, v).then_some("on close")
        }
        Row::Folder(_) => Some("d remove"),
        Row::AddFolder | Row::RescanAll => Some("↵"),
        _ => None,
    }
}

const PROTOCOLS: [CoverProtocol; 5] = [
    CoverProtocol::Auto,
    CoverProtocol::Kitty,
    CoverProtocol::Sixel,
    CoverProtocol::Halfblocks,
    CoverProtocol::Off,
];
const LAYOUTS: [LibraryLayout; 2] = [LibraryLayout::Columns, LibraryLayout::Tree];
const ICON_SETS: [IconSet; 3] = [IconSet::Unicode, IconSet::Emoji, IconSet::Nerd];

/// The value one step from `current` in `all`. At an end it wraps when `wrap`
/// is set and otherwise stays put.
fn step<T: Copy + PartialEq>(all: &[T], current: T, forward: bool, wrap: bool) -> T {
    let i = all.iter().position(|v| *v == current).unwrap_or(0);
    let last = all.len() - 1;
    let j = match (forward, wrap) {
        (true, _) if i < last => i + 1,
        (true, true) => 0,
        (true, false) => i,
        (false, _) if i > 0 => i - 1,
        (false, true) => last,
        (false, false) => i,
    };
    all[j]
}

/// A cover row whose value the Stage behind the modal does not show yet.
fn applies_on_close(row: &Row, v: &Values) -> bool {
    let Some(shown) = v.stage_shows else {
        return false;
    };
    match row {
        Row::ShowCover => shown.show_cover != v.tui.show_cover,
        Row::CoverSize => shown.cover_size != v.tui.cover_size,
        Row::CoverProtocol => shown.cover_protocol != v.tui.cover_protocol,
        Row::CoverSearch => shown.cover_search != v.tui.cover_search,
        _ => false,
    }
}

/// Which image protocols the terminal reported at start. `None` (no query
/// ran: the config said halfblocks or off) offers every protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProtocolSupport {
    pub kitty: bool,
    pub sixel: bool,
}

/// Whether `protocol` can be offered. Auto, halfblocks and off always can.
pub fn protocol_supported(protocol: CoverProtocol, support: Option<ProtocolSupport>) -> bool {
    match (protocol, support) {
        (_, None) => true,
        (CoverProtocol::Kitty, Some(s)) => s.kitty,
        (CoverProtocol::Sixel, Some(s)) => s.sixel,
        _ => true,
    }
}

/// The protocols the Cover protocol row steps through.
pub fn protocol_choices(support: Option<ProtocolSupport>) -> Vec<CoverProtocol> {
    PROTOCOLS
        .into_iter()
        .filter(|p| protocol_supported(*p, support))
        .collect()
}

/// Step the cover protocol through `choices`. A current value missing from
/// them (set in the file) steps from the first choice.
pub fn step_protocol(
    tui: &mut TuiConfig,
    choices: &[CoverProtocol],
    forward: bool,
    wrap: bool,
) -> bool {
    let new = step(choices, tui.cover_protocol, forward, wrap);
    replace(&mut tui.cover_protocol, new)
}

/// Step a choice row's value in `tui`. Returns whether the value changed.
/// Non-choice rows are left alone and return false.
pub fn step_choice(row: &Row, tui: &mut TuiConfig, forward: bool, wrap: bool) -> bool {
    match row {
        Row::CoverSize => {
            let new = step(&CoverSize::ALL, tui.cover_size, forward, wrap);
            replace(&mut tui.cover_size, new)
        }
        Row::CoverProtocol => {
            let new = step(&PROTOCOLS, tui.cover_protocol, forward, wrap);
            replace(&mut tui.cover_protocol, new)
        }
        Row::LibraryLayout => {
            let new = step(&LAYOUTS, tui.library_layout, forward, wrap);
            replace(&mut tui.library_layout, new)
        }
        Row::Icons => {
            let new = step(&ICON_SETS, tui.icons, forward, wrap);
            replace(&mut tui.icons, new)
        }
        _ => false,
    }
}

fn replace<T: PartialEq>(slot: &mut T, new: T) -> bool {
    let changed = *slot != new;
    *slot = new;
    changed
}

/// The values the TUI shows but does not own. The binary reads them from the
/// config file (or the player) and passes them in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsValues {
    pub bit_perfect: bool,
    pub update_check: bool,
    /// `[audio] device`; `None` is the system default.
    pub default_device: Option<String>,
}

impl Default for SettingsValues {
    fn default() -> Self {
        Self {
            bit_perfect: true,
            update_check: true,
            default_device: None,
        }
    }
}

/// Index of the first selectable row (0 when there is none).
pub fn first_selectable(rows: &[Row]) -> usize {
    rows.iter().position(Row::selectable).unwrap_or(0)
}

/// Index of the last selectable row (0 when there is none).
pub fn last_selectable(rows: &[Row]) -> usize {
    rows.iter().rposition(Row::selectable).unwrap_or(0)
}

/// `index` if that row can be selected, else the nearest selectable row (above
/// first). Used when the rows change under the cursor.
pub fn nearest_selectable(rows: &[Row], index: usize) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let index = index.min(rows.len() - 1);
    if rows[index].selectable() {
        return index;
    }
    let above = next_selectable(rows, index, false);
    if above != index {
        return above;
    }
    next_selectable(rows, index, true)
}

/// The next selectable index from `from` in the given direction, skipping
/// headings. Stays at `from` when no selectable row lies that way.
pub fn next_selectable(rows: &[Row], from: usize, forward: bool) -> usize {
    let found = if forward {
        (from.saturating_add(1)..rows.len()).find(|&i| rows[i].selectable())
    } else {
        (0..from.min(rows.len()))
            .rev()
            .find(|&i| rows[i].selectable())
    };
    found.unwrap_or(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        PathBuf::from(format!("/music/{name}"))
    }

    fn values(tui: &TuiConfig) -> Values<'_> {
        Values {
            tui,
            default_device: None,
            bit_perfect: false,
            update_check: true,
            protocol_needs_restart: false,
            bit_perfect_changed: false,
            protocol_unsupported: false,
            stage_shows: None,
        }
    }

    #[test]
    fn cover_rows_say_on_close_until_the_stage_shows_them() {
        let shown = TuiConfig::default();
        let tui = TuiConfig {
            cover_size: CoverSize::P50,
            ..TuiConfig::default()
        };
        let values = Values {
            tui: &tui,
            default_device: None,
            bit_perfect: true,
            update_check: true,
            protocol_needs_restart: false,
            bit_perfect_changed: false,
            protocol_unsupported: false,
            stage_shows: Some(&shown),
        };
        assert_eq!(hint_text(&Row::CoverSize, &values), Some("on close"));
        assert_eq!(hint_text(&Row::ShowCover, &values), None);
        let no_stage = Values {
            stage_shows: None,
            ..values
        };
        assert_eq!(hint_text(&Row::CoverSize, &no_stage), None);
    }

    #[test]
    fn protocol_choices_follow_what_the_terminal_reported() {
        use CoverProtocol::*;
        assert_eq!(
            protocol_choices(None),
            vec![Auto, Kitty, Sixel, Halfblocks, Off]
        );
        let sixel_only = Some(ProtocolSupport {
            kitty: false,
            sixel: true,
        });
        assert_eq!(
            protocol_choices(sixel_only),
            vec![Auto, Sixel, Halfblocks, Off]
        );
        assert_eq!(
            protocol_choices(Some(ProtocolSupport::default())),
            vec![Auto, Halfblocks, Off]
        );
        assert!(!protocol_supported(Kitty, sixel_only));
        assert!(protocol_supported(Off, sixel_only));
    }

    #[test]
    fn rows_are_grouped_under_headings_in_spec_order() {
        let rows = rows(&[folder("a"), folder("b")]);
        assert_eq!(
            rows,
            vec![
                Row::Heading("Display"),
                Row::ShowCover,
                Row::CoverSize,
                Row::CoverProtocol,
                Row::CoverSearch,
                Row::LibraryLayout,
                Row::Marquee,
                Row::Icons,
                Row::Heading("Audio"),
                Row::DefaultDevice,
                Row::BitPerfect,
                Row::Heading("Library"),
                Row::Folder(folder("a")),
                Row::Folder(folder("b")),
                Row::AddFolder,
                Row::RescanAll,
                Row::Heading("Updates"),
                Row::UpdateCheck,
            ]
        );
    }

    #[test]
    fn headings_are_not_selectable_and_other_rows_are() {
        for row in rows(&[folder("a")]) {
            assert_eq!(row.selectable(), !matches!(row, Row::Heading(_)), "{row:?}");
        }
        assert_eq!(Row::Heading("Audio").kind(), Kind::Heading);
    }

    #[test]
    fn kinds_match_the_spec_table() {
        assert_eq!(Row::ShowCover.kind(), Kind::Switch);
        assert_eq!(Row::BitPerfect.kind(), Kind::Switch);
        assert_eq!(Row::CoverSize.kind(), Kind::Choice);
        assert_eq!(Row::Icons.kind(), Kind::Choice);
        assert_eq!(Row::DefaultDevice.kind(), Kind::Action);
        assert_eq!(Row::AddFolder.kind(), Kind::Action);
        assert_eq!(Row::RescanAll.kind(), Kind::Action);
        assert_eq!(Row::Folder(folder("a")).kind(), Kind::Folder);
    }

    #[test]
    fn explanations_match_the_spec_text() {
        assert_eq!(
            Row::ShowCover.explanation(),
            "Show album art on the Stage. Off gives the details the whole width."
        );
        assert_eq!(
            Row::CoverSize.explanation(),
            "How big the album art is on the Stage, as a share of the largest size that fits."
        );
        assert_eq!(
            Row::BitPerfect.explanation(),
            "Open the device at each file's own sample rate and channels, so samples reach it unchanged."
        );
        assert_eq!(
            Row::Folder(folder("a")).explanation(),
            "A folder Znicz indexes. d removes it and its tracks from the library (files are not touched)."
        );
    }

    #[test]
    fn every_selectable_row_has_an_explanation() {
        for row in rows(&[folder("a")]) {
            if row.selectable() {
                assert!(!row.explanation().is_empty(), "{row:?}");
            }
        }
    }

    #[test]
    fn folder_label_shortens_the_home_folder() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(Row::Folder(home.join("Music")).label(), "~/Music");
            assert_eq!(Row::Folder(home.clone()).label(), "~");
        }
        assert_eq!(Row::Folder(PathBuf::from("/music")).label(), "/music");
        assert_eq!(Row::AddFolder.label(), "+ Add a folder…");
    }

    #[test]
    fn choice_rows_step_and_wrap_when_asked() {
        let mut tui = TuiConfig::default();
        assert!(step_choice(&Row::CoverSize, &mut tui, true, true));
        assert_eq!(tui.cover_size, CoverSize::P75);
        assert!(step_choice(&Row::CoverSize, &mut tui, false, true));
        assert_eq!(tui.cover_size, CoverSize::P100);
        // Wrapping from the first value to the last and back.
        assert!(step_choice(&Row::CoverSize, &mut tui, false, true));
        assert_eq!(tui.cover_size, CoverSize::P25);
        assert!(step_choice(&Row::CoverSize, &mut tui, true, true));
        assert_eq!(tui.cover_size, CoverSize::P100);

        tui.cover_protocol = CoverProtocol::Off;
        assert!(step_choice(&Row::CoverProtocol, &mut tui, true, true));
        assert_eq!(tui.cover_protocol, CoverProtocol::Auto);
        assert!(step_choice(&Row::CoverProtocol, &mut tui, false, true));
        assert_eq!(tui.cover_protocol, CoverProtocol::Off);

        assert!(step_choice(&Row::LibraryLayout, &mut tui, true, true));
        assert_eq!(tui.library_layout, LibraryLayout::Tree);
        assert!(step_choice(&Row::LibraryLayout, &mut tui, true, true));
        assert_eq!(tui.library_layout, LibraryLayout::Columns);

        assert!(step_choice(&Row::Icons, &mut tui, false, true));
        assert_eq!(tui.icons, IconSet::Nerd);
        assert!(step_choice(&Row::Icons, &mut tui, true, true));
        assert_eq!(tui.icons, IconSet::Unicode);
    }

    #[test]
    fn choice_rows_stop_at_the_ends_without_wrap() {
        let mut tui = TuiConfig::default();
        assert!(!step_choice(&Row::CoverSize, &mut tui, false, false));
        assert_eq!(tui.cover_size, CoverSize::P100);
        assert!(step_choice(&Row::CoverSize, &mut tui, true, false));
        assert!(step_choice(&Row::CoverSize, &mut tui, true, false));
        assert!(step_choice(&Row::CoverSize, &mut tui, true, false));
        assert_eq!(tui.cover_size, CoverSize::P25);
        assert!(!step_choice(&Row::CoverSize, &mut tui, true, false));
        assert_eq!(tui.cover_size, CoverSize::P25);

        tui.cover_protocol = CoverProtocol::Auto;
        assert!(!step_choice(&Row::CoverProtocol, &mut tui, false, false));
        assert_eq!(tui.cover_protocol, CoverProtocol::Auto);
        tui.cover_protocol = CoverProtocol::Off;
        assert!(!step_choice(&Row::CoverProtocol, &mut tui, true, false));
        assert_eq!(tui.cover_protocol, CoverProtocol::Off);

        tui.library_layout = LibraryLayout::Columns;
        assert!(!step_choice(&Row::LibraryLayout, &mut tui, false, false));
        assert!(step_choice(&Row::LibraryLayout, &mut tui, true, false));
        assert!(!step_choice(&Row::LibraryLayout, &mut tui, true, false));
        assert_eq!(tui.library_layout, LibraryLayout::Tree);

        tui.icons = IconSet::Unicode;
        assert!(!step_choice(&Row::Icons, &mut tui, false, false));
        assert!(step_choice(&Row::Icons, &mut tui, true, false));
        assert!(step_choice(&Row::Icons, &mut tui, true, false));
        assert!(!step_choice(&Row::Icons, &mut tui, true, false));
        assert_eq!(tui.icons, IconSet::Nerd);
    }

    #[test]
    fn step_choice_ignores_other_rows() {
        let mut tui = TuiConfig::default();
        let before = tui;
        assert!(!step_choice(&Row::ShowCover, &mut tui, true, true));
        assert!(!step_choice(&Row::DefaultDevice, &mut tui, true, true));
        assert_eq!(tui, before);
    }

    #[test]
    fn config_names_match_the_parse_functions() {
        for p in PROTOCOLS {
            assert_eq!(CoverProtocol::parse(protocol_name(p)), p);
        }
        for l in LAYOUTS {
            assert_eq!(LibraryLayout::parse(layout_name(l)), l);
        }
        for i in ICON_SETS {
            assert_eq!(IconSet::parse(icons_name(i)), i);
        }
    }

    #[test]
    fn config_keys_follow_the_spec_table() {
        assert_eq!(Row::CoverSize.config_key(), Some(("tui", "cover_size")));
        assert_eq!(Row::CoverSearch.config_key(), Some(("covers", "search")));
        assert_eq!(Row::BitPerfect.config_key(), Some(("audio", "bit_perfect")));
        assert_eq!(Row::UpdateCheck.config_key(), Some(("updates", "check")));
        assert_eq!(Row::RescanAll.config_key(), None);
        assert_eq!(Row::Folder(folder("a")).config_key(), None);
    }

    #[test]
    fn value_texts_show_the_current_values() {
        let mut tui = TuiConfig::default();
        let v = values(&tui);
        assert_eq!(value_text(&Row::ShowCover, &v), "on");
        assert_eq!(value_text(&Row::CoverSize, &v), "100%");
        assert_eq!(value_text(&Row::CoverProtocol, &v), "auto");
        assert_eq!(value_text(&Row::CoverSearch, &v), "on");
        assert_eq!(value_text(&Row::LibraryLayout, &v), "columns");
        assert_eq!(value_text(&Row::Marquee, &v), "on");
        assert_eq!(value_text(&Row::Icons, &v), "unicode");
        assert_eq!(value_text(&Row::DefaultDevice, &v), "System default");
        assert_eq!(value_text(&Row::BitPerfect, &v), "off");
        assert_eq!(value_text(&Row::UpdateCheck, &v), "on");
        assert_eq!(value_text(&Row::AddFolder, &v), "");
        assert_eq!(value_text(&Row::Folder(folder("a")), &v), "");
        assert_eq!(value_text(&Row::Heading("Audio"), &v), "");

        tui.show_cover = false;
        tui.cover_size = CoverSize::P75;
        tui.cover_protocol = CoverProtocol::Halfblocks;
        tui.cover_search = false;
        tui.library_layout = LibraryLayout::Tree;
        tui.marquee = false;
        tui.icons = IconSet::Nerd;
        let v = Values {
            default_device: Some("Built-in Audio"),
            bit_perfect: true,
            update_check: false,
            ..values(&tui)
        };
        assert_eq!(value_text(&Row::ShowCover, &v), "off");
        assert_eq!(value_text(&Row::CoverSize, &v), "75%");
        assert_eq!(value_text(&Row::CoverProtocol, &v), "halfblocks");
        assert_eq!(value_text(&Row::CoverSearch, &v), "off");
        assert_eq!(value_text(&Row::LibraryLayout, &v), "tree");
        assert_eq!(value_text(&Row::Marquee, &v), "off");
        assert_eq!(value_text(&Row::Icons, &v), "nerd");
        assert_eq!(value_text(&Row::DefaultDevice, &v), "Built-in Audio");
        assert_eq!(value_text(&Row::BitPerfect, &v), "on");
        assert_eq!(value_text(&Row::UpdateCheck, &v), "off");
    }

    #[test]
    fn hints_appear_only_where_the_spec_says() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        assert_eq!(hint_text(&Row::DefaultDevice, &v), Some("↵ choose"));
        assert_eq!(hint_text(&Row::BitPerfect, &v), None);
        assert_eq!(hint_text(&Row::CoverProtocol, &v), None);
        assert_eq!(hint_text(&Row::Folder(folder("a")), &v), Some("d remove"));
        assert_eq!(hint_text(&Row::AddFolder, &v), Some("↵"));
        assert_eq!(hint_text(&Row::RescanAll, &v), Some("↵"));
        assert_eq!(hint_text(&Row::ShowCover, &v), None);
        assert_eq!(hint_text(&Row::Heading("Audio"), &v), None);

        v.bit_perfect_changed = true;
        v.protocol_needs_restart = true;
        assert_eq!(hint_text(&Row::BitPerfect, &v), Some("on restart"));
        assert_eq!(hint_text(&Row::CoverProtocol, &v), Some("on restart"));
    }

    #[test]
    fn next_selectable_skips_headings_at_both_ends() {
        let rows = rows(&[folder("a")]);
        // Index 0 is the "Display" heading, index 1 is Show cover.
        assert_eq!(next_selectable(&rows, 0, true), 1);
        assert_eq!(
            next_selectable(&rows, 1, false),
            1,
            "no row above but a heading"
        );
        // Over the "Audio" heading (index 8) lands on Default output device.
        assert_eq!(next_selectable(&rows, 7, true), 9);
        assert_eq!(next_selectable(&rows, 9, false), 7);
        // The last row is UpdateCheck; going past it stays put.
        let last = rows.len() - 1;
        assert_eq!(next_selectable(&rows, last, true), last);
        // Stepping back from the Updates heading lands on the RescanAll row.
        let updates = last - 1;
        assert_eq!(next_selectable(&rows, updates, false), updates - 1);
        assert_eq!(next_selectable(&rows, updates, true), last);
    }

    #[test]
    fn next_selectable_stays_put_when_nothing_is_selectable() {
        let rows = vec![Row::Heading("Display"), Row::Heading("Audio")];
        assert_eq!(next_selectable(&rows, 0, true), 0);
        assert_eq!(next_selectable(&rows, 1, false), 1);
        assert_eq!(next_selectable(&[], 0, true), 0);
    }

    #[test]
    fn first_and_last_selectable_skip_the_headings() {
        let rows = rows(&[folder("a")]);
        assert_eq!(first_selectable(&rows), 1, "Show cover");
        assert_eq!(last_selectable(&rows), rows.len() - 1, "Check for updates");
        assert_eq!(first_selectable(&[Row::Heading("x")]), 0);
        assert_eq!(last_selectable(&[]), 0);
    }

    #[test]
    fn settings_values_default_to_the_config_defaults() {
        let values = SettingsValues::default();
        assert!(values.bit_perfect);
        assert!(values.update_check);
        assert_eq!(values.default_device, None);
    }
}
