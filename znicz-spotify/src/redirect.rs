//! Turn what the user pasted (the address the browser landed on, or just its
//! query) into the authorization code, checking it belongs to this login.

use crate::error::{Result, SpotifyError};

use percent_encoding::percent_decode_str;

/// The code in `input`, when its `state` is `expected_state`.
pub(crate) fn code_from(input: &str, expected_state: &str) -> Result<String> {
    let input = input.trim();
    let query = match input.split_once('?') {
        Some((_, query)) => query,
        None => input,
    };
    let query = query.split('#').next().unwrap_or("");

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = decode(value);
        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err(SpotifyError::WrongAddress);
    }
    if let Some(error) = error {
        return Err(SpotifyError::Declined(error));
    }
    match code {
        Some(code) if !code.is_empty() => Ok(code),
        _ => Err(SpotifyError::WrongAddress),
    }
}

fn decode(value: &str) -> String {
    let spaced = value.replace('+', " ");
    percent_decode_str(&spaced).decode_utf8_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_full_address() {
        let code = code_from(
            "http://127.0.0.1:8898/callback?code=AQBx-1_2&state=abc",
            "abc",
        );
        assert_eq!(code, Ok("AQBx-1_2".to_string()));
    }

    #[test]
    fn reads_a_bare_query_with_or_without_question_mark() {
        assert_eq!(code_from("code=c1&state=s", "s"), Ok("c1".to_string()));
        assert_eq!(code_from("?state=s&code=c2", "s"), Ok("c2".to_string()));
        assert_eq!(code_from("  code=c3&state=s\n", "s"), Ok("c3".to_string()));
    }

    #[test]
    fn decodes_percent_escapes_and_drops_a_fragment() {
        assert_eq!(
            code_from("http://x/callback?code=a%2Bb&state=s%3D#_", "s="),
            Ok("a+b".to_string())
        );
    }

    #[test]
    fn rejects_another_logins_state_or_a_missing_code() {
        assert_eq!(
            code_from("code=c&state=other", "s"),
            Err(SpotifyError::WrongAddress)
        );
        assert_eq!(code_from("state=s", "s"), Err(SpotifyError::WrongAddress));
        assert_eq!(
            code_from("http://127.0.0.1:8898/callback", "s"),
            Err(SpotifyError::WrongAddress)
        );
        assert_eq!(code_from("", "s"), Err(SpotifyError::WrongAddress));
    }

    #[test]
    fn reports_a_declined_login() {
        assert_eq!(
            code_from("error=access_denied&state=s", "s"),
            Err(SpotifyError::Declined("access_denied".into()))
        );
    }
}
