//! The Settings screen's rows: which rows exist, what they are called, what
//! they show and how a choice steps. Pure data; no rendering, no App.

use std::path::PathBuf;
use std::time::Duration;

use znicz_core::FadeTimes;

use crate::folder_complete;
use crate::tui_config::{CoverProtocol, CoverSize, IconSet, LibraryLayout, TuiConfig};

/// The Settings tabs, in the order the tab bar shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Display,
    Audio,
    Playback,
    Library,
    General,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Display,
        Tab::Audio,
        Tab::Playback,
        Tab::Library,
        Tab::General,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Display => "Display",
            Tab::Audio => "Audio",
            Tab::Playback => "Playback",
            Tab::Library => "Library",
            Tab::General => "General",
        }
    }

    /// The tab for a number key `'1'` to `'5'`.
    pub fn from_key(c: char) -> Option<Tab> {
        let index = c.to_digit(10)?.checked_sub(1)?;
        Tab::ALL.get(index as usize).copied()
    }

    pub fn index(self) -> usize {
        Tab::ALL
            .iter()
            .position(|tab| *tab == self)
            .unwrap_or_default()
    }

    /// The next tab to the right, wrapping from the last to the first.
    pub fn next(self) -> Tab {
        Tab::ALL[(self.index() + 1) % Tab::ALL.len()]
    }

    /// The next tab to the left, wrapping from the first to the last.
    pub fn previous(self) -> Tab {
        Tab::ALL[(self.index() + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }
}

/// One row of a Settings tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    ShowCover,
    CoverSize,
    CoverProtocol,
    CoverSearch,
    LibraryLayout,
    Marquee,
    Icons,
    DefaultDevice,
    BitPerfect,
    FadePlay,
    FadePause,
    FadeStop,
    FadeTrack,
    FadeMute,
    Crossfade,
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
    Slider(Slider),
    Action,
    Folder,
}

/// The unit a slider's value is written and shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderUnit {
    Ms,
    Seconds,
}

/// A time row's range and grid: values run from `min` to `max` in `step`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slider {
    pub min: Duration,
    pub max: Duration,
    pub step: Duration,
    pub unit: SliderUnit,
}

/// The fade rows: 0 to 2 s in 50 ms steps.
const FADE_SLIDER: Slider = Slider {
    min: Duration::ZERO,
    max: Duration::from_millis(2000),
    step: Duration::from_millis(50),
    unit: SliderUnit::Ms,
};

/// Crossfade: 0 to 12 s in half-second steps.
const CROSSFADE_SLIDER: Slider = Slider {
    min: Duration::ZERO,
    max: Duration::from_secs(12),
    step: Duration::from_millis(500),
    unit: SliderUnit::Seconds,
};

/// The slider a row edits, or `None` for rows that are not sliders.
pub fn slider(row: &Row) -> Option<Slider> {
    match row.kind() {
        Kind::Slider(slider) => Some(slider),
        _ => None,
    }
}

/// The rows of one tab, in display order. The Library tab lists `folders`.
pub fn rows(tab: Tab, folders: &[PathBuf]) -> Vec<Row> {
    match tab {
        Tab::Display => vec![
            Row::ShowCover,
            Row::CoverSize,
            Row::CoverProtocol,
            Row::CoverSearch,
            Row::LibraryLayout,
            Row::Marquee,
            Row::Icons,
        ],
        Tab::Audio => vec![Row::DefaultDevice, Row::BitPerfect],
        Tab::Playback => vec![
            Row::FadePlay,
            Row::FadePause,
            Row::FadeStop,
            Row::FadeTrack,
            Row::FadeMute,
            Row::Crossfade,
        ],
        Tab::Library => {
            let mut rows: Vec<Row> = folders.iter().cloned().map(Row::Folder).collect();
            rows.extend([Row::AddFolder, Row::RescanAll]);
            rows
        }
        Tab::General => vec![Row::UpdateCheck],
    }
}

