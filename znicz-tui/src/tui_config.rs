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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiConfig {
    pub show_cover: bool,
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
