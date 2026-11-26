//! The OSC 52 clipboard sequence. The terminal sets its clipboard from it; tmux
//! takes it when `set-clipboard` is on.

use std::io::Write;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// The bytes that ask the terminal to set its clipboard to `text`.
pub fn sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text))
}

/// Write the sequence to the terminal. It is called from a key handler, between
/// two frames: ratatui flushes at the end of every draw, so the bytes never land
/// inside a frame, and an escape sequence changes no cell, so the next frame's
/// diff is unaffected. Tests do not call this; `App::clipboard` is replaced.
pub fn copy(text: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    out.write_all(sequence(text).as_bytes())?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_is_osc_52_with_base64_and_bel() {
        assert_eq!(sequence("hi"), "\x1b]52;c;aGk=\x07");
        assert_eq!(sequence(""), "\x1b]52;c;\x07");
    }

    #[test]
    fn the_encoding_is_standard_base64_with_padding() {
        assert_eq!(sequence("a"), "\x1b]52;c;YQ==\x07");
        assert_eq!(sequence("ab"), "\x1b]52;c;YWI=\x07");
        assert_eq!(sequence("abc"), "\x1b]52;c;YWJj\x07");
        // The standard alphabet has `+` and `/`, not `-` and `_`.
        assert_eq!(sequence(">>>"), "\x1b]52;c;Pj4+\x07");
        assert_eq!(sequence("???"), "\x1b]52;c;Pz8/\x07");
    }

    #[test]
    fn the_url_survives_the_round_trip() {
        let url = "https://accounts.spotify.com/authorize?client_id=0123&state=abc&x=y%2Fz";
        let body = sequence(url);
        let encoded = body
            .strip_prefix("\x1b]52;c;")
            .and_then(|rest| rest.strip_suffix('\x07'))
            .expect("framed");
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .expect("base64");
        assert_eq!(String::from_utf8(decoded).unwrap(), url);
    }
}