impl Row {
    /// The tab the row lives on.
    pub fn tab(&self) -> Tab {
        match self {
            Row::ShowCover
            | Row::CoverSize
            | Row::CoverProtocol
            | Row::CoverSearch
            | Row::LibraryLayout
            | Row::Marquee
            | Row::Icons => Tab::Display,
            Row::DefaultDevice | Row::BitPerfect => Tab::Audio,
            Row::FadePlay
            | Row::FadePause
            | Row::FadeStop
            | Row::FadeTrack
            | Row::FadeMute
            | Row::Crossfade => Tab::Playback,
            Row::Folder(_) | Row::AddFolder | Row::RescanAll => Tab::Library,
            Row::UpdateCheck => Tab::General,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Row::ShowCover
            | Row::CoverSearch
            | Row::Marquee
            | Row::BitPerfect
            | Row::UpdateCheck => Kind::Switch,
            Row::CoverSize | Row::CoverProtocol | Row::LibraryLayout | Row::Icons => Kind::Choice,
            Row::FadePlay | Row::FadePause | Row::FadeStop | Row::FadeTrack | Row::FadeMute => {
                Kind::Slider(FADE_SLIDER)
            }
            Row::Crossfade => Kind::Slider(CROSSFADE_SLIDER),
            Row::DefaultDevice | Row::AddFolder | Row::RescanAll => Kind::Action,
            Row::Folder(_) => Kind::Folder,
        }
    }

    /// The row's name. A folder shows its path with the home folder as `~`.
    pub fn label(&self) -> String {
        match self {
            Row::ShowCover => "Show cover".into(),
            Row::CoverSize => "Cover size".into(),
            Row::CoverProtocol => "Cover protocol".into(),
            Row::CoverSearch => "Cover search for streams".into(),
            Row::LibraryLayout => "Library layout".into(),
            Row::Marquee => "Scroll long titles".into(),
            Row::Icons => "Icons".into(),
            Row::DefaultDevice => "Default output device".into(),
            Row::BitPerfect => "Bit-perfect".into(),
            Row::FadePlay => "Fade in (play, resume)".into(),
            Row::FadePause => "Fade on pause".into(),
            Row::FadeStop => "Fade on stop".into(),
            Row::FadeTrack => "Fade on track change".into(),
            Row::FadeMute => "Mute fade".into(),
            Row::Crossfade => "Crossfade".into(),
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
            Row::FadePlay => {
                "Fade in over this time on play, resume, and after a track change or device switch. Off starts at full volume."
            }
            Row::FadePause => "Fade out over this time before pausing. Off pauses at once.",
            Row::FadeStop => "Fade out over this time before stopping. Off stops at once.",
            Row::FadeTrack => {
                "Fade out over this time before Next, Previous, a new track or a device switch. Off cuts at once."
            }
            Row::FadeMute => "How long mute and unmute take.",
            Row::Crossfade => {
                "Overlap the end of a track with the start of the next when one follows the other on its own (repeat-one too). Off joins them as they are."
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
            Row::FadePlay => Some(("audio.fade", "play_ms")),
            Row::FadePause => Some(("audio.fade", "pause_ms")),
            Row::FadeStop => Some(("audio.fade", "stop_ms")),
            Row::FadeTrack => Some(("audio.fade", "track_ms")),
            Row::FadeMute => Some(("audio.fade", "mute_ms")),
            Row::Crossfade => Some(("audio.fade", "crossfade_ms")),
            Row::Folder(_) | Row::AddFolder | Row::RescanAll => None,
        }
    }
}

/// One step along a slider's grid from `current`. A value off the grid moves
/// to the next grid value in that direction. The result stays within the range.
pub fn step_slider(slider: Slider, current: Duration, forward: bool) -> Duration {
    let step = slider.step.as_millis() as u64;
    let now = current.as_millis() as u64;
    let grid = if forward {
        (now / step + 1) * step
    } else {
        now.div_ceil(step).saturating_sub(1) * step
    };
    Duration::from_millis(grid).clamp(slider.min, slider.max)
}

/// Read a value typed into a slider's prompt. A number without a unit is in the
/// slider's unit; `ms` or `s` (any case, spaces allowed) names one. The error
/// is the slider's range, for the prompt to show.
pub fn parse_slider_value(slider: Slider, text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let (number, unit) = if let Some(rest) = lower.strip_suffix("ms") {
        (&text[..rest.len()], SliderUnit::Ms)
    } else if let Some(rest) = lower.strip_suffix('s') {
        (&text[..rest.len()], SliderUnit::Seconds)
    } else {
        (text, slider.unit)
    };
    let range = slider_range_text(slider);
    let value: f64 = number.trim().parse().map_err(|_| range.clone())?;
    let millis = match unit {
        SliderUnit::Ms => value,
        SliderUnit::Seconds => value * 1000.0,
    };
    let (low, high) = (slider.min.as_millis() as f64, slider.max.as_millis() as f64);
    if !(low..=high).contains(&millis) {
        return Err(range);
    }
    Ok(Duration::from_millis(millis.round() as u64))
}

/// A slider's range as the prompt and the error show it: `0–2000 ms`.
pub fn slider_range_text(slider: Slider) -> String {
    let min = unit_number(slider.min, slider.unit);
    let max = unit_number(slider.max, slider.unit);
    format!("{min}–{max}{}", unit_suffix(slider.unit))
}

/// A time as a number in `unit`: whole milliseconds, or seconds with at most
/// three decimals and no trailing zeros.
fn unit_number(time: Duration, unit: SliderUnit) -> String {
    let ms = time.as_millis();
    match unit {
        SliderUnit::Ms => ms.to_string(),
        SliderUnit::Seconds => {
            let (secs, frac) = (ms / 1000, ms % 1000);
            if frac == 0 {
                secs.to_string()
            } else {
                format!("{secs}.{frac:03}")
                    .trim_end_matches('0')
                    .to_string()
            }
        }
    }
}

fn unit_suffix(unit: SliderUnit) -> &'static str {
    match unit {
        SliderUnit::Ms => " ms",
        SliderUnit::Seconds => " s",
    }
}

