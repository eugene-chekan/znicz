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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiConfig {
    pub show_cover: bool,
    pub cover_protocol: CoverProtocol,
    pub library_layout: LibraryLayout,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            show_cover: true,
            cover_protocol: CoverProtocol::Auto,
            library_layout: LibraryLayout::Columns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_protocol_is_auto() {
        assert_eq!(CoverProtocol::parse("nope"), CoverProtocol::Auto);
        assert_eq!(CoverProtocol::parse("KITTY"), CoverProtocol::Kitty);
        assert_eq!(CoverProtocol::parse("off"), CoverProtocol::Off);
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
