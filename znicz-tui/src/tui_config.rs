#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverProtocol {
    Auto,
    Kitty,
    Sixel,
    Halfblocks,
    Off,
}

impl CoverProtocol {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "kitty" => Self::Kitty,
            "sixel" => Self::Sixel,
            "halfblocks" => Self::Halfblocks,
            "off" => Self::Off,
            _ => Self::Auto,
        }
    }
}

/// Preferred library browse layout. Width still gates three-column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryLayout {
    Columns,
    Tree,
}

impl LibraryLayout {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "tree" => Self::Tree,
            _ => Self::Columns,
        }
    }
}

/// Which symbols the TUI draws for icons (the volume marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconSet {
    #[default]
    Unicode,
    Emoji,
    Nerd,
}

impl IconSet {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "emoji" => Self::Emoji,
            "nerd" => Self::Nerd,
            _ => Self::Unicode,
        }
    }
}

/// Stage cover size, as a share of the largest size that fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoverSize {
    #[default]
    P100,
    P75,
    P50,
    P25,
}

impl CoverSize {
    /// Every size, largest first.
    pub const ALL: [CoverSize; 4] = [Self::P100, Self::P75, Self::P50, Self::P25];

    /// Any value other than 75, 50 or 25 means full size.
    pub fn from_percent(value: i64) -> Self {
        match value {
            75 => Self::P75,
            50 => Self::P50,
            25 => Self::P25,
            _ => Self::P100,
        }
    }

    pub fn percent(self) -> u16 {
        match self {
            Self::P100 => 100,
            Self::P75 => 75,
            Self::P50 => 50,
            Self::P25 => 25,
        }
    }

    /// The next smaller size; stays at 25%.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1).min(Self::ALL.len() - 1)]
    }

    /// The next larger size; stays at 100%.
    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[i.saturating_sub(1)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiConfig {
    pub show_cover: bool,
    /// How big the Stage cover is drawn.
    pub cover_size: CoverSize,
    pub cover_protocol: CoverProtocol,
    pub library_layout: LibraryLayout,
    /// Scroll now-playing titles that do not fit (otherwise cut with `…`).
    pub marquee: bool,
    /// Look stream covers up by artist and title (Deezer, MusicBrainz).
    pub cover_search: bool,
    /// Symbol set for icons such as the volume marker.
    pub icons: IconSet,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            show_cover: true,
            cover_size: CoverSize::P100,
            cover_protocol: CoverProtocol::Auto,
            library_layout: LibraryLayout::Columns,
            marquee: true,
            cover_search: true,
            icons: IconSet::Unicode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_size_parses_the_four_steps_and_defaults_to_full() {
        assert_eq!(CoverSize::from_percent(100), CoverSize::P100);
        assert_eq!(CoverSize::from_percent(75), CoverSize::P75);
        assert_eq!(CoverSize::from_percent(50), CoverSize::P50);
        assert_eq!(CoverSize::from_percent(25), CoverSize::P25);
        for other in [0, 10, 60, 101, -5, i64::MAX] {
            assert_eq!(CoverSize::from_percent(other), CoverSize::P100, "{other}");
        }
        assert_eq!(TuiConfig::default().cover_size, CoverSize::P100);
        assert_eq!(CoverSize::P50.percent(), 50);
    }

    #[test]
    fn cover_size_steps_and_stops_at_the_ends() {
        assert_eq!(CoverSize::P100.prev(), CoverSize::P100);
        assert_eq!(CoverSize::P100.next(), CoverSize::P75);
        assert_eq!(CoverSize::P75.next(), CoverSize::P50);
        assert_eq!(CoverSize::P50.next(), CoverSize::P25);
        assert_eq!(CoverSize::P25.next(), CoverSize::P25);
        assert_eq!(CoverSize::P25.prev(), CoverSize::P50);
        assert_eq!(CoverSize::P75.prev(), CoverSize::P100);
    }

    #[test]
    fn marquee_is_on_by_default() {
        assert!(TuiConfig::default().marquee);
    }

    #[test]
    fn unknown_protocol_is_auto() {
        assert_eq!(CoverProtocol::parse("nope"), CoverProtocol::Auto);
        assert_eq!(CoverProtocol::parse("KITTY"), CoverProtocol::Kitty);
        assert_eq!(CoverProtocol::parse("off"), CoverProtocol::Off);
    }

    #[test]
    fn icon_set_parses_known_names_and_falls_back_to_unicode() {
        assert_eq!(IconSet::parse("emoji"), IconSet::Emoji);
        assert_eq!(IconSet::parse("NERD"), IconSet::Nerd);
        assert_eq!(IconSet::parse(" Emoji "), IconSet::Emoji);
        assert_eq!(IconSet::parse("unicode"), IconSet::Unicode);
        assert_eq!(IconSet::parse("nope"), IconSet::Unicode);
        assert_eq!(IconSet::parse(""), IconSet::Unicode);
        assert_eq!(TuiConfig::default().icons, IconSet::Unicode);
    }

    #[test]
    fn library_layout_defaults_to_columns() {
        assert_eq!(LibraryLayout::parse("columns"), LibraryLayout::Columns);
        assert_eq!(LibraryLayout::parse(""), LibraryLayout::Columns);
        assert_eq!(LibraryLayout::parse("nope"), LibraryLayout::Columns);
        assert_eq!(LibraryLayout::parse("TREE"), LibraryLayout::Tree);
        assert_eq!(LibraryLayout::parse("tree"), LibraryLayout::Tree);
    }
}