/// The `[audio.fade]` time a fade row edits, or `None` for other rows.
pub fn fade_slot<'a>(row: &Row, fades: &'a mut FadeTimes) -> Option<&'a mut Duration> {
    match row {
        Row::FadePlay => Some(&mut fades.play),
        Row::FadePause => Some(&mut fades.pause),
        Row::FadeStop => Some(&mut fades.stop),
        Row::FadeTrack => Some(&mut fades.track),
        Row::FadeMute => Some(&mut fades.mute),
        Row::Crossfade => Some(&mut fades.crossfade),
        _ => None,
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
    /// `[audio.fade]` as the player has it.
    pub fades: FadeTimes,
}

/// A slider's value as its row shows it: `off` for zero, else in its unit.
fn slider_text(time: Duration, slider: Slider) -> String {
    if time.is_zero() {
        "off".into()
    } else {
        format!(
            "{}{}",
            unit_number(time, slider.unit),
            unit_suffix(slider.unit)
        )
    }
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
        Row::FadePlay => slider_text(v.fades.play, FADE_SLIDER),
        Row::FadePause => slider_text(v.fades.pause, FADE_SLIDER),
        Row::FadeStop => slider_text(v.fades.stop, FADE_SLIDER),
        Row::FadeTrack => slider_text(v.fades.track, FADE_SLIDER),
        Row::FadeMute => slider_text(v.fades.mute, FADE_SLIDER),
        Row::Crossfade => slider_text(v.fades.crossfade, CROSSFADE_SLIDER),
        Row::Folder(_) | Row::AddFolder | Row::RescanAll => String::new(),
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
    /// `[audio.fade]`.
    pub fades: FadeTimes,
}

