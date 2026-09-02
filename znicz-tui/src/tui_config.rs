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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiConfig {
    pub show_cover: bool,
    pub cover_protocol: CoverProtocol,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            show_cover: true,
            cover_protocol: CoverProtocol::Auto,
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
}