impl Default for SettingsValues {
    fn default() -> Self {
        Self {
            bit_perfect: true,
            update_check: true,
            default_device: None,
            fades: FadeTimes::default(),
        }
    }
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
            fades: FadeTimes::default(),
        }
    }

    #[test]
    fn each_tab_has_its_rows() {
        let folders = [folder("jazz")];
        assert_eq!(
            rows(Tab::Audio, &folders),
            vec![Row::DefaultDevice, Row::BitPerfect]
        );
        assert_eq!(
            rows(Tab::Playback, &folders),
            vec![
                Row::FadePlay,
                Row::FadePause,
                Row::FadeStop,
                Row::FadeTrack,
                Row::FadeMute,
                Row::Crossfade
            ]
        );
        assert_eq!(
            rows(Tab::Library, &folders),
            vec![Row::Folder(folder("jazz")), Row::AddFolder, Row::RescanAll]
        );
        assert_eq!(rows(Tab::General, &folders), vec![Row::UpdateCheck]);
        assert_eq!(rows(Tab::Display, &folders)[0], Row::ShowCover);
    }

    #[test]
    fn tabs_have_labels_and_number_keys() {
        assert_eq!(
            Tab::ALL.map(Tab::label),
            ["Display", "Audio", "Playback", "Library", "General"]
        );
        assert_eq!(Tab::from_key('3'), Some(Tab::Playback));
        assert_eq!(Tab::from_key('6'), None);
        assert_eq!(Tab::General.next(), Tab::Display);
        assert_eq!(Tab::Display.previous(), Tab::General);
    }

    #[test]
    fn fade_rows_show_ms_or_off() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        v.fades.pause = Duration::ZERO;
        v.fades.play = Duration::from_millis(240);
        assert_eq!(value_text(&Row::FadePause, &v), "off");
        assert_eq!(value_text(&Row::FadePlay, &v), "240 ms");
        assert_eq!(value_text(&Row::FadeStop, &v), "300 ms");
    }

    #[test]
    fn playback_rows_are_sliders_with_crossfade_last() {
        assert_eq!(rows(Tab::Playback, &[]).last(), Some(&Row::Crossfade));
        assert!(matches!(Row::Crossfade.kind(), Kind::Slider(_)));
        assert!(matches!(Row::FadePlay.kind(), Kind::Slider(_)));
    }

    #[test]
    fn sliders_step_on_their_grid_and_stop_at_the_ends() {
        let fade = slider(&Row::FadePause).unwrap();
        let ms = Duration::from_millis;
        assert_eq!(step_slider(fade, ms(150), true), ms(200));
        assert_eq!(step_slider(fade, ms(240), true), ms(250));
        assert_eq!(step_slider(fade, ms(240), false), ms(200));
        assert_eq!(step_slider(fade, ms(2000), true), ms(2000));
        assert_eq!(step_slider(fade, Duration::ZERO, false), Duration::ZERO);
        let cross = slider(&Row::Crossfade).unwrap();
        assert_eq!(step_slider(cross, ms(3000), true), ms(3500));
        assert_eq!(step_slider(cross, ms(12_000), true), ms(12_000));
    }

    #[test]
    fn typed_values_parse_in_the_rows_unit() {
        let fade = slider(&Row::FadeStop).unwrap();
        let cross = slider(&Row::Crossfade).unwrap();
        let ms = Duration::from_millis;
        assert_eq!(parse_slider_value(fade, "150"), Ok(ms(150)));
        assert_eq!(parse_slider_value(fade, "0.2s"), Ok(ms(200)));
        assert_eq!(parse_slider_value(fade, " 300 ms "), Ok(ms(300)));
        assert_eq!(parse_slider_value(cross, "3.5"), Ok(ms(3500)));
        assert_eq!(parse_slider_value(cross, "3500ms"), Ok(ms(3500)));
        assert_eq!(parse_slider_value(cross, "13"), Err("0–12 s".to_string()));
        assert_eq!(parse_slider_value(fade, "-5"), Err("0–2000 ms".to_string()));
        assert_eq!(
            parse_slider_value(fade, "slow"),
            Err("0–2000 ms".to_string())
        );
    }

    #[test]
    fn slider_values_show_their_unit() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        v.fades.crossfade = Duration::from_millis(3500);
        assert_eq!(value_text(&Row::Crossfade, &v), "3.5 s");
        v.fades.crossfade = Duration::from_secs(3);
        assert_eq!(value_text(&Row::Crossfade, &v), "3 s");
        v.fades.crossfade = Duration::ZERO;
        assert_eq!(value_text(&Row::Crossfade, &v), "off");
        assert_eq!(value_text(&Row::FadeStop, &v), "300 ms");
    }

    #[test]
    fn crossfade_saves_under_audio_fade() {
        assert_eq!(
            Row::Crossfade.config_key(),
            Some(("audio.fade", "crossfade_ms"))
        );
    }

    #[test]
    fn fade_rows_save_under_audio_fade() {
        assert_eq!(Row::FadePlay.config_key(), Some(("audio.fade", "play_ms")));
        assert_eq!(Row::FadeMute.config_key(), Some(("audio.fade", "mute_ms")));
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
            fades: FadeTimes::default(),
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
    fn every_row_has_an_explanation() {
        for tab in Tab::ALL {
            for row in rows(tab, &[folder("a")]) {
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

        v.bit_perfect_changed = true;
        v.protocol_needs_restart = true;
        assert_eq!(hint_text(&Row::BitPerfect, &v), Some("on restart"));
        assert_eq!(hint_text(&Row::CoverProtocol, &v), Some("on restart"));
    }

    #[test]
    fn settings_values_default_to_the_config_defaults() {
        let values = SettingsValues::default();
        assert!(values.bit_perfect);
        assert!(values.update_check);
        assert_eq!(values.default_device, None);
    }
}
